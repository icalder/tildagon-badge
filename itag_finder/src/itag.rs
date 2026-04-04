use crate::BleExternalController;
use crate::events::{BLE_EVENTS, BUTTON_EVENTS, BleEvent, ButtonEvent, DISPLAY_SIGNAL};
use core::sync::atomic::Ordering;
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Instant, Timer};
use esp_println::println;
use heapless::Vec;
use trouble_host::central::Central;
use trouble_host::prelude::*;

const DEVICE_STALE_AFTER: Duration = Duration::from_secs(30);
const SCAN_IDLE_RESTART_AFTER: Duration = Duration::from_secs(2);
const SCAN_SESSION_RESTART_AFTER: Duration = Duration::from_secs(10);
const IMMEDIATE_ALERT_SERVICE_UUID: u16 = 0x1802;
const ALERT_LEVEL_CHARACTERISTIC_UUID: u16 = 0x2A06;
const ALERT_LEVEL_OFF: u8 = 0x00;
const ALERT_LEVEL_MILD: u8 = 0x01;

#[derive(Debug, Clone)]
pub struct ITagEntry {
    pub addr: Address,
    pub rssi: i8,
    pub name: Option<heapless::String<32>>,
    pub last_seen: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Scanning,
    Connecting,
    Alarming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RssiHeatLevel {
    Hot,
    Warm,
    Near,
    Cool,
    Cold,
    Far,
}

pub fn rssi_heat_level(rssi: i8) -> RssiHeatLevel {
    // RSSI is already logarithmic (dBm), so bias the buckets toward weaker
    // values to keep nearby devices in the warmer colors longer.
    if rssi >= -58 {
        RssiHeatLevel::Hot
    } else if rssi >= -66 {
        RssiHeatLevel::Warm
    } else if rssi >= -74 {
        RssiHeatLevel::Near
    } else if rssi >= -84 {
        RssiHeatLevel::Cool
    } else if rssi >= -96 {
        RssiHeatLevel::Cold
    } else {
        RssiHeatLevel::Far
    }
}

pub struct AppState {
    pub devices: Vec<ITagEntry, 16>,
    pub selected_index: usize,
    pub radar_mode_enabled: bool,
    pub mode: Mode,
    pub target_addr: Option<Address>,
}

pub static APP_STATE: Mutex<CriticalSectionRawMutex, AppState> = Mutex::new(AppState {
    devices: Vec::new(),
    selected_index: 0,
    radar_mode_enabled: false,
    mode: Mode::Scanning,
    target_addr: None,
});

async fn write_alert_level(
    client: &GattClient<'_, BleExternalController, DefaultPacketPool, 10>,
    characteristic: &Characteristic<[u8]>,
    level: u8,
) -> bool {
    match client.write_characteristic(characteristic, &[level]).await {
        Ok(()) => true,
        Err(write_err) => {
            println!(
                "BLE: write_characteristic failed for alert level {}: {:?}; retrying without response",
                level, write_err
            );
            match client
                .write_characteristic_without_response(characteristic, &[level])
                .await
            {
                Ok(()) => true,
                Err(without_response_err) => {
                    println!(
                        "BLE: write_characteristic_without_response failed for alert level {}: {:?}",
                        level, without_response_err
                    );
                    false
                }
            }
        }
    }
}

async fn disconnect_and_wait(connection: &Connection<'_, DefaultPacketPool>) {
    if !connection.is_connected() {
        return;
    }

    println!(
        "BLE: Requesting disconnect from {:?}",
        connection.peer_address()
    );
    connection.disconnect();

    match embassy_time::with_timeout(Duration::from_secs(2), async {
        loop {
            if let ConnectionEvent::Disconnected { reason } = connection.next().await {
                println!("BLE: Disconnected: {:?}", reason);
                break;
            }
        }
    })
    .await
    {
        Ok(()) => {}
        Err(_) => println!("BLE: Timed out waiting for disconnect"),
    }
}

#[embassy_executor::task]
pub async fn itag_task(
    mut central: Central<'static, BleExternalController, DefaultPacketPool>,
    stack: &'static Stack<'static, BleExternalController, DefaultPacketPool>,
) {
    println!("App Controller task started");
    let ble_scan_config =
        crate::ble::active_scan_config(Duration::from_millis(100), Duration::from_millis(100));

    loop {
        if crate::SHUTTING_DOWN.load(Ordering::Relaxed) {
            break;
        }

        let mode = {
            let state = APP_STATE.lock().await;
            state.mode
        };

        match mode {
            Mode::Scanning => {
                println!("BLE: Starting scan");
                let mut scanner = Scanner::new(central);
                let scan_session = scanner
                    .scan(&ble_scan_config)
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
                                        state.selected_index =
                                            (state.selected_index + 1) % state.devices.len();
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
                                if now.duration_since(last_detection_at) >= SCAN_IDLE_RESTART_AFTER
                                {
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
            }
            Mode::Connecting => {
                let target_addr = {
                    let state = APP_STATE.lock().await;
                    state.target_addr.unwrap()
                };

                println!("BLE: Connecting to {:?}", target_addr.addr);
                let filter = [(target_addr.kind, &target_addr.addr)];
                let config = crate::ble::build_connect_config(&filter);

                match select(central.connect(&config), BUTTON_EVENTS.receive()).await {
                    Either::First(Ok(connection)) => {
                        println!("BLE: Connected to {:?}", target_addr.addr);
                        match GattClient::<BleExternalController, DefaultPacketPool, 10>::new(
                            stack,
                            &connection,
                        )
                        .await
                        {
                            Ok(client) => {
                                let gatt_task = client.task();
                                let alarm_task = async {
                                    Timer::after(Duration::from_millis(500)).await;
                                    match client
                                        .services_by_uuid(&Uuid::new_short(
                                            IMMEDIATE_ALERT_SERVICE_UUID,
                                        ))
                                        .await
                                    {
                                        Ok(services) if !services.is_empty() => {
                                            let service = &services[0];
                                            match client
                                                .characteristic_by_uuid::<[u8]>(
                                                    service,
                                                    &Uuid::new_short(
                                                        ALERT_LEVEL_CHARACTERISTIC_UUID,
                                                    ),
                                                )
                                                .await
                                            {
                                                Ok(characteristic) => {
                                                    println!("BLE: Triggering mild alert...");
                                                    if write_alert_level(
                                                        &client,
                                                        &characteristic,
                                                        ALERT_LEVEL_MILD,
                                                    )
                                                    .await
                                                    {
                                                        let mut state = APP_STATE.lock().await;
                                                        state.mode = Mode::Alarming;
                                                        DISPLAY_SIGNAL.signal(());
                                                    } else {
                                                        println!(
                                                            "BLE: Failed to trigger alert level {}",
                                                            ALERT_LEVEL_MILD
                                                        );
                                                        return;
                                                    }

                                                    loop {
                                                        match select(
                                                            Timer::after(Duration::from_secs(2)),
                                                            BUTTON_EVENTS.receive(),
                                                        )
                                                        .await
                                                        {
                                                            Either::First(_) => {}
                                                            Either::Second(ButtonEvent::Back)
                                                            | Either::Second(ButtonEvent::Select) =>
                                                            {
                                                                println!("BLE: Stopping alarm");
                                                                if !write_alert_level(
                                                                    &client,
                                                                    &characteristic,
                                                                    ALERT_LEVEL_OFF,
                                                                )
                                                                .await
                                                                {
                                                                    println!(
                                                                        "BLE: Failed to stop alert level {}",
                                                                        ALERT_LEVEL_OFF
                                                                    );
                                                                }
                                                                break;
                                                            }
                                                            _ => {}
                                                        }
                                                    }
                                                }
                                                Err(e) => println!(
                                                    "BLE: Characteristic 0x{:04X} not found: {:?}",
                                                    ALERT_LEVEL_CHARACTERISTIC_UUID, e
                                                ),
                                            }
                                        }
                                        _ => println!(
                                            "BLE: Service 0x{:04X} not found",
                                            IMMEDIATE_ALERT_SERVICE_UUID
                                        ),
                                    }
                                };

                                match select3(gatt_task, alarm_task, BUTTON_EVENTS.receive()).await
                                {
                                    Either3::First(_) => println!("BLE: GATT task finished"),
                                    Either3::Second(_) => println!("BLE: Alarm task finished"),
                                    Either3::Third(ButtonEvent::Back) => {
                                        println!("BLE: Back pressed, disconnecting")
                                    }
                                    Either3::Third(_) => {}
                                }
                            }
                            Err(e) => println!("BLE: GATT client failed: {:?}", e),
                        }
                        disconnect_and_wait(&connection).await;
                        {
                            let mut state = APP_STATE.lock().await;
                            state.mode = Mode::Scanning;
                            state.target_addr = None;
                            DISPLAY_SIGNAL.signal(());
                        }
                    }
                    Either::First(Err(e)) => {
                        println!("BLE: Connection failed: {:?}", e);
                        let mut state = APP_STATE.lock().await;
                        state.mode = Mode::Scanning;
                        DISPLAY_SIGNAL.signal(());
                    }
                    Either::Second(ButtonEvent::Back) => {
                        println!("BLE: Connection cancelled");
                        let mut state = APP_STATE.lock().await;
                        state.mode = Mode::Scanning;
                        DISPLAY_SIGNAL.signal(());
                    }
                    Either::Second(_) => {}
                }
            }
            Mode::Alarming => {
                // This state is handled inside the Connecting match's connection success block
                // but we should set it back to scanning if we somehow get here without a connection.
                let mut state = APP_STATE.lock().await;
                state.mode = Mode::Scanning;
            }
        }
    }
}
