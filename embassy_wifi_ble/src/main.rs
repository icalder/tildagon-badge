#![no_std]
#![no_main]

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

use alloc::string::ToString;
use bt_hci::controller::ExternalController;
use bt_hci::param::LeAdvReportsIter;
use core::str;
use embassy_executor::Spawner;
use embassy_net::{Runner as NetRunner, StackResources};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::rng::Rng;
use esp_println::println;
use esp_radio::ble::controller::BleConnector;
use esp_radio::wifi::{
    ClientConfig, Config as WifiConfig, ScanConfig as WifiScanConfig, WifiController, WifiDevice,
    WifiEvent,
};
use static_cell::StaticCell;
use tildagon::hardware::TildagonHardware;
use trouble_host::advertise::AdStructure;
use trouble_host::central::Central;
use trouble_host::prelude::*;

const SSID: &str = "YOUR_SSID";
const PASSWORD: &str = "YOUR_PASSWORD";

#[embassy_executor::task]
async fn net_task(mut runner: NetRunner<'static, WifiDevice<'static>>) {
    runner.run().await;
}

#[embassy_executor::task]
async fn wifi_connection_task(mut controller: WifiController<'static>) {
    println!("WiFi connection task started");
    loop {
        if matches!(controller.is_started(), Ok(false)) {
            controller.start_async().await.expect("WiFi start failed");
        }

        println!("Connecting to {}...", SSID);
        let config = ClientConfig::default()
            .with_ssid(SSID.to_string())
            .with_password(PASSWORD.to_string());

        controller
            .set_config(&esp_radio::wifi::ModeConfig::Client(config))
            .unwrap();

        match controller.connect_async().await {
            Ok(_) => {
                println!("WiFi connected!");
                // Wait for disconnect
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                println!("WiFi disconnected!");
            }
            Err(e) => {
                println!("WiFi connect error: {:?}", e);
                Timer::after(Duration::from_secs(5)).await;
            }
        }
    }
}

#[embassy_executor::task]
async fn wifi_scan_task(mut controller: WifiController<'static>) {
    println!("WiFi scan task started");
    loop {
        if matches!(controller.is_started(), Ok(false)) {
            let config = ClientConfig::default();
            controller
                .set_config(&esp_radio::wifi::ModeConfig::Client(config))
                .unwrap();
            controller.start_async().await.expect("WiFi start failed");
        }

        println!("Scanning for WiFi networks...");
        let config = WifiScanConfig::default();
        match controller.scan_with_config_async(config).await {
            Ok(networks) => {
                println!("Found {} networks:", networks.len());
                for network in networks {
                    println!(
                        "SSID: {:15} | BSSID: {:02X?}:{:02X?}:{:02X?}:{:02X?}:{:02X?}:{:02X?} | RSSI: {:4} | Channel: {:2}",
                        network.ssid,
                        network.bssid[0],
                        network.bssid[1],
                        network.bssid[2],
                        network.bssid[3],
                        network.bssid[4],
                        network.bssid[5],
                        network.signal_strength,
                        network.channel
                    );
                }
            }
            Err(e) => {
                println!("WiFi scan error: {:?}", e);
            }
        }
        Timer::after(Duration::from_secs(10)).await;
    }
}

struct ScannerHandler;

fn advertised_name(data: &[u8]) -> Option<&str> {
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

impl EventHandler for ScannerHandler {
    fn on_adv_reports(&self, reports: LeAdvReportsIter<'_>) {
        for report in reports {
            if let Ok(report) = report {
                if let Some(name) = advertised_name(report.data) {
                    println!("BLE: Discovered {name}, RSSI: {}", report.rssi);
                } else {
                    println!("BLE: Discovered {:?}, RSSI: {}", report.addr, report.rssi);
                }
            }
        }
    }
}

type BleExternalController = ExternalController<BleConnector<'static>, 1>;

fn random_seed() -> u64 {
    let rng = Rng::new();
    ((rng.random() as u64) << 32) | rng.random() as u64
}

fn random_ble_address() -> Address {
    let rng = Rng::new();
    let mut bytes = [0u8; 6];
    rng.read(&mut bytes);
    Address::random(bytes)
}

#[embassy_executor::task]
async fn ble_task(mut runner: Runner<'static, BleExternalController, DefaultPacketPool>) {
    println!("BLE runner started");
    static HANDLER: ScannerHandler = ScannerHandler;
    runner.run_with_handler(&HANDLER).await.unwrap();
}

#[embassy_executor::task]
async fn ble_scan_task(central: Central<'static, BleExternalController, DefaultPacketPool>) {
    println!("BLE scan task started");
    let mut scanner = Scanner::new(central);
    let mut ble_scan_config = ScanConfig::default();
    ble_scan_config.active = true;
    ble_scan_config.interval = Duration::from_secs(1);
    ble_scan_config.window = Duration::from_secs(1);

    loop {
        let _scan_session = scanner
            .scan(&ble_scan_config)
            .await
            .expect("BLE scan failed");
        Timer::after(Duration::from_secs(30)).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    println!("Init!");
    let peripherals = esp_hal::init(esp_hal::Config::default());

    static TILDAGON: StaticCell<TildagonHardware> = StaticCell::new();
    let tildagon = TILDAGON.init(
        TildagonHardware::new(peripherals)
            .await
            .expect("Tildagon hardware init failed"),
    );
    let mut radio = tildagon.init_radio().expect("Tildagon radio init failed");

    // WiFi Init
    let (wifi_controller, wifi_interfaces) = radio
        .init_wifi(WifiConfig::default())
        .expect("WiFi init failed");

    // TODO only create embassy_net and its runner when we want to transmit: avoids WARN - esp_wifi_internal_tx 12294
    // let config = embassy_net::Config::dhcpv4(Default::default());
    // let seed = random_seed();

    // static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();

    // let (stack, runner) = embassy_net::new(
    //     wifi_interfaces.sta,
    //     config,
    //     RESOURCES.init(StackResources::new()),
    //     seed,
    // );

    // BLE Init
    let connector = radio
        .init_ble_connector(Default::default())
        .expect("BLE connector init failed");

    let controller: BleExternalController = ExternalController::new(connector);

    // Use DefaultPacketPool, CONNS=1, CHANNELS=1, ADV_SETS=1
    static BLE_RESOURCES: StaticCell<HostResources<DefaultPacketPool, 1, 1, 1>> = StaticCell::new();
    let ble_resources = BLE_RESOURCES.init(HostResources::new());

    let address = random_ble_address();

    static BLE_STACK: StaticCell<Stack<'static, BleExternalController, DefaultPacketPool>> =
        StaticCell::new();
    let ble_stack =
        BLE_STACK.init(trouble_host::new(controller, ble_resources).set_random_address(address));

    let Host {
        central,
        runner: ble_runner,
        ..
    } = ble_stack.build();

    // spawner
    //     .spawn(net_task(runner))
    //     .expect("Failed to spawn net_task");
    spawner
        .spawn(wifi_scan_task(wifi_controller))
        .expect("Failed to spawn wifi_scan_task");
    spawner
        .spawn(ble_task(ble_runner))
        .expect("Failed to spawn ble_task");
    spawner
        .spawn(ble_scan_task(central))
        .expect("Failed to spawn ble_scan_task");

    loop {
        // if stack.is_link_up() {
        //     if let Some(config) = stack.config_v4() {
        //         println!("WiFi IP: {}", config.address);
        //     }
        // }
        Timer::after(Duration::from_secs(5)).await;
    }
}
