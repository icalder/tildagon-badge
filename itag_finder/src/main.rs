#![no_std]
#![no_main]

mod ble;
mod itag;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_radio::ble::controller::BleConnector;
use tildagon::hardware::TildagonHardware;
use trouble_host::prelude::*;

pub type BleExternalController = ExternalController<BleConnector<'static>, 1>;

esp_bootloader_esp_idf::esp_app_desc!();

#[embassy_executor::task]
async fn hello_world_task() {
    loop {
        esp_println::println!("Hello World");
        Timer::after(Duration::from_secs(1)).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut hardware = TildagonHardware::new(peripherals)
        .await
        .expect("Tildagon hardware init failed");

    let mut radio = hardware.init_radio().expect("Tildagon radio init failed");

    let ble_connector = radio
        .init_ble_connector(Default::default())
        .expect("BLE connector init failed");
    let controller: BleExternalController = ExternalController::new(ble_connector);
    
    let (
        Host {
            central,
            runner: ble_runner,
            ..
        },
        stack,
    ) = crate::ble::build_ble_stack(controller);

    static ITAG_HANDLER: crate::itag::ItagScannerHandler = crate::itag::ItagScannerHandler;

    spawner.spawn(crate::itag::ble_task(ble_runner, &ITAG_HANDLER).expect("spawn ble_task"));
    spawner.spawn(crate::itag::itag_task(central, stack).expect("spawn itag_task"));

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
