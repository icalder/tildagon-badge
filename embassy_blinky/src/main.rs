//! Tildagon Badge LED Control with USB Support
//!
//! This firmware initializes the Tildagon badge LEDs while maintaining
//! USB serial communication. Button presses trigger LED animation sequences.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::{PubSubChannel, Subscriber};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use smart_leds::colors::*;
use static_cell::StaticCell;
use tildagon::hardware::TildagonHardware;
use tildagon::leds::{Leds, NUM_LEDS};
use tildagon::buttons::{Buttons, Button};

esp_bootloader_esp_idf::esp_app_desc!();

#[embassy_executor::task]
async fn run() {
    loop {
        esp_println::println!("Hello world from embassy!");
        Timer::after(Duration::from_millis(3_000)).await;
    }
}

#[embassy_executor::task]
async fn button_monitor(mut sub: Subscriber<'static, CriticalSectionRawMutex, Button, 1, 2, 1>) {
    esp_println::println!("[BUTTON_MONITOR] Ready, awaiting button press...");

    loop {
        let button = sub.next_message_pure().await;
        esp_println::println!("[BUTTON_MONITOR] Button {:?} pressed!", button);
    }
}

#[embassy_executor::task]
async fn blinky(
    mut leds: Leds,
    mut sub: Subscriber<'static, CriticalSectionRawMutex, Button, 1, 2, 1>,
) {
    esp_println::println!("[BLINKY] Waiting for button C (CONFIRM) press to start animation...");

    loop {
        let button = sub.next_message_pure().await;

        if button == Button::C {
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
    
    let mut i2c = tildagon.i2c;
    let mut button_int = tildagon.button_int;

    esp_println::println!("Boot: Tildagon hardware init done, USB should be stable");

    spawner.spawn(run()).ok();

    static BUTTON_CHANNEL: StaticCell<PubSubChannel<CriticalSectionRawMutex, Button, 1, 2, 1>> =
        StaticCell::new();
    let channel = BUTTON_CHANNEL.init(PubSubChannel::new());

    spawner
        .spawn(button_monitor(channel.subscriber().unwrap()))
        .ok();

    let leds = Leds::new(tildagon.rmt, tildagon.led_pin);

    spawner
        .spawn(blinky(leds, channel.subscriber().unwrap()))
        .ok();

    let publisher = channel.publisher().unwrap();
    let mut buttons = Buttons::new();

    esp_println::println!("[BUTTON] Entering main loop, waiting for falling edge...");
    loop {
        match buttons.wait_for_press(&mut i2c, &mut button_int).await {
            Ok(Some(button)) => publisher.publish_immediate(button),
            Ok(None) => {}
            Err(e) => esp_println::println!("[BUTTON] Error reading buttons: {:?}", e),
        }
    }
}
