use core::sync::atomic::Ordering;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::Subscriber;
use embassy_time::{Duration, Timer};
use tildagon::battery::Battery;
use tildagon::buttons::{Button, ButtonEvent as PhysicalButtonEvent};

use crate::events::{BUTTON_EVENTS, ButtonEvent as AppButtonEvent, SYSTEM_EVENTS, SystemEvent};

type ButtonSubscriber = Subscriber<'static, CriticalSectionRawMutex, PhysicalButtonEvent, 16, 4, 1>;

#[embassy_executor::task]
pub async fn button_monitor(
    mut sub: ButtonSubscriber,
    mut battery: Battery<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>,
) {
    esp_println::println!("[BUTTON_MONITOR] Ready, awaiting button events...");
    let power_hold_time = Duration::from_secs(2);
    let shutdown_grace_period = Duration::from_millis(500);

    loop {
        let event = sub.next_message_pure().await;

        match event {
            PhysicalButtonEvent::Pressed(Button::A) => {
                let _ = BUTTON_EVENTS.try_send(AppButtonEvent::Up);
            }
            PhysicalButtonEvent::Pressed(Button::B) => {
                let _ = BUTTON_EVENTS.try_send(AppButtonEvent::ToggleRadar);
            }
            PhysicalButtonEvent::Pressed(Button::D) => {
                let _ = BUTTON_EVENTS.try_send(AppButtonEvent::Down);
            }
            PhysicalButtonEvent::Pressed(Button::C) => {
                let _ = BUTTON_EVENTS.try_send(AppButtonEvent::Select);
            }
            PhysicalButtonEvent::Pressed(Button::F) => {
                // Handle short press (Back) and long press (PowerOff)
                match embassy_time::with_timeout(power_hold_time, async {
                    loop {
                        let event = sub.next_message_pure().await;
                        if event == PhysicalButtonEvent::Released(Button::F) {
                            return;
                        }
                    }
                })
                .await
                {
                    Ok(()) => {
                        // Short press
                        let _ = BUTTON_EVENTS.try_send(AppButtonEvent::Back);
                    }
                    Err(_) => {
                        // Long press
                        esp_println::println!("[BUTTON_MONITOR] Long press detected, powering off");
                        crate::SHUTTING_DOWN.store(true, Ordering::Relaxed);
                        let _ = SYSTEM_EVENTS.try_send(SystemEvent::PowerOff);
                        Timer::after(shutdown_grace_period).await;

                        match battery.power_off().await {
                            Ok(()) => {
                                esp_println::println!(
                                    "[BUTTON_MONITOR] BATFET disabled; waiting for power loss"
                                );
                                break;
                            }
                            Err(e) => {
                                esp_println::println!(
                                    "[BUTTON_MONITOR] Failed to request power-off: {:?}",
                                    e
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
