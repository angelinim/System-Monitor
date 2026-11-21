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
                        if let Some(diagnostic_info) = parse_buffer(&line_buf) {
                            clear_screen(&mut display);

                            for info in diagnostic_info {
                                let mut label = String::<128>::new();
                                let _ = write!(label, "{}{}", info.label, info.val_str);
                                draw_message_at_point(style, &mut display, info.screen_pos, &label);
                                set_history(&info, &mut diagnostic_hist);
                                tmp_draw_hist_length(&diagnostic_hist, &info, &mut display, style);
                            }
                            flush_screen(&mut display);
                        }
                        line_buf.clear();
                    } else if let Err(item) = line_buf.push(b) {
                        if let Some(diagnostic_info) = parse_buffer(&line_buf) {
                            clear_screen(&mut display);

                            for info in diagnostic_info {
                                let mut label = String::<128>::new();
                                let _ = write!(label, "{}{}", info.label, info.val_str);
                                draw_message_at_point(style, &mut display, info.screen_pos, &label);
                                set_history(&info, &mut diagnostic_hist);
                                tmp_draw_hist_length(&diagnostic_hist, &info, &mut display, style);
                            }
                            flush_screen(&mut display);
                        }
                        parse_buffer(&line_buf);
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
                    draw_message_at_point(
                        style,
                        &mut display,
                        Point::new(0, 0),
                        "Host Not Responding",
                    );
                    flush_screen(&mut display);
                } else {
                    clear_screen(&mut display);
                    draw_message_at_point(
                        style,
                        &mut display,
                        Point::new(0, 0),
                        "Waiting for Host",
                    );
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
                s if s.eq_ignore_ascii_case("cpu") => {
                    ("CPU: ", 0,DiagnosticType::CpuUsage)
                },
                s if s.eq_ignore_ascii_case("cpu_temp") => {
                    (">TEMP: ", 10,DiagnosticType::CpuTemp)
                }
                s if s.eq_ignore_ascii_case("gpu_usage") => {
                    ("GPU: ", 20,DiagnosticType::GpuUsage)
                }
                s if s.eq_ignore_ascii_case("gpu_temp") => {
                    (">TEMP: ", 30,DiagnosticType::GpuTemp)
                }
                s if s.eq_ignore_ascii_case("mem") => {
                    ("MEM: ", 40,DiagnosticType::MemoryUsage)
                }
                s if s.eq_ignore_ascii_case("disk") => {
                    ("DSK: ", 50,DiagnosticType::DiskUsage)
                }
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
    if parsed_val < 0. {
        return None;
    }
    Some(parsed_val)
}

struct DiagnosticHistory {
    cpu_history: HistoryBuf<f32, 60>,
    cpu_temp_history: HistoryBuf<f32, 60>,
    gpu_history: HistoryBuf<f32, 60>,
    gpu_temp_history: HistoryBuf<f32, 60>,
    mem_history: HistoryBuf<f32, 60>,
    disk_history: HistoryBuf<f32, 60>,
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
    if let Some(parsed_val)= parse_val(diag_info.val_str) {
        match diag_info.diag_type {
            DiagnosticType::CpuUsage => history.cpu_history.write(parsed_val),
            DiagnosticType::CpuTemp => history.cpu_temp_history.write(parsed_val),
            DiagnosticType::GpuUsage => history.gpu_history.write(parsed_val),
            DiagnosticType::GpuTemp => history.gpu_temp_history.write(parsed_val),
            DiagnosticType::MemoryUsage => history.mem_history.write(parsed_val),
            DiagnosticType::DiskUsage => history.disk_history.write(parsed_val),
        }
    }
}

fn draw_graph() {}

fn tmp_draw_hist_length(
    history: &DiagnosticHistory,
     info: &DiagnosticLabelInfo,
     display: &mut MyDisplay,
     style: MonoTextStyle<'static, BinaryColor> 
) {
    // history.cpu_history.len();
    let mut message = String::<32>::new();
    match info.diag_type {
        DiagnosticType::CpuUsage => {
            let _ = write!(message, "H_len:{}", history.cpu_history.len());
            draw_message_at_point(style, display, Point::new(TEXT_WIDTH + 2 * CHAR_WIDTH, info.screen_pos.y), &message);
        },
        DiagnosticType::CpuTemp => {
            let _ = write!(message, "H_len:{}", history.cpu_temp_history.len());
            draw_message_at_point(style, display, Point::new(TEXT_WIDTH + 2 * CHAR_WIDTH, info.screen_pos.y), &message);
        },
        DiagnosticType::GpuUsage => {
            let _ = write!(message, "H_len:{}", history.gpu_history.len());
            draw_message_at_point(style, display, Point::new(TEXT_WIDTH + 2 * CHAR_WIDTH, info.screen_pos.y), &message);
        },
        DiagnosticType::GpuTemp => {
            let _ = write!(message, "H_len:{}", history.gpu_temp_history.len());
            draw_message_at_point(style, display, Point::new(TEXT_WIDTH + 2 * CHAR_WIDTH, info.screen_pos.y), &message);
        },
        DiagnosticType::MemoryUsage => {
            let _ = write!(message, "H_len:{}", history.mem_history.len());
            draw_message_at_point(style, display, Point::new(TEXT_WIDTH + 2 * CHAR_WIDTH, info.screen_pos.y), &message);
        },
        DiagnosticType::DiskUsage => {
            let _ = write!(message, "H_len:{}", history.disk_history.len());
            draw_message_at_point(style, display, Point::new(TEXT_WIDTH + 2 * CHAR_WIDTH, info.screen_pos.y), &message);
        },
    }
}
