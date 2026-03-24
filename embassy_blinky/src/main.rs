//! Tildagon Badge LED Control with USB Support
//!
//! This firmware initializes the Tildagon badge LEDs while maintaining
//! USB serial communication. Button presses trigger LED animation sequences.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::pubsub::{PubSubChannel, Subscriber};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use smart_leds::colors::*;
use static_cell::StaticCell;
use tildagon::hardware::TildagonHardware;
use tildagon::leds::{TypedLeds, NUM_LEDS};
use tildagon::buttons::{Button, ButtonEvent, TypedButtons};
use tildagon::i2c::{SharedI2cBus, system_i2c_bus, top_i2c_bus};
use tildagon::pins::Pins;

esp_bootloader_esp_idf::esp_app_desc!();

#[embassy_executor::task]
async fn run() {
    loop {
        esp_println::println!("Hello world from embassy!");
        Timer::after(Duration::from_millis(3_000)).await;
    }
}

#[embassy_executor::task]
async fn button_monitor(mut sub: Subscriber<'static, CriticalSectionRawMutex, ButtonEvent, 1, 2, 1>) {
    esp_println::println!("[BUTTON_MONITOR] Ready, awaiting button events...");

    loop {
        let event = sub.next_message_pure().await;
        esp_println::println!("[BUTTON_MONITOR] Event: {:?}", event);
    }
}

#[embassy_executor::task]
async fn blinky(
    mut leds: TypedLeds<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>,
    mut sub: Subscriber<'static, CriticalSectionRawMutex, ButtonEvent, 1, 2, 1>,
) {
    esp_println::println!("[BLINKY] Waiting for button C press to start animation...");

    loop {
        let event = sub.next_message_pure().await;

        if event == ButtonEvent::Pressed(Button::C) {
            esp_println::println!("[BLINKY] Starting LED animation...");
            let colors = [RED, GREEN, BLUE, YELLOW, MAGENTA, CYAN, WHITE];

            for color in colors {
                let dim: smart_leds::RGB8 = smart_leds::RGB8 {
                    r: color.r / 2,
                    g: color.g / 2,
                    b: color.b / 2,
                };
                let data = [dim; NUM_LEDS];
                if let Err(e) = leds.write(data.iter().cloned()).await {
                    esp_println::println!("LED write error: {:?}", e);
                }
                Timer::after(Duration::from_millis(500)).await;
            }

            if let Err(e) = leds.clear().await {
                esp_println::println!("LED clear error: {:?}", e);
            }
            Timer::after(Duration::from_millis(500)).await;

            esp_println::println!("[BLINKY] Animation complete, LEDs off");
        }
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let tildagon = TildagonHardware::new(esp_hal::init(esp_hal::Config::default()))
        .await
        .expect("Tildagon hardware init failed");

    static SHARED_I2C: StaticCell<
        SharedI2cBus<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>,
    > = StaticCell::new();
    let shared_i2c = SHARED_I2C.init(Mutex::new(tildagon.i2c.into_async()));
    let mut button_int = tildagon.button_int;
    let pins = Pins::new();

    esp_println::println!("Boot: Tildagon hardware init done, typed shared I2C ready");

    spawner.spawn(run()).ok();

    static BUTTON_CHANNEL: StaticCell<
        PubSubChannel<CriticalSectionRawMutex, ButtonEvent, 1, 2, 1>,
    > = StaticCell::new();
    let channel = BUTTON_CHANNEL.init(PubSubChannel::new());

    spawner
        .spawn(button_monitor(channel.subscriber().unwrap()))
        .ok();

    let leds = TypedLeds::new(
        tildagon.rmt,
        tildagon.led_pin,
        pins.led,
        system_i2c_bus(shared_i2c),
    )
    .await
    .expect("Typed LED init failed");

    spawner
        .spawn(blinky(leds, channel.subscriber().unwrap()))
        .ok();

    let publisher = channel.publisher().unwrap();
    let mut buttons = TypedButtons::new(
        system_i2c_bus(shared_i2c),
        top_i2c_bus(shared_i2c),
    );

    esp_println::println!("[BUTTON] Entering main loop, waiting for button events...");
    loop {
        match buttons.wait_for_event(&mut button_int).await {
            Ok(Some(event)) => publisher.publish_immediate(event),
            Ok(None) => {}
            Err(e) => esp_println::println!("[BUTTON] Error reading buttons: {:?}", e),
        }
    }
}
