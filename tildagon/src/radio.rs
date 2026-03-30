use core::sync::atomic::{AtomicBool, Ordering};

use esp_radio::{
    Controller,
    ble::{Config as BleConfig, controller::BleConnector},
    wifi::{Config as WifiConfig, Interfaces, WifiController},
};
use static_cell::StaticCell;

use crate::{Error, resources::RadioResources};

/// Default internal-RAM heap reservation used before initializing the radio stack.
///
/// Peak observed usage is ~83 KB (WiFi + BLE scanning). 112 KB leaves ~30 KB headroom
/// while still freeing 16 KB of internal SRAM compared to the original 128 KB, giving
/// the task stack adequate room for LED driver construction and future features.
pub const DEFAULT_RADIO_HEAP_SIZE: usize = 112 * 1024;

static RADIO_CELL: StaticCell<Controller<'static>> = StaticCell::new();
static RADIO_HEAP_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Shared badge radio handle that owns the WiFi and BLE peripherals.
pub struct TildagonRadio {
    controller: &'static Controller<'static>,
    wifi: Option<esp_hal::peripherals::WIFI<'static>>,
    bt: Option<esp_hal::peripherals::BT<'static>>,
}

impl TildagonRadio {
    pub(crate) fn new(
        controller: &'static Controller<'static>,
        resources: RadioResources<'static>,
    ) -> Self {
        Self {
            controller,
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
        esp_radio::wifi::new(self.controller, wifi, config).map_err(Error::Wifi)
    }

    /// Initialize the BLE HCI connector used by `trouble-host`.
    pub fn init_ble_connector(
        &mut self,
        config: BleConfig,
    ) -> Result<BleConnector<'static>, Error> {
        let bt = self.bt.take().ok_or(Error::BleAlreadyInitialized)?;
        BleConnector::new(self.controller, bt, config).map_err(Error::BleConfig)
    }
}

fn init_radio_heap_once() {
    if !RADIO_HEAP_INITIALIZED.swap(true, Ordering::AcqRel) {
        esp_alloc::heap_allocator!(size: DEFAULT_RADIO_HEAP_SIZE);
    }
}

pub(crate) fn init_radio_controller() -> Result<&'static Controller<'static>, Error> {
    init_radio_heap_once();

    if let Some(cell) = RADIO_CELL.try_uninit() {
        Ok(cell.write(esp_radio::init().map_err(Error::Radio)?))
    } else {
        Err(Error::RadioAlreadyInitialized)
    }
}
