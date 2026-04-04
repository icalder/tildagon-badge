use embassy_time::{Duration, Timer};
use esp_println::println;
use smart_leds::RGB8;
use tildagon::leds::TypedLeds;

use crate::itag::{APP_STATE, Mode, RssiHeatLevel, rssi_heat_level};

type BadgeLeds = TypedLeds<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>;

const RADAR_LED_FIRST: usize = 1;
const RADAR_LED_LAST: usize = 12;
const RADAR_PULSE_IDLE: Duration = Duration::from_millis(200);
const RADAR_PULSE_STEP: Duration = Duration::from_millis(90);
const RADAR_PULSE_STEPS: u8 = 24;
const RADAR_MIN_BRIGHTNESS: u8 = 12;
const RADAR_MAX_BRIGHTNESS: u8 = 96;

fn heat_color(level: RssiHeatLevel) -> RGB8 {
    match level {
        RssiHeatLevel::Hot => RGB8 { r: 255, g: 0, b: 0 },
        RssiHeatLevel::Warm => RGB8 {
            r: 255,
            g: 96,
            b: 0,
        },
        RssiHeatLevel::Near => RGB8 {
            r: 255,
            g: 200,
            b: 0,
        },
        RssiHeatLevel::Cool => RGB8 { r: 0, g: 255, b: 0 },
        RssiHeatLevel::Cold => RGB8 {
            r: 0,
            g: 200,
            b: 255,
        },
        RssiHeatLevel::Far => RGB8 { r: 0, g: 0, b: 255 },
    }
}

fn scale_channel(value: u8, brightness: u8) -> u8 {
    ((value as u16 * brightness as u16) / 255) as u8
}

fn scale_color(color: RGB8, brightness: u8) -> RGB8 {
    RGB8 {
        r: scale_channel(color.r, brightness),
        g: scale_channel(color.g, brightness),
        b: scale_channel(color.b, brightness),
    }
}

fn pulse_brightness(step: u8) -> u8 {
    let half_cycle = RADAR_PULSE_STEPS / 2;
    let ramp = if step < half_cycle {
        step
    } else {
        RADAR_PULSE_STEPS - step
    };
    let span = (RADAR_MAX_BRIGHTNESS - RADAR_MIN_BRIGHTNESS) as u16;
    RADAR_MIN_BRIGHTNESS + ((span * ramp as u16) / half_cycle as u16) as u8
}

fn radar_frame(color: RGB8, brightness: u8) -> [RGB8; tildagon::leds::NUM_LEDS] {
    let mut frame = [RGB8::default(); tildagon::leds::NUM_LEDS];
    let pulsed = scale_color(color, brightness);
    for led in RADAR_LED_FIRST..=RADAR_LED_LAST {
        frame[led] = pulsed;
    }
    frame
}

#[embassy_executor::task]
pub async fn radar_led_task(mut leds: BadgeLeds) {
    let mut pulse_step = 0u8;
    let mut cleared = true;

    loop {
        let snapshot = {
            let state = APP_STATE.lock().await;
            let selected_rssi = state
                .devices
                .get(state.selected_index)
                .map(|device| device.rssi);
            (
                state.radar_mode_enabled && state.mode == Mode::Scanning,
                selected_rssi,
            )
        };

        match snapshot {
            (true, Some(rssi)) => {
                let frame = radar_frame(
                    heat_color(rssi_heat_level(rssi)),
                    pulse_brightness(pulse_step),
                );
                if let Err(e) = leds.write(frame.iter().cloned()).await {
                    println!("[LED] radar write error: {:?}", e);
                } else {
                    cleared = false;
                }
                pulse_step = (pulse_step + 1) % RADAR_PULSE_STEPS;
                Timer::after(RADAR_PULSE_STEP).await;
            }
            _ => {
                pulse_step = 0;
                if !cleared {
                    if let Err(e) = leds.clear().await {
                        println!("[LED] radar clear error: {:?}", e);
                    } else {
                        cleared = true;
                    }
                }
                Timer::after(RADAR_PULSE_IDLE).await;
            }
        }
    }
}
