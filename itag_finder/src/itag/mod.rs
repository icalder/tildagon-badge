use crate::BleExternalController;
use core::sync::atomic::Ordering;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Instant};
use esp_println::println;
use heapless::Vec;
use trouble_host::central::Central;
use trouble_host::prelude::*;

mod connecting;
mod scanning;

const DEVICE_STALE_AFTER: Duration = Duration::from_secs(20);
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
    if rssi >= -68 {
        RssiHeatLevel::Hot
    } else if rssi >= -76 {
        RssiHeatLevel::Warm
    } else if rssi >= -84 {
        RssiHeatLevel::Near
    } else if rssi >= -92 {
        RssiHeatLevel::Cool
    } else if rssi >= -100 {
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
                central = scanning::run_scanning(central, &ble_scan_config).await;
            }
            Mode::Connecting => {
                connecting::run_connecting(&mut central, stack).await;
            }
            Mode::Alarming => {
                connecting::handle_alarming().await;
            }
        }
    }
}

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
