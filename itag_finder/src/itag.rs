use core::sync::atomic::Ordering;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Instant, Timer};
use esp_println::println;
use heapless::Vec;
use trouble_host::prelude::*;
use crate::BleExternalController;
use crate::events::{BLE_EVENTS, BleEvent, BUTTON_EVENTS, ButtonEvent, DISPLAY_SIGNAL};

#[derive(Debug, Clone)]
pub struct ITagEntry {
    pub addr: Address,
    pub rssi: i8,
    pub name: Option<heapless::String<32>>,
    pub last_seen: Instant,
}

pub struct AppState {
    pub devices: Vec<ITagEntry, 16>,
    pub selected_index: usize,
}

pub static APP_STATE: Mutex<CriticalSectionRawMutex, AppState> = Mutex::new(AppState {
    devices: Vec::new(),
    selected_index: 0,
});

#[embassy_executor::task]
pub async fn itag_task(
    _stack: &'static Stack<'static, BleExternalController, DefaultPacketPool>,
) {
    println!("App Controller task started");

    loop {
        if crate::SHUTTING_DOWN.load(Ordering::Relaxed) {
            break;
        }

        use embassy_futures::select::{Either3, select3};

        match select3(
            BLE_EVENTS.receive(),
            BUTTON_EVENTS.receive(),
            Timer::after(Duration::from_millis(500)), // Periodic aging and cleanup
        ).await {
            Either3::First(ble_event) => {
                match ble_event {
                    BleEvent::DeviceSeen(addr, rssi, name) => {
                        let mut state = APP_STATE.lock().await;
                        let mut found = false;
                        for device in state.devices.iter_mut() {
                            if device.addr == addr {
                                device.rssi = rssi;
                                if name.is_some() {
                                    device.name = name.clone();
                                }
                                device.last_seen = Instant::now();
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            esp_println::println!("App: New device seen: {:?}", addr.addr);
                            if let Err(_) = state.devices.push(ITagEntry {
                                addr,
                                rssi,
                                name,
                                last_seen: Instant::now(),
                            }) {
                                esp_println::println!("App: Device list full");
                            }
                        }
                        DISPLAY_SIGNAL.signal(());
                    }
                }
            }
            Either3::Second(button_event) => {
                let mut state = APP_STATE.lock().await;
                match button_event {
                    ButtonEvent::Up => {
                        if state.devices.len() > 0 {
                            if state.selected_index == 0 {
                                state.selected_index = state.devices.len() - 1;
                            } else {
                                state.selected_index -= 1;
                            }
                        }
                    }
                    ButtonEvent::Down => {
                        if state.devices.len() > 0 {
                            state.selected_index = (state.selected_index + 1) % state.devices.len();
                        }
                    }
                    _ => {}
                }
                DISPLAY_SIGNAL.signal(());
            }
            Either3::Third(_) => {
                // Periodic aging
                let mut state = APP_STATE.lock().await;
                let now = Instant::now();
                let initial_count = state.devices.len();
                
                state.devices.retain(|d| {
                    let age = now.duration_since(d.last_seen);
                    if age >= Duration::from_secs(30) {
                        esp_println::println!("App: Aging out device {:?}", d.addr.addr);
                        false
                    } else {
                        true
                    }
                });

                if state.devices.len() != initial_count {
                    if state.devices.is_empty() {
                        state.selected_index = 0;
                    } else if state.selected_index >= state.devices.len() {
                        state.selected_index = state.devices.len() - 1;
                    }
                    DISPLAY_SIGNAL.signal(());
                }
            }
        }
    }
}
