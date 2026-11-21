#![no_std]
#![no_main]

use core::fmt::Write;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::i2c::{self, I2c};
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver, InterruptHandler};
use embassy_time::{Duration, TimeoutError, with_timeout};
use embassy_usb::UsbDevice;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::{Baseline, Text};
use heapless::{HistoryBuf, String, Vec};
use panic_halt as _;
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};
use static_cell::StaticCell;

const SCREEN_WIDTH: i32 = 128;
const SCREEN_HEIGHT: i32 = 64;
const CHAR_WIDTH: i32 = 6;
const LABEL_CHARS: i32 = 11;
const TEXT_WIDTH: i32 = CHAR_WIDTH * LABEL_CHARS;
const GRAPH_X: i32 = TEXT_WIDTH + 2;
const GRAPH_WIDTH: i32 = SCREEN_WIDTH - GRAPH_X;

bind_interrupts!(struct Irqs {
   USBCTRL_IRQ => InterruptHandler<USB>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let driver = Driver::new(p.USB, Irqs);

    let config = {
        let mut config = embassy_usb::Config::new(0xc0de, 0xcafe);
        config.manufacturer = Some("Ginko");
        config.product = Some("USB-serial Test");
        config.serial_number = Some("00000000");
        config.max_power = 100;
        config.max_packet_size_0 = 64;
        config
    };

    let mut builder = {
        static CONFIG_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
        static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
        static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();

        let builder = embassy_usb::Builder::new(
            driver,
            config,
            CONFIG_DESCRIPTOR.init([0; 256]),
            BOS_DESCRIPTOR.init([0; 256]),
            &mut [],
            CONTROL_BUF.init([0; 64]),
        );
        builder
    };

    let mut class = {
        static STATE: StaticCell<State> = StaticCell::new();
        let state = STATE.init(State::new());
        CdcAcmClass::new(&mut builder, state, 64)
    };

    let usb = builder.build();

    spawner.spawn(usb_task(usb).unwrap());

    let sda = p.PIN_4;
    let scl = p.PIN_5;

    let i2c = I2c::new_blocking(p.I2C0, scl, sda, i2c::Config::default());
    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    display.init().unwrap();

    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    draw_message_at_point(style, &mut display, Point::new(0, 0), "Waiting for Host");

    let mut buf = [0; 64];
    let mut line_buf: Vec<u8, 128> = Vec::new();

    loop {
        class.wait_connection().await;
        match with_timeout(Duration::from_secs(3), class.read_packet(&mut buf)).await {
            Ok(Ok(n)) => {
                for &b in &buf[..n] {
                    if !b.is_ascii() {
                        continue;
                    }
                    if b == b'\r' || b == b'\n' {
                        clear_and_draw_lines(&line_buf, style, &mut display);
                        line_buf.clear();
                    } else if let Err(item) = line_buf.push(b) {
                        clear_and_draw_lines(&line_buf, style, &mut display);
                        line_buf.clear();
                        let _ = line_buf.push(item);
                    }
                }
            }
            Ok(Err(_)) => {
                line_buf.clear();
            }
            Err(TimeoutError) => {
                line_buf.clear();
                if class.dtr() {
                    clear_screen(&mut display);
                    draw_message_at_point(style, &mut display, Point::new(0, 0), "Host Not Responding");
                    flush_screen(&mut display);
                } else {
                    clear_screen(&mut display);
                    draw_message_at_point(style, &mut display, Point::new(0, 0), "Waiting for Host");
                    flush_screen(&mut display);
                }
            }
        }
    }
}

type MyDisplay = Ssd1306<
    I2CInterface<I2c<'static, embassy_rp::peripherals::I2C0, embassy_rp::i2c::Blocking>>,
    DisplaySize128x64,
    ssd1306::mode::BufferedGraphicsMode<DisplaySize128x64>,
>;
type MyUsbDriver = Driver<'static, USB>;
type MyUsbDevice = UsbDevice<'static, MyUsbDriver>;

#[embassy_executor::task]
async fn usb_task(mut usb: MyUsbDevice) -> ! {
    usb.run().await
}

fn draw_message_at_point(
    style: MonoTextStyle<'static, BinaryColor>,
    display: &mut MyDisplay,
    pos: Point,
    message: &str,
) {
    Text::with_baseline(message, pos, style, Baseline::Top)
        .draw(display)
        .unwrap();
}

fn clear_screen(display: &mut MyDisplay) {
    display.clear(BinaryColor::Off).unwrap();
}

fn flush_screen(display: &mut MyDisplay) {
    display.flush().unwrap();
}

fn clear_and_draw_lines(
    line_buf: &[u8],
    style: MonoTextStyle<'static, BinaryColor>,
    display: &mut MyDisplay,
) {
    if line_buf.is_empty() {
        return;
    }
    clear_screen(display);
    let data = core::str::from_utf8(&line_buf).unwrap();
    // split data on ,
    let split_data = data.split(',');
    for pair in split_data {
        if let Some((key, val)) = pair.split_once(':') {
            let mut line = String::<64>::new();
            let parsed_val = parse_val(val);
            let (label, line_y_pos) = match key {
                s if s.eq_ignore_ascii_case("cpu") => ("CPU: ", 0),
                s if s.eq_ignore_ascii_case("cpu_temp") => (">TEMP: ", 10),
                s if s.eq_ignore_ascii_case("gpu_usage") => ("GPU: ", 20),
                s if s.eq_ignore_ascii_case("gpu_temp") => (">TEMP: ", 30),
                s if s.eq_ignore_ascii_case("mem") => ("MEM: ", 40),
                s if s.eq_ignore_ascii_case("disk") => ("DSK: ", 50),
                _ => continue,
            };
            tmp_draw_parsed_val(parsed_val, line_y_pos, style, display);
            let _ = write!(line, "{}{}", label, val);
            draw_message_at_point(style, display, Point::new(0, line_y_pos), &line);
        }
    }
    flush_screen(display);
}

fn parse_val(val: &str) -> Option<f32> {
    let trimmed_val = val.trim_end_matches("%").trim_end_matches("GB");

    let parsed_val = trimmed_val.parse::<f32>().ok()?;
    if parsed_val < 0. {
        return None;
    }
    Some(parsed_val)
}

struct SensorHistory {
    cpu_history: HistoryBuf<f32, 60>,
    cpu_temp_history: HistoryBuf<f32, 60>,
    gpu_history: HistoryBuf<f32, 60>,
    gpu_temp_history: HistoryBuf<f32, 60>,
    mem_history: HistoryBuf<f32, 60>,
    disk_history: HistoryBuf<f32, 60>,
}

fn init_history() -> SensorHistory {
    SensorHistory {
        cpu_history: HistoryBuf::new(),
        cpu_temp_history: HistoryBuf::new(),
        gpu_history: HistoryBuf::new(),
        gpu_temp_history: HistoryBuf::new(),
        mem_history: HistoryBuf::new(),
        disk_history: HistoryBuf::new()
    }
}

fn write_history() {
    
}

fn tmp_draw_parsed_val(
    parsed_val: Option<f32>,
    line_y_pos: i32,
    style: MonoTextStyle<'static, BinaryColor>,
    display: &mut MyDisplay,
) {
    match parsed_val {
        Some(pv) => {
            let mut tmp_str = String::<32>::new();
            let _ = write!(tmp_str, "{:1.}", pv);
            Text::with_baseline(
                &tmp_str,
                Point::new(TEXT_WIDTH + 2 * CHAR_WIDTH, line_y_pos),
                style,
                Baseline::Top,
            )
            .draw(display)
            .unwrap()
        }
        None => Text::with_baseline(
            "No Value",
            Point::new(TEXT_WIDTH + 2 * CHAR_WIDTH, line_y_pos),
            style,
            Baseline::Top,
        )
        .draw(display)
        .unwrap(),
    };
}
