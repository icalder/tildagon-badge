use core::sync::atomic::Ordering;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::Subscriber;
use embassy_time::Duration;
use tildagon::battery::Battery;
use tildagon::buttons::{Button, ButtonEvent};

type ButtonSubscriber = Subscriber<'static, CriticalSectionRawMutex, ButtonEvent, 16, 4, 1>;

#[embassy_executor::task]
pub async fn button_monitor(
    mut sub: ButtonSubscriber,
    mut battery: Battery<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>,
) {
    esp_println::println!("[BUTTON_MONITOR] Ready, awaiting button events...");
    let power_hold_time = Duration::from_secs(2);

    loop {
        let event = sub.next_message_pure().await;
        esp_println::println!("[BUTTON_MONITOR] Event: {:?}", event);

        if event != ButtonEvent::Pressed(Button::F) {
            continue;
        }

        esp_println::println!("[BUTTON_MONITOR] Hold F for 2s to power off");

        match embassy_time::with_timeout(power_hold_time, async {
            loop {
                let event = sub.next_message_pure().await;
                esp_println::println!("[BUTTON_MONITOR] Event: {:?}", event);

                if event == ButtonEvent::Released(Button::F) {
                    break;
                }
            }
        })
        .await
        {
            Ok(()) => {
                esp_println::println!("[BUTTON_MONITOR] Power-off cancelled");
            }
            Err(_) => {
                esp_println::println!("[BUTTON_MONITOR] Long press detected, powering off");
                match battery.power_off().await {
                    Ok(()) => {
                        crate::SHUTTING_DOWN.store(true, Ordering::Relaxed);
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
}
