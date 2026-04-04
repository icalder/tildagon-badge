use core::str;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::rng::Rng;
use static_cell::StaticCell;
use trouble_host::advertise::AdStructure;
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
    pub last_emitted_rssi: i8,
    pub last_emit_at: Option<Instant>,
    pub emitted_with_name: bool,
}

const RSSI_EMIT_THRESHOLD_DBM: i16 = 2;
const MIN_EMIT_INTERVAL: Duration = Duration::from_millis(1500);

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

    fn should_emit(&self, rssi: i8, now: Instant) -> bool {
        let Some(last_emit_at) = self.last_emit_at else {
            return true;
        };

        let rssi_delta = (rssi as i16 - self.last_emitted_rssi as i16).abs();
        let name_gained = self.name_len > 0 && !self.emitted_with_name;

        name_gained
            || rssi_delta >= RSSI_EMIT_THRESHOLD_DBM
            || now.duration_since(last_emit_at) >= MIN_EMIT_INTERVAL
    }

    fn mark_emitted(&mut self, rssi: i8, now: Instant) {
        self.last_emitted_rssi = rssi;
        self.last_emit_at = Some(now);
        self.emitted_with_name = self.name_len > 0;
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
        rssi: i8,
    ) -> Option<DiscoveryState> {
        let mut devices = self.seen_devices.borrow_mut();
        let now = Instant::now();

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
                    if state.is_complete() && state.should_emit(rssi, now) {
                        state.mark_emitted(rssi, now);
                        return Some(*state);
                    }
                    return None;
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
                    esp_println::println!(
                        "New iTag candidate: {:?} (svc={}, mfg={})",
                        addr.addr,
                        has_itag_service,
                        has_itag_mfg
                    );
                    new_state.mark_emitted(rssi, now);
                    *entry = Some(new_state);
                    return Some(new_state);
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
            let mut emitted_state = new_state;
            emitted_state.mark_emitted(rssi, now);
            devices[idx] = Some(emitted_state);
            return Some(emitted_state);
        }

        None
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

                if let Some(discovery_state) =
                    self.update_device(addr, name, has_itag_service, has_itag_mfg, report.rssi)
                {
                    let mut heapless_name = None;
                    if let Some(n) = discovery_state.name_as_str() {
                        let mut s = heapless::String::new();
                        let _ = s.push_str(n);
                        heapless_name = Some(s);
                    }

                    if let Err(_) =
                        BLE_EVENTS.try_send(BleEvent::DeviceSeen(addr, report.rssi, heapless_name))
                    {
                        esp_println::println!(
                            "BLE: Channel full, dropping report for {:?}",
                            addr.addr
                        );
                    } else {
                        esp_println::println!(
                            "BLE: queued detection {:?} rssi={} name={:?}",
                            addr.addr,
                            report.rssi,
                            discovery_state.name_as_str()
                        );
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

pub fn build_connect_config<'a>(
    filter_accept_list: &'a [(AddrKind, &'a BdAddr)],
) -> ConnectConfig<'a> {
    ConnectConfig {
        scan_config: ScanConfig {
            active: true,
            interval: Duration::from_millis(100),
            window: Duration::from_millis(100),
            filter_accept_list,
            phys: PhySet::M1,
            timeout: Duration::from_secs(10),
        },
        connect_params: RequestedConnParams {
            min_connection_interval: Duration::from_millis(30),
            max_connection_interval: Duration::from_millis(60),
            max_latency: 0,
            supervision_timeout: Duration::from_secs(10),
            min_event_length: Duration::from_millis(0),
            max_event_length: Duration::from_millis(0),
        },
    }
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
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        esp_println::println!("BLE runner starting (attempt {})", attempt);
        match runner.run_with_handler(handler).await {
            Ok(()) => {
                esp_println::println!("BLE runner exited cleanly; restarting");
            }
            Err(e) => {
                esp_println::println!("BLE runner error: {:?}; restarting", e);
            }
        }
        Timer::after(Duration::from_millis(250)).await;
    }
}
