use core::cell::RefCell;
use core::sync::atomic::Ordering;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Timer};
use esp_println::println;
use trouble_host::central::Central;
use trouble_host::prelude::*;

use crate::BleExternalController;

static SEEN_ITAGS: embassy_sync::blocking_mutex::Mutex<
    CriticalSectionRawMutex,
    RefCell<crate::ble::SeenAddresses<16>>,
> = embassy_sync::blocking_mutex::Mutex::new(RefCell::new(crate::ble::SeenAddresses::new()));

#[derive(Debug, Clone, Copy)]
pub struct DiscoveryState {
    pub addr: Option<BdAddr>,
    pub name: [u8; 32],
    pub name_len: u8,
    pub service_found: bool,
    pub mfg_found: bool,
}

impl DiscoveryState {
    pub fn is_complete(&self) -> bool {
        self.name_len > 0 && self.service_found && self.mfg_found
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

impl Default for DiscoveryState {
    fn default() -> Self {
        Self {
            addr: None,
            name: [0; 32],
            name_len: 0,
            service_found: false,
            mfg_found: false,
        }
    }
}

pub struct ItagScannerHandler {
    seen_devices: RefCell<[Option<DiscoveryState>; 32]>,
}

impl ItagScannerHandler {
    pub const fn new() -> Self {
        Self {
            seen_devices: RefCell::new([None; 32]),
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

        // Check if we already have an entry for this device
        for entry in devices.iter_mut() {
            if let Some(state) = entry {
                if state.addr == Some(addr.addr) {
                    state.set_name(name);
                    if has_itag_service {
                        state.service_found = true;
                    }
                    if has_itag_mfg {
                        state.mfg_found = true;
                    }
                    return *state;
                }
            }
        }

        // If not found, add a new entry if there's space
        for entry in devices.iter_mut() {
            if entry.is_none() {
                let mut new_state = DiscoveryState::default();
                new_state.set_name(name);
                new_state.addr = Some(addr.addr);
                new_state.service_found = has_itag_service;
                new_state.mfg_found = has_itag_mfg;
                *entry = Some(new_state);
                return new_state;
            }
        }

        DiscoveryState::default()
    }

    fn remove_device(&self, addr: Address) {
        let mut devices = self.seen_devices.borrow_mut();
        for entry in devices.iter_mut() {
            if let Some(state) = entry {
                if state.addr == Some(addr.addr) {
                    *entry = None;
                    return;
                }
            }
        }
    }
}

impl EventHandler for ItagScannerHandler {
    fn on_adv_reports(&self, reports: LeAdvReportsIter<'_>) {
        for report in reports {
            if let Ok(report) = report {
                let name = crate::ble::advertised_name(report.data);
                let has_itag_service = crate::ble::has_service_uuid16(report.data, 0xFFE0)
                    || crate::ble::has_service_uuid16(report.data, 0x1803);
                let has_itag_mfg = crate::ble::has_manufacturer_data(report.data, 0x0105);
                let discovery_state = self.update_device(
                    Address {
                        kind: report.addr_kind,
                        addr: report.addr,
                    },
                    name,
                    has_itag_service,
                    has_itag_mfg,
                );

                if discovery_state.is_complete() {
                    let addr = Address {
                        kind: report.addr_kind,
                        addr: report.addr,
                    };

                    // Clear space for next discovery by removing from seen list (we'll add back if it's still there on next report)
                    self.remove_device(addr);

                    // Only process if we haven't seen this iTAG yet
                    let is_new = SEEN_ITAGS.lock(|seen| seen.borrow_mut().insert(addr));
                    if !is_new {
                        continue;
                    }

                    println!(
                        "BLE: Discovered {} [{:02X?} ({:?})], RSSI: {}",
                        discovery_state.name_as_str().unwrap_or("UNKNOWN"),
                        addr.addr.0,
                        addr.kind,
                        report.rssi
                    );

                    // Try to send to the scan result channel, ignore if full
                    // let _ = crate::ble::SCAN_RESULTS.try_send(addr);
                }
            }
        }
    }
}

#[embassy_executor::task]
pub async fn ble_task(
    mut runner: Runner<'static, BleExternalController, DefaultPacketPool>,
    handler: &'static ItagScannerHandler,
) {
    println!("BLE runner started");
    runner.run_with_handler(handler).await.unwrap();
}

#[embassy_executor::task]
pub async fn itag_task(
    mut central: Central<'static, BleExternalController, DefaultPacketPool>,
    stack: &'static Stack<'static, BleExternalController, DefaultPacketPool>,
) {
    println!("iTAG task started");
    let ble_scan_config =
        crate::ble::active_scan_config(Duration::from_secs(1), Duration::from_secs(1));

    loop {
        if crate::SHUTTING_DOWN.load(Ordering::Relaxed) {
            println!("Shutting down, stopping iTAG task");
            break;
        }

        println!("Scanning for iTAG devices...");
        let (next_central, addr) = crate::ble::scan_until_result(central, &ble_scan_config).await;
        central = next_central;

        println!("BLE: Probing device [{:02X?}]...", addr.addr.0);

        let filter = [(addr.kind, &addr.addr)];
        let config = crate::ble::build_connect_config(&filter);

        match central.connect(&config).await {
            Ok(connection) => {
                println!("BLE: Connected to [{:02X?}]", addr.addr.0);

                match GattClient::<BleExternalController, DefaultPacketPool, 10>::new(
                    stack,
                    &connection,
                )
                .await
                {
                    Ok(client) => {
                        println!("BLE: GATT client created, discovering services...");

                        use embassy_futures::select::{Either, select};

                        let gatt_task = client.task();
                        let discovery_task = async {
                            // Give some time for the connection to stabilize and MTU exchange if needed
                            Timer::after(Duration::from_millis(500)).await;

                            match embassy_time::with_timeout(
                                Duration::from_secs(10),
                                client.services(),
                            )
                            .await
                            {
                                Ok(Ok(services)) => {
                                    for service in services {
                                        println!("    Service: {:?}", service.uuid());
                                        if service.uuid() == Uuid::new_short(0xFFE0) {
                                            println!("    *** FOUND iTAG SERVICE 0xFFE0 ***");
                                            match client
                                                .characteristic_by_uuid::<[u8]>(
                                                    &service,
                                                    &Uuid::new_short(0xFFE1),
                                                )
                                                .await
                                            {
                                                Ok(c) => println!(
                                                    "    *** FOUND iTAG CHAR 0xFFE1 (handle {:?}) ***",
                                                    c.handle
                                                ),
                                                Err(e) => println!(
                                                    "    iTAG CHAR 0xFFE1 discovery failed: {:?}",
                                                    e
                                                ),
                                            }
                                        }
                                    }
                                }
                                Ok(Err(e)) => println!("BLE: Service discovery failed: {:?}", e),
                                Err(_) => println!("BLE: Service discovery timed out"),
                            }
                        };

                        match select(gatt_task, discovery_task).await {
                            Either::First(Ok(())) => {
                                println!("BLE: GATT task finished unexpectedly")
                            }
                            Either::First(Err(e)) => println!("BLE: GATT task failed: {:?}", e),
                            Either::Second(_) => println!("BLE: Discovery completed"),
                        }
                    }
                    Err(e) => println!("BLE: GATT client creation failed: {:?}", e),
                }
                drop(connection);
            }
            Err(e) => println!("BLE: Connection failed to [{:02X?}]: {:?}", addr.addr.0, e),
        }

        // Give some time before restarting scan
        Timer::after(Duration::from_secs(1)).await;
    }
}
