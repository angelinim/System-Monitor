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
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle};
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
const GRAPH_START: i32 = TEXT_WIDTH + 2;
const GRAPH_END: i32 = SCREEN_WIDTH - 1;
const GRAPH_WIDTH: i32 = SCREEN_WIDTH - GRAPH_START;
const GRAPH_BOTTOM: i32 = SCREEN_HEIGHT - 1;
const GRAPH_TOP: i32 = 1;
const HISTORY_LEN: usize = GRAPH_WIDTH as usize;

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
    let mut diagnostic_hist = init_history();

    loop {
        class.wait_connection().await;
        match with_timeout(Duration::from_secs(3), class.read_packet(&mut buf)).await {
            Ok(Ok(n)) => {
                for &b in &buf[..n] {
                    if !b.is_ascii() {
                        continue;
                    }
                    if b == b'\r' || b == b'\n' {
                        let Some(diagnostic_info) = parse_buffer(&line_buf) else {
                            line_buf.clear();
                            continue;
                        };
                        clear_screen(&mut display);

                        for info in diagnostic_info {
                            let mut label = String::<128>::new();
                            let _ = write!(label, "{}{}", info.label, info.val_str);
                            draw_message_at_point(style, &mut display, info.screen_pos, &label);
                            set_history(&info, &mut diagnostic_hist);
                        }
                        draw_graph(&diagnostic_hist, &DiagnosticType::CpuUsage, &mut display);

                        flush_screen(&mut display);
                        line_buf.clear();
                    } else if let Err(item) = line_buf.push(b) {
                        let Some(diagnostic_info) = parse_buffer(&line_buf) else {
                            line_buf.clear();
                            let _ = line_buf.push(item);
                            continue;
                        };
                        clear_screen(&mut display);

                        for info in diagnostic_info {
                            let mut label = String::<128>::new();
                            let _ = write!(label, "{}{}", info.label, info.val_str);
                            draw_message_at_point(style, &mut display, info.screen_pos, &label);
                            set_history(&info, &mut diagnostic_hist);
                        }
                        draw_graph(&diagnostic_hist, &DiagnosticType::CpuUsage, &mut display);

                        flush_screen(&mut display);
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
                let message = if class.dtr() {
                    "Host Not Responding"
                } else {
                    "Waiting for Host"
                };
                clear_screen(&mut display);
                draw_message_at_point(style, &mut display, Point::new(0, 0), message);
                flush_screen(&mut display);
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

struct DiagnosticLabelInfo<'a> {
    label: &'a str,
    val_str: &'a str,
    screen_pos: Point,
    diag_type: DiagnosticType,
}

enum DiagnosticType {
    CpuUsage,
    CpuTemp,
    GpuUsage,
    GpuTemp,
    MemoryUsage,
    DiskUsage,
}

fn parse_buffer<'a>(line_buf: &'a [u8]) -> Option<Vec<DiagnosticLabelInfo<'a>, 6>> {
    if line_buf.is_empty() {
        return None;
    }
    let mut return_vec: Vec<DiagnosticLabelInfo, 6> = Vec::new();
    let data = core::str::from_utf8(&line_buf).unwrap();

    let split_data = data.split(',');
    for pair in split_data {
        if let Some((key, val_str)) = pair.split_once(':') {
            let (label, screen_y_pos, diag_type) = match key {
                s if s.eq_ignore_ascii_case("cpu") => ("CPU: ", 0, DiagnosticType::CpuUsage),
                s if s.eq_ignore_ascii_case("cpu_temp") => (">TEMP: ", 10, DiagnosticType::CpuTemp),
                s if s.eq_ignore_ascii_case("gpu_usage") => ("GPU: ", 20, DiagnosticType::GpuUsage),
                s if s.eq_ignore_ascii_case("gpu_temp") => (">TEMP: ", 30, DiagnosticType::GpuTemp),
                s if s.eq_ignore_ascii_case("mem") => ("MEM: ", 40, DiagnosticType::MemoryUsage),
                s if s.eq_ignore_ascii_case("disk") => ("DSK: ", 50, DiagnosticType::DiskUsage),
                _ => continue,
            };
            let _ = return_vec.push(DiagnosticLabelInfo {
                label,
                val_str,
                screen_pos: Point::new(0, screen_y_pos),
                diag_type,
            });
        }
    }
    Some(return_vec)
}

fn parse_val(val: &str) -> Option<f32> {
    let trimmed_val = val.trim_end_matches("%").trim_end_matches("GB");

    let parsed_val = trimmed_val.parse::<f32>().ok()?;
    if !parsed_val.is_finite() || parsed_val < 0. {
        return None;
    }
    Some(parsed_val)
}

struct DiagnosticHistory {
    cpu_history: HistoryBuf<f32, HISTORY_LEN>,
    cpu_temp_history: HistoryBuf<f32, HISTORY_LEN>,
    gpu_history: HistoryBuf<f32, HISTORY_LEN>,
    gpu_temp_history: HistoryBuf<f32, HISTORY_LEN>,
    mem_history: HistoryBuf<f32, HISTORY_LEN>,
    disk_history: HistoryBuf<f32, HISTORY_LEN>,
}

impl DiagnosticHistory {
    fn get(&self, diagnostic_type: &DiagnosticType) -> &HistoryBuf<f32, HISTORY_LEN> {
        match diagnostic_type {
            DiagnosticType::CpuUsage => &self.cpu_history,
            DiagnosticType::CpuTemp => &self.cpu_temp_history,
            DiagnosticType::GpuUsage => &self.gpu_history,
            DiagnosticType::GpuTemp => &self.gpu_temp_history,
            DiagnosticType::MemoryUsage => &self.mem_history,
            DiagnosticType::DiskUsage => &self.disk_history,
        }
    }
    fn get_mut(&mut self, diagnostic_type: &DiagnosticType) -> &mut HistoryBuf<f32, HISTORY_LEN> {
        match diagnostic_type {
            DiagnosticType::CpuUsage => &mut self.cpu_history,
            DiagnosticType::CpuTemp => &mut self.cpu_temp_history,
            DiagnosticType::GpuUsage => &mut self.gpu_history,
            DiagnosticType::GpuTemp => &mut self.gpu_temp_history,
            DiagnosticType::MemoryUsage => &mut self.mem_history,
            DiagnosticType::DiskUsage => &mut self.disk_history,
        }
    }

    fn write_history_point(&mut self, diagnostic_type: &DiagnosticType, val: f32) {
        self.get_mut(diagnostic_type).write(val);
    }
}

fn init_history() -> DiagnosticHistory {
    DiagnosticHistory {
        cpu_history: HistoryBuf::new(),
        cpu_temp_history: HistoryBuf::new(),
        gpu_history: HistoryBuf::new(),
        gpu_temp_history: HistoryBuf::new(),
        mem_history: HistoryBuf::new(),
        disk_history: HistoryBuf::new(),
    }
}

// parse string and add f32 value to history
fn set_history(diag_info: &DiagnosticLabelInfo, history: &mut DiagnosticHistory) {
    if let Some(parsed_val) = parse_val(diag_info.val_str) {
        history.write_history_point(&diag_info.diag_type, parsed_val);
    }
}

// draw each point of the history on the graph
fn draw_graph(history: &DiagnosticHistory, diag_type: &DiagnosticType, display: &mut MyDisplay) {
    Rectangle::new(
        Point::new(GRAPH_START, GRAPH_TOP),
        Size::new(GRAPH_WIDTH as u32, GRAPH_BOTTOM as u32)
    )
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(display).unwrap();

    let hist = history.get(diag_type);
    let actual_hist_len = hist.len();
    for (i, hist_val) in hist.oldest_ordered().enumerate() {
        // could use a match on diagnostic type to decide on min and max values.
        let y = convert_val_to_y(0., 100., *hist_val);
        let x = GRAPH_START + (HISTORY_LEN - actual_hist_len) as i32 + i as i32;
        Line::new(Point::new(x, y), Point::new(x, GRAPH_BOTTOM))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(display)
            .unwrap();
    }
}

fn convert_val_to_y(min: f32, max: f32, val: f32) -> i32 {
    let fraction = ((val - min) / (max - min)).clamp(0., 1.);
    // bottom of screen -> SCREEN_HEIGHT - 1 this is because y starts at top of screen
    (GRAPH_BOTTOM as f32 - (GRAPH_BOTTOM as f32 * fraction)) as i32
}
