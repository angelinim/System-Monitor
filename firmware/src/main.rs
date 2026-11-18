#![no_std]
#![no_main]

use core::fmt::Write;
use embassy_time::{Duration, TimeoutError, with_timeout};
use heapless::{ Vec, String };
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::i2c::{self, I2c};
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver, InterruptHandler};
use embassy_usb::UsbDevice;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embedded_graphics::prelude::*;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::text::{Baseline, Text};
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};
use panic_halt as _;
use static_cell::StaticCell;

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
           CONFIG_DESCRIPTOR.init([0;256]), 
           BOS_DESCRIPTOR.init([0;256]), 
           &mut [],
           CONTROL_BUF.init([0;64]),
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
    let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0).into_buffered_graphics_mode();
    display.init().unwrap();

    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    draw_message(style, &mut display,"Waiting for Host");

    let mut buf = [0; 64];
    let mut line_buf: Vec<u8, 128> = Vec::new();

    loop {
        class.wait_connection().await;
        match with_timeout(Duration::from_secs(3), class.read_packet(&mut buf)).await {
            Ok(Ok(n)) => {
                for &b in &buf[..n] {
                    if !b.is_ascii() {continue}
                    if b == b'\r' || b == b'\n' {
                        clear_and_draw_lines(&line_buf, style, &mut display);
                        line_buf.clear();
                    } else if let Err(item) = line_buf.push(b){
                        clear_and_draw_lines(&line_buf, style, &mut display);
                        line_buf.clear();
                        let _ = line_buf.push(item);
                    }
                }
            },
            Ok(Err(_)) => {
                line_buf.clear();
            },
            Err(TimeoutError) => {
                line_buf.clear();
                if class.dtr() {
                    draw_message(style, &mut display,"Host Not Responding");
                } else {
                    draw_message(style, &mut display,"Waiting for Host");
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

fn draw_message(
    style: MonoTextStyle<'static, BinaryColor>,
    display: &mut MyDisplay,
    message: &str,
) {
    display.clear(BinaryColor::Off).unwrap();
    Text::with_baseline(message, Point::new(0, 0), style, Baseline::Top).draw(display).unwrap();
    display.flush().unwrap();
}


fn clear_and_draw_lines(
    line_buf: &[u8],
    style: MonoTextStyle<'static, BinaryColor>,
    display: &mut MyDisplay,
){
    if line_buf.is_empty() {return}
    display.clear(BinaryColor::Off).unwrap();
    let data = core::str::from_utf8(&line_buf).unwrap();
    // split data on ,
    let split_data = data.split(',');
    for pair in split_data {
        if let Some((key, val)) = pair.split_once(':'){
            let mut line = String::<64>::new();
            match key {
                s if s.eq_ignore_ascii_case("mem") => {
                    let _ = write!(line, "MEM: {}", val);
                    Text::with_baseline(line.as_str(), Point::new(0, 0), style, Baseline::Top).draw(display).unwrap();
                }
                s if s.eq_ignore_ascii_case("cpu") => {
                    let _ = write!(line, "CPU: {}", val);
                    Text::with_baseline(line.as_str(), Point::new(64, 0), style, Baseline::Top).draw(display).unwrap();
                }
                s if s.eq_ignore_ascii_case("temp") => {
                    let _ = write!(line, "TEMP: {}", val);
                    Text::with_baseline(line.as_str(), Point::new(0, 50), style, Baseline::Top).draw(display).unwrap();
                }
                s if s.eq_ignore_ascii_case("disk") => {
                    let _ = write!(line, "DSK: {}", val);
                    Text::with_baseline(line.as_str(), Point::new(64, 50), style, Baseline::Top).draw(display).unwrap();
                }
                _ => continue,
            }
        }
    }
    display.flush().unwrap();
}
