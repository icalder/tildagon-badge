#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use tildagon::hardware::TildagonHardware;

esp_bootloader_esp_idf::esp_app_desc!();

#[embassy_executor::task]
async fn hello_world_task() {
    loop {
        esp_println::println!("Hello World");
        Timer::after(Duration::from_secs(1)).await;
    }
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let _hardware = TildagonHardware::new(peripherals)
        .await
        .expect("Tildagon hardware init failed");

    _spawner.spawn(hello_world_task().expect("spawn hello_world_task"));
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
