#![no_std]
#![no_main]

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

use alloc::{collections::BTreeSet, format};
use bt_hci::controller::ExternalController;
use bt_hci::param::LeAdvReportsIter;
use core::cell::RefCell;
use core::str;
use core::sync::atomic::{AtomicU32, Ordering};
use embassy_executor::Spawner;
use embassy_net::Runner as NetRunner;
use embassy_sync::blocking_mutex::{Mutex, raw::CriticalSectionRawMutex};
use embassy_sync::mutex::Mutex as AsyncMutex;
use embassy_time::{Duration, Instant, Timer};
use embedded_graphics::mono_font::{MonoTextStyle, MonoTextStyleBuilder, ascii::FONT_8X13};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, PrimitiveStyle, Rectangle, Triangle};
use embedded_graphics::text::{Alignment, Text};
use esp_backtrace as _;
use esp_hal::rng::Rng;
use esp_println::println;
use esp_radio::ble::controller::BleConnector;
use esp_radio::wifi::{
    ClientConfig, Config as WifiConfig, ScanConfig as WifiScanConfig, WifiController, WifiDevice,
};
use static_cell::StaticCell;
use tildagon::battery::{Battery, BatteryState};
use tildagon::buttons::{Button, ButtonEvent, TypedButtons};
use tildagon::display::TildagonDisplay;
use tildagon::hardware::TildagonHardware;
use tildagon::i2c::{SharedI2cBus, system_i2c_bus, top_i2c_bus};
use trouble_host::advertise::AdStructure;
use trouble_host::central::Central;
use trouble_host::prelude::*;

static WIFI_SCAN_COUNT: AtomicU32 = AtomicU32::new(0);
static WIFI_NETWORK_COUNT: AtomicU32 = AtomicU32::new(0);
static BLE_SEEN_COUNT: AtomicU32 = AtomicU32::new(0);
static BLE_SEEN_DEVICES: Mutex<CriticalSectionRawMutex, RefCell<BTreeSet<[u8; 6]>>> =
    Mutex::new(RefCell::new(BTreeSet::new()));

const BG: Rgb565 = Rgb565::new(0, 0, 0);
const TEXT: Rgb565 = Rgb565::new(24, 48, 24);
const WIFI: Rgb565 = Rgb565::new(0, 63, 31);
const SHAPE: Rgb565 = Rgb565::new(31, 0, 20);
const WARN: Rgb565 = Rgb565::new(31, 63, 0);

const BAR_FILL_LEFT: i32 = 68;
const BAR_MAX_WIDTH: u32 = 124;
const WIFI_BAR_TOP_LEFT: Point = Point::new(BAR_FILL_LEFT, 116);
const BLE_BAR_TOP_LEFT: Point = Point::new(BAR_FILL_LEFT, 136);

#[derive(Clone, Copy)]
struct FrameState {
    circle_center_x: i32,
    triangle_tip_y: i32,
    wifi_bar_width: u32,
    ble_bar_width: u32,
    accent: Rgb565,
}

#[embassy_executor::task]
async fn net_task(mut runner: NetRunner<'static, WifiDevice<'static>>) {
    runner.run().await;
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
                WIFI_SCAN_COUNT.fetch_add(1, Ordering::Relaxed);
                WIFI_NETWORK_COUNT.store(networks.len() as u32, Ordering::Relaxed);
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
                let mut addr = [0u8; 6];
                addr.copy_from_slice(report.addr.raw());
                let was_new = BLE_SEEN_DEVICES.lock(|devices| devices.borrow_mut().insert(addr));
                if was_new {
                    BLE_SEEN_COUNT.fetch_add(1, Ordering::Relaxed);
                    if let Some(name) = advertised_name(report.data) {
                        println!("BLE: Discovered {name}, RSSI: {}", report.rssi);
                    } else {
                        println!("BLE: Discovered {:?}, RSSI: {}", report.addr, report.rssi);
                    }
                }
            }
        }
    }
}

type BleExternalController = ExternalController<BleConnector<'static>, 1>;

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

#[embassy_executor::task]
async fn display_task(
    mut display: TildagonDisplay<'static>,
    mut battery: Battery<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>,
) {
    println!("Display task started");
    let mut frame = 0u32;
    let mut previous = None;
    let mut battery_state: Option<BatteryState> = None;
    let mut next_battery_refresh = Instant::now();

    if let Err(e) = render_display_background(&mut display) {
        println!("Display background render error: {:?}", e);
    }

    loop {
        if Instant::now() >= next_battery_refresh {
            let battery_refresh_interval = match battery.read().await {
                Ok(state) if state.vbat_volts > 0.0 => {
                    battery_state = Some(state);
                    Duration::from_secs(30)
                }
                Ok(_) => Duration::from_secs(1),
                Err(e) => {
                    println!("Battery read error: {:?}", e);
                    Duration::from_secs(1)
                }
            };

            next_battery_refresh = Instant::now() + battery_refresh_interval;
        }

        let current = frame_state(frame);
        if let Err(e) = render_display_frame(&mut display, previous, current, battery_state) {
            println!("Display render error: {:?}", e);
        }

        previous = Some(current);
        frame = frame.wrapping_add(1);
        Timer::after(Duration::from_millis(120)).await;
    }
}

type DisplayDrawError = <TildagonDisplay<'static> as DrawTarget>::Error;

fn render_display_background(
    display: &mut TildagonDisplay<'static>,
) -> Result<(), DisplayDrawError> {
    let title_style = MonoTextStyle::new(&FONT_8X13, TEXT);

    display.clear(BG)?;
    Text::with_alignment(
        "WiFi BLE LCD",
        Point::new(120, 34),
        title_style,
        Alignment::Center,
    )
    .draw(display)?;
    Text::with_alignment(
        "battery: --.-V",
        Point::new(120, 52),
        title_style,
        Alignment::Center,
    )
    .draw(display)?;
    Text::new("WiFi", Point::new(26, 126), title_style).draw(display)?;
    Text::new("BLE", Point::new(26, 146), title_style).draw(display)?;
    Ok(())
}

fn frame_state(frame: u32) -> FrameState {
    FrameState {
        circle_center_x: ping_pong(frame, 42, 198, 20),
        triangle_tip_y: ping_pong(frame.wrapping_add(10), 154, 188, 24),
        wifi_bar_width: (WIFI_NETWORK_COUNT.load(Ordering::Relaxed).saturating_mul(7))
            .clamp(10, BAR_MAX_WIDTH),
        ble_bar_width: (BLE_SEEN_COUNT.load(Ordering::Relaxed).saturating_mul(7))
            .clamp(10, BAR_MAX_WIDTH),
        accent: match frame % 3 {
            0 => Rgb565::new(31, 0, 0),
            1 => Rgb565::new(0, 63, 0),
            _ => Rgb565::new(0, 0, 31),
        },
    }
}

fn render_display_frame(
    display: &mut TildagonDisplay<'static>,
    previous: Option<FrameState>,
    current: FrameState,
    battery_state: Option<tildagon::battery::BatteryState>,
) -> Result<(), DisplayDrawError> {
    let text_style = MonoTextStyleBuilder::new()
        .font(&FONT_8X13)
        .text_color(TEXT)
        .background_color(BG)
        .build();
    let wifi_scans = WIFI_SCAN_COUNT.load(Ordering::Relaxed);
    let wifi_networks = WIFI_NETWORK_COUNT.load(Ordering::Relaxed);
    let ble_seen = BLE_SEEN_COUNT.load(Ordering::Relaxed);

    if let Some(previous) = previous {
        draw_shapes(display, previous, BG, BG)?;
    }

    Text::with_alignment(
        &format!("WiFi scans: {wifi_scans}"),
        Point::new(120, 78),
        text_style,
        Alignment::Center,
    )
    .draw(display)?;
    Text::with_alignment(
        &format!("Networks: {wifi_networks}"),
        Point::new(120, 94),
        text_style,
        Alignment::Center,
    )
    .draw(display)?;
    Text::with_alignment(
        &format!("BLE seen: {ble_seen}"),
        Point::new(120, 110),
        text_style,
        Alignment::Center,
    )
    .draw(display)?;
    let battery_text = match battery_state {
        Some(state) => format!("battery: {:.2}V", state.vbat_volts),
        None => format!("battery: --.-V"),
    };
    Text::with_alignment(
        battery_text.as_str(),
        Point::new(120, 52),
        text_style,
        Alignment::Center,
    )
    .draw(display)?;

    Rectangle::new(WIFI_BAR_TOP_LEFT, Size::new(BAR_MAX_WIDTH, 10))
        .into_styled(PrimitiveStyle::with_fill(BG))
        .draw(display)?;
    Rectangle::new(BLE_BAR_TOP_LEFT, Size::new(BAR_MAX_WIDTH, 10))
        .into_styled(PrimitiveStyle::with_fill(BG))
        .draw(display)?;
    Rectangle::new(WIFI_BAR_TOP_LEFT, Size::new(current.wifi_bar_width, 10))
        .into_styled(PrimitiveStyle::with_fill(WIFI))
        .draw(display)?;
    Rectangle::new(BLE_BAR_TOP_LEFT, Size::new(current.ble_bar_width, 10))
        .into_styled(PrimitiveStyle::with_fill(current.accent))
        .draw(display)?;
    draw_shapes(display, current, SHAPE, WARN)?;

    Ok(())
}

fn draw_shapes(
    display: &mut TildagonDisplay<'static>,
    state: FrameState,
    circle_color: Rgb565,
    triangle_color: Rgb565,
) -> Result<(), DisplayDrawError> {
    Circle::new(Point::new(state.circle_center_x - 16, 156), 32)
        .into_styled(PrimitiveStyle::with_fill(circle_color))
        .draw(display)?;
    Triangle::new(
        Point::new(120, state.triangle_tip_y),
        Point::new(94, 224),
        Point::new(146, 224),
    )
    .into_styled(PrimitiveStyle::with_fill(triangle_color))
    .draw(display)?;

    Ok(())
}

fn ping_pong(frame: u32, min: i32, max: i32, period: u32) -> i32 {
    let span = max - min;
    let cycle = period.saturating_mul(2).max(2);
    let step = (frame % cycle) as i32;
    let period = period.max(1) as i32;
    let offset = if step < period {
        step * span / period
    } else {
        (period * 2 - step) * span / period
    };

    min + offset
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    println!("Init!");
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let mut tildagon = TildagonHardware::new(peripherals)
        .await
        .expect("Tildagon hardware init failed");

    static DISPLAY_BUFFER: StaticCell<[u8; 4096]> = StaticCell::new();
    let display_buffer = DISPLAY_BUFFER.init([0u8; 4096]);
    let display = match tildagon.init_display(display_buffer) {
        Ok(display) => Some(display),
        Err(e) => {
            println!("Display init failed: {:?}", e);
            None
        }
    };

    let mut radio = tildagon.init_radio().expect("Tildagon radio init failed");

    static SHARED_I2C: StaticCell<
        SharedI2cBus<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>,
    > = StaticCell::new();
    let shared_i2c = SHARED_I2C.init(AsyncMutex::new(tildagon.i2c.into_async()));
    let mut button_int = tildagon.button_int;
    let mut battery = Battery::new(system_i2c_bus(shared_i2c));
    let mut buttons = TypedButtons::new(system_i2c_bus(shared_i2c), top_i2c_bus(shared_i2c));

    if let Some(display) = display {
        spawner
            .spawn(display_task(
                display,
                Battery::new(system_i2c_bus(shared_i2c)),
            ))
            .expect("Failed to spawn display_task");
    }

    // WiFi Init
    let (wifi_controller, _wifi_interfaces) = radio
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

    println!("[BUTTON] Waiting for button events...");
    loop {
        match buttons.wait_for_event(&mut button_int).await {
            Ok(Some(ButtonEvent::Pressed(Button::F))) => {
                println!("[BUTTON] Hold F for 2s to power off");
                match embassy_time::with_timeout(Duration::from_secs(2), async {
                    loop {
                        match buttons.wait_for_event(&mut button_int).await {
                            Ok(Some(ButtonEvent::Released(Button::F))) => break,
                            Ok(Some(event)) => println!("[BUTTON] Event: {:?}", event),
                            Ok(None) => {}
                            Err(e) => {
                                println!("[BUTTON] Error reading buttons: {:?}", e);
                                break;
                            }
                        }
                    }
                })
                .await
                {
                    Ok(()) => {
                        println!("[BUTTON] Power-off cancelled");
                    }
                    Err(_) => {
                        println!("[BUTTON] Long press detected, powering off");
                        match battery.power_off().await {
                            Ok(()) => {
                                println!("[BUTTON] BATFET disabled; waiting for power loss");
                                loop {
                                    Timer::after(Duration::from_secs(1)).await;
                                }
                            }
                            Err(e) => {
                                println!("[BUTTON] Failed to request power-off: {:?}", e);
                            }
                        }
                    }
                }
            }
            Ok(Some(event)) => println!("[BUTTON] Event: {:?}", event),
            Ok(None) => {}
            Err(e) => println!("[BUTTON] Error reading buttons: {:?}", e),
        }
    }
}
