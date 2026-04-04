use core::str;
use embassy_time::Duration;
use esp_hal::rng::Rng;
use static_cell::StaticCell;
use trouble_host::advertise::AdStructure;
use trouble_host::central::Central;
use trouble_host::prelude::*;

use crate::BleExternalController;
use crate::events::{BLE_EVENTS, BleEvent};

use core::cell::{Cell, RefCell};

#[derive(Debug, Clone, Copy, Default)]
pub struct DiscoveryState {
    pub addr: Option<BdAddr>,
    pub name: [u8; 32],
    pub name_len: u8,
    pub service_found: bool,
    pub mfg_found: bool,
}

impl DiscoveryState {
    pub fn is_complete(&self) -> bool {
        // Relaxed: either service or manufacturer data is enough to identify as iTag
        self.service_found || self.mfg_found
    }

    pub fn name_as_str(&self) -> Option<&str> {
        if self.name_len == 0 {
            None
        } else {
            core::str::from_utf8(&self.name[..self.name_len as usize]).ok()
        }
    }

    pub fn set_name(&mut self, name: Option<&str>) {
        if let Some(n) = name {
            let bytes = n.as_bytes();
            let len = core::cmp::min(32, bytes.len());
            self.name[..len].copy_from_slice(&bytes[..len]);
            self.name_len = len as u8;
        }
    }
}

pub struct ScannerHandler {
    seen_devices: RefCell<[Option<DiscoveryState>; 64]>,
    next_eviction: Cell<usize>,
}

impl ScannerHandler {
    pub const fn new() -> Self {
        Self {
            seen_devices: RefCell::new([None; 64]),
            next_eviction: Cell::new(0),
        }
    }

    fn update_device(
        &self,
        addr: Address,
        name: Option<&str>,
        has_itag_service: bool,
        has_itag_mfg: bool,
    ) -> DiscoveryState {
        let mut devices = self.seen_devices.borrow_mut();

        // 1. Try to find existing entry
        for entry in devices.iter_mut() {
            if let Some(state) = entry {
                if state.addr == Some(addr.addr) {
                    if name.is_some() && state.name_len == 0 {
                        state.set_name(name);
                    }
                    if has_itag_service && !state.service_found {
                        state.service_found = true;
                        esp_println::println!("Device {:?} now has service", addr.addr);
                    }
                    if has_itag_mfg && !state.mfg_found {
                        state.mfg_found = true;
                        esp_println::println!("Device {:?} now has mfg data", addr.addr);
                    }
                    return *state;
                }
            }
        }

        // 2. Not found. Only add if it has iTag markers (service or mfg)
        if has_itag_service || has_itag_mfg {
            // Find an empty slot
            for entry in devices.iter_mut() {
                if entry.is_none() {
                    let mut new_state = DiscoveryState::default();
                    new_state.set_name(name);
                    new_state.addr = Some(addr.addr);
                    new_state.service_found = has_itag_service;
                    new_state.mfg_found = has_itag_mfg;
                    *entry = Some(new_state);
                    esp_println::println!("New iTag candidate: {:?} (svc={}, mfg={})", addr.addr, has_itag_service, has_itag_mfg);
                    return new_state;
                }
            }

            // 3. Cache full, evict one (round-robin)
            let idx = self.next_eviction.get();
            let mut new_state = DiscoveryState::default();
            new_state.set_name(name);
            new_state.addr = Some(addr.addr);
            new_state.service_found = has_itag_service;
            new_state.mfg_found = has_itag_mfg;
            devices[idx] = Some(new_state);
            self.next_eviction.set((idx + 1) % 64);
            esp_println::println!("Cache full, evicting slot {} for {:?}", idx, addr.addr);
            return new_state;
        }
        
        DiscoveryState::default()
    }
}

impl EventHandler for ScannerHandler {
    fn on_adv_reports(&self, reports: LeAdvReportsIter<'_>) {
        for report in reports {
            if let Ok(report) = report {
                let name = advertised_name(report.data);
                let has_itag_service = has_service_uuid16(report.data, 0xFFE0)
                    || has_service_uuid16(report.data, 0x1802)
                    || has_service_uuid16(report.data, 0x1803);
                let has_itag_mfg = has_manufacturer_data(report.data, 0x0105);
                
                let addr = Address {
                    kind: report.addr_kind,
                    addr: report.addr,
                };

                let discovery_state = self.update_device(addr, name, has_itag_service, has_itag_mfg);

                if discovery_state.is_complete() {
                    let mut heapless_name = None;
                    if let Some(n) = discovery_state.name_as_str() {
                        let mut s = heapless::String::new();
                        let _ = s.push_str(n);
                        heapless_name = Some(s);
                    }
                    
                    if let Err(_) = BLE_EVENTS.try_send(BleEvent::DeviceSeen(
                        addr,
                        report.rssi,
                        heapless_name,
                    )) {
                        esp_println::println!("BLE: Channel full, dropping report for {:?}", addr.addr);
                    } else {
                        // Success: app should receive it
                    }
                }
            }
        }
    }
}

fn random_ble_address() -> Address {
    let rng = Rng::new();
    let mut bytes = [0u8; 6];
    rng.read(&mut bytes);
    Address::random(bytes)
}

pub fn build_ble_stack(
    controller: BleExternalController,
) -> (
    Host<'static, BleExternalController, DefaultPacketPool>,
    &'static Stack<'static, BleExternalController, DefaultPacketPool>,
) {
    static BLE_RESOURCES: StaticCell<HostResources<DefaultPacketPool, 1, 1, 1>> = StaticCell::new();
    let ble_resources = BLE_RESOURCES.init(HostResources::new());

    let address = random_ble_address();

    static BLE_STACK: StaticCell<Stack<'static, BleExternalController, DefaultPacketPool>> =
        StaticCell::new();
    let ble_stack =
        BLE_STACK.init(trouble_host::new(controller, ble_resources).set_random_address(address));

    (ble_stack.build(), ble_stack)
}

pub fn active_scan_config(interval: Duration, window: Duration) -> ScanConfig<'static> {
    let mut config = ScanConfig::default();
    config.active = true;
    config.interval = interval;
    config.window = window;
    config
}

pub fn advertised_name(data: &[u8]) -> Option<&str> {
    let mut shortened_name = None;

    for structure in AdStructure::decode(data).flatten() {
        match structure {
            AdStructure::CompleteLocalName(name) => return str::from_utf8(name).ok(),
            AdStructure::ShortenedLocalName(name) => {
                if shortened_name.is_none() {
                    shortened_name = str::from_utf8(name).ok();
                }
            }
            _ => {}
        }
    }

    shortened_name
}

pub fn has_manufacturer_data(data: &[u8], target_id: u16) -> bool {
    for res in AdStructure::decode(data) {
        if let Ok(AdStructure::ManufacturerSpecificData {
            company_identifier, ..
        }) = res
        {
            if company_identifier == target_id {
                return true;
            }
        }
    }
    false
}

pub fn has_service_uuid16(data: &[u8], target_uuid: u16) -> bool {
    for res in AdStructure::decode(data) {
        if let Ok(structure) = res {
            match structure {
                AdStructure::ServiceUuids16(uuids) => {
                    for uuid in uuids {
                        if u16::from_le_bytes(*uuid) == target_uuid {
                            return true;
                        }
                    }
                }
                // Also handle cases where decode might return Unknown for these types if not fully supported
                AdStructure::Unknown { ty, data } if ty == 0x02 || ty == 0x03 => {
                    for chunk in data.chunks_exact(2) {
                        if let Ok(uuid_bytes) = chunk.try_into() {
                            if u16::from_le_bytes(uuid_bytes) == target_uuid {
                                return true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    false
}

#[embassy_executor::task]
pub async fn ble_task(
    mut runner: Runner<'static, BleExternalController, DefaultPacketPool>,
    handler: &'static ScannerHandler,
) {
    esp_println::println!("BLE runner started");
    runner.run_with_handler(handler).await.unwrap();
}

#[embassy_executor::task]
pub async fn scanner_task(
    central: Central<'static, BleExternalController, DefaultPacketPool>,
) {
    esp_println::println!("Scanner task started");
    let mut scanner = Scanner::new(central);
    let config = active_scan_config(Duration::from_millis(100), Duration::from_millis(100));
    
    loop {
        let _scan_session = scanner.scan(&config).await.expect("BLE scan failed");
        
        // Restart scan every 10 seconds to keep it fresh
        embassy_futures::select::select(
            embassy_time::Timer::after(Duration::from_secs(10)),
            async {
                loop {
                    if crate::SHUTTING_DOWN.load(core::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    embassy_time::Timer::after(Duration::from_secs(1)).await;
                }
            }
        ).await;

        if crate::SHUTTING_DOWN.load(core::sync::atomic::Ordering::Relaxed) {
            break;
        }
    }
}
