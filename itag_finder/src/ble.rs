use core::str;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::Duration;
use esp_hal::rng::Rng;
use static_cell::StaticCell;
use trouble_host::advertise::AdStructure;
use trouble_host::central::Central;
use trouble_host::prelude::*;

use crate::BleExternalController;

pub static SCAN_RESULTS: Channel<CriticalSectionRawMutex, Address, 4> = Channel::new();

pub struct SeenAddresses<const N: usize> {
    addresses: [Option<Address>; N],
    count: usize,
}

impl<const N: usize> SeenAddresses<N> {
    pub const fn new() -> Self {
        Self {
            addresses: [None; N],
            count: 0,
        }
    }

    fn contains(&self, addr: &Address) -> bool {
        for i in 0..self.count {
            if let Some(seen_addr) = self.addresses[i] {
                if seen_addr.kind == addr.kind && seen_addr.addr == addr.addr {
                    return true;
                }
            }
        }
        false
    }

    pub fn insert(&mut self, addr: Address) -> bool {
        if self.contains(&addr) {
            return false;
        }
        if self.count < N {
            self.addresses[self.count] = Some(addr);
            self.count += 1;
            true
        } else {
            false
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

pub fn has_service_uuid16(data: &[u8], target_uuid: u16) -> bool {
    for res in AdStructure::decode(data) {
        if let Ok(AdStructure::ServiceUuids16(uuids)) = res {
            for uuid in uuids {
                if u16::from_le_bytes(*uuid) == target_uuid {
                    return true;
                }
            }
        }
    }
    false
}

/// Start an active scan and wait until some higher-level handler publishes a
/// matching device address into `SCAN_RESULTS`.
///
/// The ownership flow here is a little unusual:
///
/// 1. We take ownership of `central` and wrap it in a `Scanner`, because the
///    trouble-host API performs scanning through that wrapper rather than
///    directly on `Central`.
/// 2. Calling `scanner.scan(...)` starts scanning and returns a `scan_session`.
///    Keeping that session alive is what keeps the scan running.
/// 3. While the scan is active, the BLE runner task delivers advertisement
///    reports to the current `EventHandler`. That handler decides which
///    advertisements are interesting and sends their `Address` values into
///    `SCAN_RESULTS`.
/// 4. This function waits on `SCAN_RESULTS.receive()` until one such address
///    arrives. It does not inspect advertisements itself; it only manages the
///    generic scan lifecycle and hands back the first matching result.
/// 5. After receiving one address, we drain any queued extras so the caller
///    does not immediately reconnect to stale discoveries from the same scan.
/// 6. We then drop `scan_session` explicitly. That stops the active scan before
///    we try to use the same controller for a connection attempt.
/// 7. Finally, `scanner.into_inner()` gives ownership of `Central` back to the
///    caller together with the chosen address.
///
/// In short: this helper owns the scan start/stop mechanics, while the
/// caller's event handler owns the policy for deciding which advertisements are
/// worth returning.
pub async fn scan_until_result(
    central: Central<'static, BleExternalController, DefaultPacketPool>,
    scan_config: &ScanConfig<'_>,
) -> (
    Central<'static, BleExternalController, DefaultPacketPool>,
    Address,
) {
    let mut scanner = Scanner::new(central);
    let scan_session = scanner
        .scan(scan_config)
        .await
        .expect("BLE scan failed");

    let addr = SCAN_RESULTS.receive().await;
    while SCAN_RESULTS.try_receive().is_ok() {}

    drop(scan_session);

    (scanner.into_inner(), addr)
}
