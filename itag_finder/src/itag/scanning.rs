use crate::events::{BLE_EVENTS, BUTTON_EVENTS, BleEvent, ButtonEvent, DISPLAY_SIGNAL};
use crate::itag::{APP_STATE, DEVICE_STALE_AFTER, Mode, SCAN_IDLE_RESTART_AFTER, SCAN_SESSION_RESTART_AFTER};
use core::sync::atomic::Ordering;
use embassy_futures::select::{Either3, select3};
use embassy_time::{Duration, Instant, Timer};
use esp_println::println;
use trouble_host::central::Central;
use trouble_host::prelude::*;

use super::ITagEntry;

pub async fn run_scanning(
    mut central: Central<'static, crate::BleExternalController, DefaultPacketPool>,
    ble_scan_config: &ScanConfig<'static>,
) -> Central<'static, crate::BleExternalController, DefaultPacketPool> {
    println!("BLE: Starting scan");
    let mut scanner = Scanner::new(central);
    let scan_session = scanner
        .scan(ble_scan_config)
        .await
        .expect("BLE scan failed");
    let scan_started_at = Instant::now();
    let mut last_detection_at = None;

    loop {
        match select3(
            BLE_EVENTS.receive(),
            BUTTON_EVENTS.receive(),
            Timer::after(Duration::from_millis(500)),
        )
        .await
        {
            Either3::First(ble_event) => match ble_event {
                BleEvent::DeviceSeen(addr, rssi, name) => {
                    last_detection_at = Some(Instant::now());
                    let mut state = APP_STATE.lock().await;
                    let mut found = false;
                    for device in state.devices.iter_mut() {
                        if device.addr == addr {
                            let previous_rssi = device.rssi;
                            let previous_name = device.name.clone();
                            device.rssi = rssi;
                            if name.is_some() {
                                device.name = name.clone();
                            }
                            device.last_seen = Instant::now();
                            if previous_rssi != rssi || previous_name != device.name {
                                println!(
                                    "App: Refreshed device {:?} rssi {} -> {} name={:?}",
                                    addr.addr,
                                    previous_rssi,
                                    rssi,
                                    device.name.as_ref().map(|n| n.as_str())
                                );
                            }
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        println!(
                            "App: New device seen {:?} rssi={} name={:?}",
                            addr.addr,
                            rssi,
                            name.as_ref().map(|n| n.as_str())
                        );
                        let _ = state.devices.push(ITagEntry {
                            addr,
                            rssi,
                            name,
                            last_seen: Instant::now(),
                        });
                    }
                    DISPLAY_SIGNAL.signal(());
                }
            },
            Either3::Second(button_event) => {
                let mut state = APP_STATE.lock().await;
                match button_event {
                    ButtonEvent::Up => {
                        if !state.devices.is_empty() {
                            if state.selected_index == 0 {
                                state.selected_index = state.devices.len() - 1;
                            } else {
                                state.selected_index -= 1;
                            }
                        }
                    }
                    ButtonEvent::Down => {
                        if !state.devices.is_empty() {
                            state.selected_index = (state.selected_index + 1) % state.devices.len();
                        }
                    }
                    ButtonEvent::ToggleRadar => {
                        state.radar_mode_enabled = !state.radar_mode_enabled;
                    }
                    ButtonEvent::Select => {
                        if !state.devices.is_empty() {
                            let device = state.devices[state.selected_index].clone();
                            state.mode = Mode::Connecting;
                            state.target_addr = Some(device.addr);
                            DISPLAY_SIGNAL.signal(());
                            break; // Exit inner loop to re-enter outer loop and handle connecting
                        }
                    }
                    _ => {}
                }
                DISPLAY_SIGNAL.signal(());
            }
            Either3::Third(_) => {
                let mut state = APP_STATE.lock().await;
                let now = Instant::now();
                let mut removed_any = false;
                state.devices.retain(|d| {
                    let age = now.duration_since(d.last_seen);
                    let keep = age < DEVICE_STALE_AFTER;
                    if !keep {
                        println!(
                            "App: Removing stale device {:?} age_ms={} rssi={} name={:?}",
                            d.addr.addr,
                            age.as_millis(),
                            d.rssi,
                            d.name.as_ref().map(|n| n.as_str())
                        );
                        removed_any = true;
                    }
                    keep
                });
                if removed_any {
                    if state.devices.is_empty() {
                        state.selected_index = 0;
                    } else if state.selected_index >= state.devices.len() {
                        state.selected_index = state.devices.len() - 1;
                    }
                    DISPLAY_SIGNAL.signal(());
                }
                if let Some(last_detection_at) = last_detection_at {
                    if now.duration_since(last_detection_at) >= SCAN_IDLE_RESTART_AFTER {
                        println!(
                            "BLE: Restarting quiet scan session after {} ms",
                            SCAN_IDLE_RESTART_AFTER.as_millis()
                        );
                        break;
                    }
                }
                if now.duration_since(scan_started_at) >= SCAN_SESSION_RESTART_AFTER {
                    println!(
                        "BLE: Restarting scan session after {} ms",
                        SCAN_SESSION_RESTART_AFTER.as_millis()
                    );
                    break;
                }
            }
        }
        if crate::SHUTTING_DOWN.load(Ordering::Relaxed) {
            println!("BLE: Scan loop exiting due to shutdown");
            break;
        }
    }
    println!("BLE: Scan loop ended, dropping scan session");
    drop(scan_session);
    central = scanner.into_inner();
    println!("BLE: Scan task ready to restart scanner");
    central
}

