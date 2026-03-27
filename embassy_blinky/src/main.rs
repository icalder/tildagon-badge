//! Tildagon Badge LED Control with USB Support
//!
//! This firmware initializes the Tildagon badge LEDs while maintaining
//! USB serial communication. Button presses trigger LED animation sequences.

#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::Write as _;

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::pubsub::{PubSubChannel, Subscriber};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use heapless::String;
use smart_leds::colors::*;
use static_cell::StaticCell;
use tildagon::battery::{Battery, BatteryDiagnostics, BatteryState};
use tildagon::buttons::{Button, ButtonEvent, TypedButtons};
use tildagon::display::TildagonDisplay;
use tildagon::hardware::TildagonHardware;
use tildagon::i2c::{SharedI2cBus, system_i2c_bus, top_i2c_bus};
use tildagon::leds::{TypedLeds, NUM_LEDS};
use tildagon::pins::Pins;

use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::text::{Alignment, Text};
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
async fn button_monitor(
    mut sub: Subscriber<'static, CriticalSectionRawMutex, ButtonEvent, 1, 3, 1>,
) {
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
    mut battery: Battery<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>,
) {
    esp_println::println!("[DISPLAY] Task started");

    let level_style = MonoTextStyle::new(&PROFONT_24_POINT, Rgb565::WHITE);
    let detail_style = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    if let Err(e) = render_startup(&mut display, level_style, detail_style) {
        esp_println::println!("[DISPLAY] Startup render error: {:?}", e);
        return;
    }

    let mut battery_state = battery.read().await.ok();
    let mut overlay: Option<(&'static str, Point, u8)> = None;
    let mut battery_refresh_ticks = 0u8;

    loop {
        let mut redraw = false;

        if battery_refresh_ticks == 0 {
            battery_refresh_ticks = 10;
            match battery.read().await {
                Ok(state) => {
                    battery_state = Some(state);
                    redraw = true;
                }
                Err(e) => {
                    esp_println::println!("[DISPLAY] Battery read error: {:?}", e);
                    battery_state = None;
                    redraw = true;
                }
            }
        } else {
            battery_refresh_ticks -= 1;
        }

        match embassy_time::with_timeout(Duration::from_millis(200), sub.next_message_pure()).await
        {
            Ok(event) => {
                overlay = Some(button_overlay(event));
                redraw = true;
            }
            Err(_) => {
                if let Some((text, pos, ticks_left)) = overlay {
                    if ticks_left > 1 {
                        overlay = Some((text, pos, ticks_left - 1));
                    } else {
                        overlay = None;
                        redraw = true;
                    }
                }
            }
        }

        if redraw {
            let overlay_text = overlay.map(|(text, pos, _)| (text, pos));
            let render_result = match battery_state {
                Some(state) => {
                    render_battery_info(&mut display, level_style, detail_style, state, overlay_text)
                }
                None => render_battery_error(&mut display, detail_style, overlay_text),
            };

            if let Err(e) = render_result {
                esp_println::println!("[DISPLAY] Render error: {:?}", e);
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
    level_style: MonoTextStyle<'static, Rgb565>,
    detail_style: MonoTextStyle<'static, Rgb565>,
) -> Result<(), DisplayDrawError> {
    clear_display(display)?;
    Text::with_alignment("Battery", Point::new(120, 96), detail_style, Alignment::Center)
        .draw(display)?;
    Text::with_alignment("--%", Point::new(120, 132), level_style, Alignment::Center)
        .draw(display)?;
    Ok(())
}

fn render_battery_info(
    display: &mut TildagonDisplay<'static>,
    level_style: MonoTextStyle<'static, Rgb565>,
    detail_style: MonoTextStyle<'static, Rgb565>,
    state: BatteryState,
    overlay: Option<(&str, Point)>,
) -> Result<(), DisplayDrawError> {
    let mut level_text: String<8> = String::new();
    let mut voltage_text: String<16> = String::new();
    let mut status_text: String<24> = String::new();

    let _ = write!(level_text, "{}%", state.estimated_level_percent());
    let _ = write!(voltage_text, "{:.2}V", state.vbat_volts);
    let _ = write!(status_text, "{}", state.charge_status.as_str());

    clear_display(display)?;
    Text::with_alignment("Battery", Point::new(120, 84), detail_style, Alignment::Center)
        .draw(display)?;
    Text::with_alignment(
        level_text.as_str(),
        Point::new(120, 128),
        level_style,
        Alignment::Center,
    )
    .draw(display)?;
    Text::with_alignment(
        voltage_text.as_str(),
        Point::new(120, 160),
        detail_style,
        Alignment::Center,
    )
    .draw(display)?;
    Text::with_alignment(
        status_text.as_str(),
        Point::new(120, 184),
        detail_style,
        Alignment::Center,
    )
    .draw(display)?;

    if let Some((text, pos)) = overlay {
        Text::new(text, pos, detail_style).draw(display)?;
    }

    Ok(())
}

fn render_battery_error(
    display: &mut TildagonDisplay<'static>,
    detail_style: MonoTextStyle<'static, Rgb565>,
    overlay: Option<(&str, Point)>,
) -> Result<(), DisplayDrawError> {
    clear_display(display)?;
    Text::with_alignment("Battery", Point::new(120, 108), detail_style, Alignment::Center)
        .draw(display)?;
    Text::with_alignment("read error", Point::new(120, 136), detail_style, Alignment::Center)
        .draw(display)?;

    if let Some((text, pos)) = overlay {
        Text::new(text, pos, detail_style).draw(display)?;
    }

    Ok(())
}

fn log_battery_diagnostics(diag: &BatteryDiagnostics) {
    esp_println::println!(
        "[PMIC] status=0x{:02X} fault=0x{:02X} charge={} vbat={:.2}V vsys={:.2}V vbus={:.2}V ichg={:.2}A",
        diag.state.raw_status,
        diag.state.raw_fault,
        diag.state.charge_status.as_str(),
        diag.state.vbat_volts,
        diag.state.vsys_volts,
        diag.state.vbus_volts,
        diag.state.charge_current_amps,
    );
    esp_println::println!(
        "[PMIC] reg00=0x{:02X} hiz={} reg03=0x{:02X} boost={} reg07=0x{:02X} reg09=0x{:02X} batfet_disabled={}",
        diag.reg00_input_source,
        diag.input_hiz_enabled(),
        diag.reg03_power_on_config,
        diag.boost_enabled(),
        diag.reg07_charge_timer,
        diag.reg09_misc_operation,
        diag.batfet_disabled(),
    );
}

fn button_overlay(event: ButtonEvent) -> (&'static str, Point, u8) {
    let (text, button) = match event {
        ButtonEvent::Pressed(button) => {
            let text = match button {
                Button::A => "A P",
                Button::B => "B P",
                Button::C => "C P",
                Button::D => "D P",
                Button::E => "E P",
                Button::F => "F P",
            };
            (text, button)
        }
        ButtonEvent::Released(button) => {
            let text = match button {
                Button::A => "A R",
                Button::B => "B R",
                Button::C => "C R",
                Button::D => "D R",
                Button::E => "E R",
                Button::F => "F R",
            };
            (text, button)
        }
    };

    (text, get_button_pos(button), 5)
}

fn get_button_pos(btn: Button) -> Point {
    match btn {
        Button::A => Point::new(100, 40),
        Button::B => Point::new(170, 80),
        Button::C => Point::new(170, 170),
        Button::D => Point::new(100, 210),
        Button::E => Point::new(30, 170),
        Button::F => Point::new(30, 80),
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let mut tildagon = TildagonHardware::new(esp_hal::init(esp_hal::Config::default()))
        .await
        .expect("Tildagon hardware init failed");

    static DISPLAY_BUFFER: StaticCell<[u8; 1024]> = StaticCell::new();
    let display_buffer = DISPLAY_BUFFER.init([0u8; 1024]);
    let display = tildagon.init_display(display_buffer);

    static SHARED_I2C: StaticCell<
        SharedI2cBus<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>,
    > = StaticCell::new();
    let shared_i2c = SHARED_I2C.init(Mutex::new(tildagon.i2c.into_async()));
    let mut button_int = tildagon.button_int;
    let pins = Pins::new();

    esp_println::println!("Boot: Tildagon hardware init done, typed shared I2C ready");

    {
        let mut battery_diag = Battery::new(system_i2c_bus(shared_i2c));
        match battery_diag.diagnostics().await {
            Ok(diag) => log_battery_diagnostics(&diag),
            Err(e) => esp_println::println!("[PMIC] Diagnostic read failed: {:?}", e),
        }
    }

    spawner.spawn(run()).expect("Failed to spawn run_task");

    static BUTTON_CHANNEL: StaticCell<
        PubSubChannel<CriticalSectionRawMutex, ButtonEvent, 1, 3, 1>,
    > = StaticCell::new();
    let channel = BUTTON_CHANNEL.init(PubSubChannel::new());

    spawner
        .spawn(button_monitor(channel.subscriber().unwrap()))
        .expect("Failed to spawn button_monitor");

    match display {
        Ok(display) => {
            let battery = Battery::new(system_i2c_bus(shared_i2c));
            spawner
                .spawn(display_task(display, channel.subscriber().unwrap(), battery))
                .expect("Failed to spawn display_task");
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
        .expect("Failed to spawn blinky");

    let publisher = channel.publisher().unwrap();
    let mut buttons = TypedButtons::new(system_i2c_bus(shared_i2c), top_i2c_bus(shared_i2c));

    esp_println::println!("[BUTTON] Entering main loop, waiting for button events...");
    loop {
        match buttons.wait_for_event(&mut button_int).await {
            Ok(Some(event)) => publisher.publish_immediate(event),
            Ok(None) => {}
            Err(e) => esp_println::println!("[BUTTON] Error reading buttons: {:?}", e),
        }
    }
}
