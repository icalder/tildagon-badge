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
use tildagon::display::{self, TildagonDisplay};
use tildagon::i2c::{SharedI2cBus, system_i2c_bus, top_i2c_bus};
use tildagon::pins::Pins;

use embedded_graphics::prelude::*;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::text::Text;
use embedded_graphics::mono_font::MonoTextStyle;
use profont::PROFONT_24_POINT;

esp_bootloader_esp_idf::esp_app_desc!();

#[embassy_executor::task]
async fn run() {
    loop {
        esp_println::println!("Hello world from embassy!");
        Timer::after(Duration::from_millis(3_000)).await;
    }
}

#[embassy_executor::task]
async fn button_monitor(mut sub: Subscriber<'static, CriticalSectionRawMutex, ButtonEvent, 1, 3, 1>) {
    esp_println::println!("[BUTTON_MONITOR] Ready, awaiting button events...");

    loop {
        let event = sub.next_message_pure().await;
        esp_println::println!("[BUTTON_MONITOR] Event: {:?}", event);
    }
}

#[embassy_executor::task]
async fn blinky(
    mut leds: TypedLeds<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>,
    mut sub: Subscriber<'static, CriticalSectionRawMutex, ButtonEvent, 1, 3, 1>,
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

#[embassy_executor::task]
async fn display_task(
    mut display: TildagonDisplay<'static>,
    mut sub: Subscriber<'static, CriticalSectionRawMutex, ButtonEvent, 1, 3, 1>,
) {
    esp_println::println!("[DISPLAY] Task started");

    let style = MonoTextStyle::new(&PROFONT_24_POINT, Rgb565::WHITE);
    if let Err(e) = render_startup(&mut display, style) {
        esp_println::println!("[DISPLAY] Startup render error: {:?}", e);
        return;
    }

    let mut next_event = None;
    loop {
        let event = if let Some(e) = next_event.take() {
            e
        } else {
            sub.next_message_pure().await
        };

        let is_released = matches!(event, ButtonEvent::Released(_));
        let (text, pos) = match event {
            ButtonEvent::Pressed(b) => {
                let t = match b {
                    Button::A => "A P",
                    Button::B => "B P",
                    Button::C => "C P",
                    Button::D => "D P",
                    Button::E => "E P",
                    Button::F => "F P",
                };
                (t, get_button_pos(b))
            }
            ButtonEvent::Released(b) => {
                 let t = match b {
                    Button::A => "A R",
                    Button::B => "B R",
                    Button::C => "C R",
                    Button::D => "D R",
                    Button::E => "E R",
                    Button::F => "F R",
                };
                (t, get_button_pos(b))
            }
        };

        if let Err(e) = render_button_event(&mut display, style, text, pos) {
            esp_println::println!("[DISPLAY] Event render error: {:?}", e);
            continue;
        }

        if is_released {
            match embassy_time::with_timeout(Duration::from_secs(1), sub.next_message_pure()).await {
                Ok(e) => {
                    next_event = Some(e);
                }
                Err(_) => {
                    if let Err(e) = clear_display(&mut display) {
                        esp_println::println!("[DISPLAY] Clear error: {:?}", e);
                    }
                }
            }
        }
    }
}

type DisplayDrawError = <TildagonDisplay<'static> as DrawTarget>::Error;

fn clear_display(display: &mut TildagonDisplay<'static>) -> Result<(), DisplayDrawError> {
    display.clear(Rgb565::BLACK)
}

fn render_startup(
    display: &mut TildagonDisplay<'static>,
    style: MonoTextStyle<'static, Rgb565>,
) -> Result<(), DisplayDrawError> {
    clear_display(display)?;
    Text::new("Tildagon", Point::new(60, 120), style).draw(display)?;
    Ok(())
}

fn render_button_event(
    display: &mut TildagonDisplay<'static>,
    style: MonoTextStyle<'static, Rgb565>,
    text: &str,
    pos: Point,
) -> Result<(), DisplayDrawError> {
    clear_display(display)?;
    Text::new(text, pos, style).draw(display)?;
    Ok(())
}

fn get_button_pos(btn: Button) -> Point {
    match btn {
        Button::A => Point::new(100, 40),   // Top
        Button::B => Point::new(170, 80),   // Top-Right
        Button::C => Point::new(170, 170),  // Bottom-Right
        Button::D => Point::new(100, 210),  // Bottom
        Button::E => Point::new(30, 170),   // Bottom-Left
        Button::F => Point::new(30, 80),    // Top-Left
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
        PubSubChannel<CriticalSectionRawMutex, ButtonEvent, 1, 3, 1>,
    > = StaticCell::new();
    let channel = BUTTON_CHANNEL.init(PubSubChannel::new());

    spawner
        .spawn(button_monitor(channel.subscriber().unwrap()))
        .ok();

    static DISPLAY_BUFFER: StaticCell<[u8; 1024]> = StaticCell::new();
    let display_buffer = DISPLAY_BUFFER.init([0u8; 1024]);
    match display::init(tildagon.top_board, tildagon.display, display_buffer) {
        Ok(display) => {
            spawner
                .spawn(display_task(display, channel.subscriber().unwrap()))
                .ok();
        }
        Err(e) => {
            esp_println::println!("[DISPLAY] Init error: {:?}", e);
        }
    }

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
