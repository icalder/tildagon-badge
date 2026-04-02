use core::sync::atomic::{AtomicBool, Ordering};

use esp_radio::{
    ble::{Config as BleConfig, controller::BleConnector},
    wifi::{ControllerConfig as WifiConfig, Interfaces, WifiController},
};

use crate::{Error, resources::RadioResources};

/// Default internal-RAM heap reservation used before initializing the radio stack.
///
/// Peak observed usage is ~83 KB (WiFi + BLE scanning). 112 KB leaves ~30 KB headroom
/// while still freeing 16 KB of internal SRAM compared to the original 128 KB, giving
/// the task stack adequate room for LED driver construction and future features.
pub const DEFAULT_RADIO_HEAP_SIZE: usize = 112 * 1024;

static RADIO_HEAP_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Shared badge radio handle that owns the WiFi and BLE peripherals.
pub struct TildagonRadio {
    wifi: Option<esp_hal::peripherals::WIFI<'static>>,
    bt: Option<esp_hal::peripherals::BT<'static>>,
}

impl TildagonRadio {
    pub(crate) fn new(
        resources: RadioResources<'static>,
    ) -> Self {
        Self {
            wifi: Some(resources.wifi),
            bt: Some(resources.bt),
        }
    }

    /// Initialize the WiFi controller and interfaces.
    pub fn init_wifi(
        &mut self,
        config: WifiConfig,
    ) -> Result<(WifiController<'static>, Interfaces<'static>), Error> {
        let wifi = self.wifi.take().ok_or(Error::WifiAlreadyInitialized)?;
        esp_radio::wifi::new(wifi, config).map_err(Error::Wifi)
    }

    /// Initialize the BLE HCI connector used by `trouble-host`.
    pub fn init_ble_connector(
        &mut self,
        config: BleConfig,
    ) -> Result<BleConnector<'static>, Error> {
        let bt = self.bt.take().ok_or(Error::BleAlreadyInitialized)?;
        BleConnector::new(bt, config).map_err(Error::BleInit)
    }
}

pub(crate) fn init_radio_heap_once() {
    if !RADIO_HEAP_INITIALIZED.swap(true, Ordering::AcqRel) {
        esp_alloc::heap_allocator!(size: DEFAULT_RADIO_HEAP_SIZE);
    }
}
