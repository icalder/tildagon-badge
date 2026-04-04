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

pub struct ItagScannerHandler;

impl EventHandler for ItagScannerHandler {
    fn on_adv_reports(&self, reports: LeAdvReportsIter<'_>) {
        for report in reports {
            if let Ok(report) = report {
                let name = crate::ble::advertised_name(report.data);
                let is_itag_name = name
                    .as_ref()
                    .map(|n| n.to_lowercase().contains("itag"))
                    .unwrap_or(false);
                let has_itag_service = crate::ble::has_service_uuid16(report.data, 0xFFE0)
                    || crate::ble::has_service_uuid16(report.data, 0x1803);

                if is_itag_name || has_itag_service {
                    let addr = Address {
                        kind: report.addr_kind,
                        addr: report.addr,
                    };

                    // Only process if we haven't seen this iTAG yet
                    let is_new = SEEN_ITAGS.lock(|seen| seen.borrow_mut().insert(addr));
                    if !is_new {
                        continue;
                    }

                    println!(
                        "BLE: Discovered {} [{:02X?} ({:?})], RSSI: {}",
                        name.unwrap_or("<unknown>"),
                        addr.addr.0,
                        addr.kind,
                        report.rssi
                    );

                    // Try to send to the scan result channel, ignore if full
                    let _ = crate::ble::SCAN_RESULTS.try_send(addr);
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
