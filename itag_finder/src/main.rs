#![no_std]
#![no_main]

mod ble;
mod buttons;
mod display;
mod itag;

use core::sync::atomic::AtomicBool;
use embassy_executor::Spawner;
use embassy_sync::mutex::Mutex as AsyncMutex;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_radio::ble::controller::BleConnector;
use static_cell::StaticCell;
use tildagon::battery::Battery;
use tildagon::hardware::TildagonHardware;
use tildagon::i2c::{SharedI2cBus, system_i2c_bus};
use trouble_host::prelude::*;

pub type BleExternalController = ExternalController<BleConnector<'static>, 1>;

static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

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

    static DISPLAY_BUFFER: StaticCell<[u8; 1024]> = StaticCell::new();
    let display_buffer = DISPLAY_BUFFER.init([0u8; 1024]);
    let display = hardware
        .init_display(display_buffer)
        .expect("Display init failed");

    let mut radio = hardware.init_radio().expect("Tildagon radio init failed");

    static SHARED_I2C: StaticCell<
        SharedI2cBus<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>,
    > = StaticCell::new();
    let shared_i2c = SHARED_I2C.init(AsyncMutex::new(hardware.i2c.into_async()));

    // Start the background button service
    let button_manager = TildagonHardware::init_button_manager(&spawner, shared_i2c);

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

    spawner.spawn(
        crate::buttons::button_monitor(
            button_manager.subscribe(),
            Battery::new(system_i2c_bus(shared_i2c)),
        )
        .expect("Failed to spawn button_monitor"),
    );
    static ITAG_HANDLER: StaticCell<crate::itag::ItagScannerHandler> = StaticCell::new();
    let itag_handler = ITAG_HANDLER.init(crate::itag::ItagScannerHandler::new());
    spawner.spawn(crate::itag::ble_task(ble_runner, itag_handler).expect("spawn ble_task"));
    spawner.spawn(crate::itag::itag_task(central, stack).expect("spawn itag_task"));
    spawner.spawn(crate::display::display_task(display).expect("spawn display_task"));

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
