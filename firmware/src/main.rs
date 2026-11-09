#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::i2c::{self, I2c};
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver, InterruptHandler};
use embassy_time::Timer;
use embassy_usb::UsbDevice;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::driver::EndpointError;
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
    
    // pinout and information for controlling the led
    // let data = p.PIN_4;
    // let clock = p.PIN_5;
    let mut led = Output::new(p.PIN_25, Level::Low);

    // let i2c = I2c::new_blocking(p.I2C0, clock, data, i2c::Config::default());
    // let interface = I2CDisplayInterface::new(i2c);
    // let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0).into_buffered_graphics_mode();
    // display.init().unwrap();

    // let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    // Text::with_baseline("Hello, Pico!", Point::new(0, 0), style, Baseline::Top).draw(&mut display).unwrap();
    // display.flush().unwrap();

    loop {
        class.wait_connection().await;
        let _ = echo(&mut class, &mut led).await;
    }
}

type MyUsbDriver = Driver<'static, USB>;
type MyUsbDevice = UsbDevice<'static, MyUsbDriver>;

#[embassy_executor::task]
async fn usb_task(mut usb: MyUsbDevice) -> ! {
    usb.run().await
}

struct Disconnected {}

impl From<EndpointError> for Disconnected {
    fn from(val: EndpointError) -> Self {
        match val {
            EndpointError::BufferOverflow => panic!("Buffer Overflow"),
            EndpointError::Disabled => Disconnected {},
        }
    }
}

async fn echo(class: &mut CdcAcmClass<'static, Driver<'static, USB>>, led: &mut Output<'static>) -> Result<(), Disconnected> {
    let mut buf = [0; 64];
    loop {
        led.toggle();
        let n = class.read_packet(&mut buf).await?;
        let data = &buf[..n];
        class.write_packet(data).await?;
    }
}
