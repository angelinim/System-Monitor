#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::i2c::{self, I2c};
use embassy_time::Timer;
use embedded_graphics::prelude::*;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::text::{Baseline, Text};
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};
use panic_halt as _;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let data = p.PIN_4;
    let clock = p.PIN_5;
    let mut led = Output::new(p.PIN_25, Level::Low);

    let i2c = I2c::new_blocking(p.I2C0, clock, data, i2c::Config::default());
    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0).into_buffered_graphics_mode();
    display.init().unwrap();

    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    Text::with_baseline("Hello, Pico!", Point::new(0, 0), style, Baseline::Top).draw(&mut display).unwrap();
    display.flush().unwrap();

    loop {
        led.set_high();
        Timer::after_millis(500).await;
        led.set_low();
        Timer::after_millis(500).await;
    }
}
