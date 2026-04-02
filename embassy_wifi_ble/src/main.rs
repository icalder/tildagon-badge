#![no_std]
#![no_main]

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

use alloc::{
    collections::{BTreeSet, VecDeque},
    format,
};
use bt_hci::controller::ExternalController;
use bt_hci::param::LeAdvReportsIter;
use core::cell::RefCell;
use core::str;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::{Mutex, raw::CriticalSectionRawMutex};
use embassy_sync::mutex::Mutex as AsyncMutex;
use embassy_sync::pubsub::{PubSubChannel, Subscriber};
use embassy_time::{Duration, Instant, Timer};
use embedded_hal_async::i2c::I2c as _;

#[derive(Clone, Copy, Debug)]
enum StatusUpdate {
    WifiCount(u32),
    BleCount(u32),
}

type ButtonSubscriber = Subscriber<'static, CriticalSectionRawMutex, ButtonEvent, 16, 4, 1>;
type StatusSubscriber = Subscriber<'static, CriticalSectionRawMutex, StatusUpdate, 4, 2, 1>;
use embedded_graphics::mono_font::{MonoTextStyle, ascii::FONT_8X13};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, PrimitiveStyle, Rectangle, Triangle};
use embedded_graphics::text::{Alignment, Text};
use esp_backtrace as _;
use esp_hal::rng::Rng;
use esp_println::println;
use esp_radio::ble::controller::BleConnector;
use esp_radio::wifi::WifiController;
use esp_radio::wifi::scan::ScanConfig as WifiScanConfig;
use smart_leds::colors::*;
use static_cell::StaticCell;
use tildagon::battery::{Battery, BatteryState};
use tildagon::buttons::{Button, ButtonEvent};
use tildagon::display::{StripeBuffer, TildagonDisplay, render_with_stripes};
use tildagon::draw_stripe;
use tildagon::hardware::TildagonHardware;
use tildagon::i2c::{SharedI2cBus, system_i2c_bus};
use tildagon::leds::{NUM_LEDS, TypedLeds};
use tildagon::pins::Pins;
use trouble_host::advertise::AdStructure;
use trouble_host::central::Central;
use trouble_host::prelude::*;

static WIFI_SCAN_COUNT: AtomicU32 = AtomicU32::new(0);
static WIFI_NETWORK_COUNT: AtomicU32 = AtomicU32::new(0);
static BLE_SEEN_COUNT: AtomicU32 = AtomicU32::new(0);
static BUTTON_A_PRESSED: AtomicBool = AtomicBool::new(false);
static BUTTON_F_PRESSED: AtomicBool = AtomicBool::new(false);
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

static STATUS_CHANNEL: PubSubChannel<CriticalSectionRawMutex, StatusUpdate, 4, 2, 1> =
    PubSubChannel::new();

/// LRU-evicting set of BLE device addresses.
///
/// Tracks up to `CAPACITY` unique addresses. When full and a new address arrives,
/// the least-recently-seen address is evicted. Re-seeing a known address refreshes
/// its position so actively-advertising devices are never evicted while present.
struct LruBleDevices {
    order: VecDeque<[u8; 6]>,
    set: BTreeSet<[u8; 6]>,
}

impl LruBleDevices {
    const CAPACITY: usize = 256;

    const fn new() -> Self {
        Self {
            order: VecDeque::new(),
            set: BTreeSet::new(),
        }
    }

    /// Insert an address. Returns `true` if the address was not already tracked.
    fn insert(&mut self, addr: [u8; 6]) -> bool {
        if self.set.contains(&addr) {
            // Refresh: move to back so it isn't evicted while still active.
            if let Some(pos) = self.order.iter().position(|a| *a == addr) {
                self.order.remove(pos);
                self.order.push_back(addr);
            }
            false
        } else {
            if self.order.len() >= Self::CAPACITY {
                if let Some(evicted) = self.order.pop_front() {
                    self.set.remove(&evicted);
                }
            }
            self.order.push_back(addr);
            self.set.insert(addr);
            true
        }
    }
}

static BLE_SEEN_DEVICES: Mutex<CriticalSectionRawMutex, RefCell<LruBleDevices>> =
    Mutex::new(RefCell::new(LruBleDevices::new()));

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

#[embassy_executor::task]
async fn status_led_task(
    mut leds: TypedLeds<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>,
    mut button_sub: ButtonSubscriber,
    mut status_sub: StatusSubscriber,
) {
    println!("Status LED task started");
    let mut last_wifi_count = u32::MAX;
    let mut last_ble_count = u32::MAX;
    let mut last_button_a = false;
    let mut last_button_f = false;

    let half_green = smart_leds::RGB8 {
        r: GREEN.r / 2,
        g: GREEN.g / 2,
        b: GREEN.b / 2,
    };
    let half_blue = smart_leds::RGB8 {
        r: BLUE.r / 2,
        g: BLUE.g / 2,
        b: BLUE.b / 2,
    };

    loop {
        if SHUTTING_DOWN.load(Ordering::Relaxed) {
            let _ = leds.clear().await;
            break;
        }
        let wifi_count = WIFI_NETWORK_COUNT.load(Ordering::Relaxed);
        let ble_count = BLE_SEEN_COUNT.load(Ordering::Relaxed);
        let button_a = BUTTON_A_PRESSED.load(Ordering::Relaxed);
        let button_f = BUTTON_F_PRESSED.load(Ordering::Relaxed);

        if wifi_count != last_wifi_count
            || ble_count != last_ble_count
            || button_a != last_button_a
            || button_f != last_button_f
        {
            let mut data = [BLACK; NUM_LEDS];

            // WiFi: LEDs 1-6 (inner ring), binary encoded, capped at 63
            let display_wifi = wifi_count.min(63);
            for i in 0..6 {
                if (display_wifi >> i) & 1 == 1 {
                    data[i + 1] = half_green;
                }
            }

            // BLE: LEDs 7-12 (outer ring), binary encoded, capped at 63
            // LSB (bit 0) at LED 12, MSB (bit 5) at LED 7
            let display_ble = ble_count.min(63);
            for i in 0..6 {
                if (display_ble >> i) & 1 == 1 {
                    data[12 - i] = half_blue;
                }
            }

            // Button A: LED 13, bright red when pressed
            if button_a {
                data[13] = RED;
            }

            // Button F: LED 18, magenta when pressed
            if button_f {
                data[18] = MAGENTA;
            }

            if let Err(e) = leds.write(data.iter().cloned()).await {
                println!("LED write error: {:?}", e);
            }
            last_wifi_count = wifi_count;
            last_ble_count = ble_count;
            last_button_a = button_a;
            last_button_f = button_f;
        }

        // Wait for EITHER a button event OR a status count update
        match select(
            button_sub.next_message_pure(),
            status_sub.next_message_pure(),
        )
        .await
        {
            Either::First(event) => match event {
                ButtonEvent::Pressed(Button::A) => BUTTON_A_PRESSED.store(true, Ordering::Relaxed),
                ButtonEvent::Released(Button::A) => {
                    BUTTON_A_PRESSED.store(false, Ordering::Relaxed)
                }
                ButtonEvent::Pressed(Button::F) => BUTTON_F_PRESSED.store(true, Ordering::Relaxed),
                ButtonEvent::Released(Button::F) => {
                    BUTTON_F_PRESSED.store(false, Ordering::Relaxed)
                }
                _ => {}
            },
            Either::Second(update) => match update {
                StatusUpdate::WifiCount(count) => {
                    WIFI_NETWORK_COUNT.store(count, Ordering::Relaxed)
                }
                StatusUpdate::BleCount(count) => BLE_SEEN_COUNT.store(count, Ordering::Relaxed),
            },
        }
    }
}

#[embassy_executor::task]
async fn wifi_scan_task(mut controller: WifiController<'static>) {
    println!("WiFi scan task started");
    loop {
        if SHUTTING_DOWN.load(Ordering::Relaxed) {
            break;
        }

        println!("Scanning for WiFi networks...");
        let config = WifiScanConfig::default();
        match controller.scan_async(&config).await {
            Ok(networks) => {
                WIFI_SCAN_COUNT.fetch_add(1, Ordering::Relaxed);
                let count = networks.len() as u32;
                WIFI_NETWORK_COUNT.store(count, Ordering::Relaxed);
                let _ = STATUS_CHANNEL
                    .publisher()
                    .unwrap()
                    .publish(StatusUpdate::WifiCount(count))
                    .await;
                println!("Found {} networks:", networks.len());
                for network in networks {
                    println!(
                        "SSID: {:?} | RSSI: {:4} | Channel: {:2}",
                        network.ssid,
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
                    let count = BLE_SEEN_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                    let _ = STATUS_CHANNEL
                        .publisher()
                        .unwrap()
                        .publish_immediate(StatusUpdate::BleCount(count));
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
        if SHUTTING_DOWN.load(Ordering::Relaxed) {
            break;
        }
        let _scan_session = scanner
            .scan(&ble_scan_config)
            .await
            .expect("BLE scan failed");
        Timer::after(Duration::from_secs(30)).await;
    }
}

struct UIStrings<'a> {
    battery: &'a str,
    wifi_scans: &'a str,
    wifi_networks: &'a str,
    ble_seen: &'a str,
    fps: &'a str,
}

const TEXT_STYLE: MonoTextStyle<Rgb565> = MonoTextStyle::new(&FONT_8X13, TEXT);

#[embassy_executor::task]
async fn display_task(
    mut display: TildagonDisplay<'static>,
    mut battery: Battery<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>,
) {
    println!("Display task started");
    let mut frame = 0u32;
    let mut battery_state: Option<BatteryState> = None;
    let mut next_battery_refresh = Instant::now();

    static STRIPE_BUFFER: StaticCell<StripeBuffer> = StaticCell::new();
    let stripe_buffer = STRIPE_BUFFER.init(StripeBuffer::new(BG));

    // Cache strings to avoid frequent allocations and heavy stack usage from format!
    let mut battery_str = format!("battery: --.-V");
    let mut wifi_scans_str = format!("WiFi scans: 0");
    let mut wifi_networks_str = format!("Networks: 0");
    let mut ble_seen_str = format!("BLE seen: 0");
    let mut fps_str = format!("--");

    let mut last_wifi_scans = u32::MAX;
    let mut last_wifi_networks = u32::MAX;
    let mut last_ble_seen = u32::MAX;
    let mut last_battery_v = -1.0f32;

    let mut fps_frames = 0u32;
    let mut last_fps_update = Instant::now();

    loop {
        if SHUTTING_DOWN.load(Ordering::Relaxed) {
            break;
        }
        let frame_start = Instant::now();

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

        // Update cached strings only when values change
        let scans = WIFI_SCAN_COUNT.load(Ordering::Relaxed);
        if scans != last_wifi_scans {
            wifi_scans_str = format!("WiFi scans: {scans}");
            last_wifi_scans = scans;
        }

        let networks = WIFI_NETWORK_COUNT.load(Ordering::Relaxed);
        if networks != last_wifi_networks {
            wifi_networks_str = format!("Networks: {networks}");
            last_wifi_networks = networks;
        }

        let ble_seen = BLE_SEEN_COUNT.load(Ordering::Relaxed);
        if ble_seen != last_ble_seen {
            ble_seen_str = format!("BLE seen: {ble_seen}");
            last_ble_seen = ble_seen;
        }

        let battery_v = battery_state.map(|s| s.vbat_volts).unwrap_or(-1.0);
        if (battery_v - last_battery_v).abs() > 0.01 {
            battery_str = if battery_v >= 0.0 {
                // Avoid heavy floating point formatting logic on the stack
                let v = (battery_v * 100.0) as u32;
                format!("battery: {}.{:02}V", v / 100, v % 100)
            } else {
                format!("battery: --.-V")
            };
            last_battery_v = battery_v;
        }

        // FPS calculation
        fps_frames += 1;
        let now = Instant::now();
        if now.duration_since(last_fps_update) >= Duration::from_secs(1) {
            fps_str = format!("{fps_frames}");
            fps_frames = 0;
            last_fps_update = now;
        }

        let current = frame_state(frame);

        let ui_strings = UIStrings {
            battery: &battery_str,
            wifi_scans: &wifi_scans_str,
            wifi_networks: &wifi_networks_str,
            ble_seen: &ble_seen_str,
            fps: &fps_str,
        };

        if let Err(e) =
            render_with_stripes(&mut display, stripe_buffer, BG, |target, stripe_rect| {
                draw_ui(target, current, &ui_strings, stripe_rect)
            })
        {
            println!("Display render error: {:?}", e);
        }

        frame = frame.wrapping_add(1);

        // Target ~30fps, but account for render time
        let elapsed = frame_start.elapsed();
        let target_period = Duration::from_millis(33);
        if elapsed < target_period {
            Timer::after(target_period - elapsed).await;
        } else {
            // Yield if we're running behind
            Timer::after(Duration::from_millis(1)).await;
        }
    }
}

fn draw_ui<D>(
    target: &mut D,
    current: FrameState,
    ui_strings: &UIStrings<'_>,
    stripe_rect: Rectangle,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    draw_stripe!(
        target,
        stripe_rect,
        Text::with_alignment(
            "WiFi BLE LCD",
            Point::new(120, 34),
            TEXT_STYLE,
            Alignment::Center,
        ),
        Text::with_alignment(
            ui_strings.battery,
            Point::new(120, 52),
            TEXT_STYLE,
            Alignment::Center,
        ),
        Text::with_alignment(
            ui_strings.wifi_scans,
            Point::new(120, 78),
            TEXT_STYLE,
            Alignment::Center,
        ),
        Text::with_alignment(
            ui_strings.wifi_networks,
            Point::new(120, 94),
            TEXT_STYLE,
            Alignment::Center,
        ),
        Text::with_alignment(
            ui_strings.ble_seen,
            Point::new(120, 110),
            TEXT_STYLE,
            Alignment::Center,
        ),
        Text::new("WiFi", Point::new(26, 126), TEXT_STYLE),
        Text::new("BLE", Point::new(26, 146), TEXT_STYLE),
        Text::new(ui_strings.fps, Point::new(160, 210), TEXT_STYLE),
        Rectangle::new(WIFI_BAR_TOP_LEFT, Size::new(current.wifi_bar_width, 10))
            .into_styled(PrimitiveStyle::with_fill(WIFI)),
        Rectangle::new(BLE_BAR_TOP_LEFT, Size::new(current.ble_bar_width, 10))
            .into_styled(PrimitiveStyle::with_fill(current.accent)),
        Circle::new(Point::new(current.circle_center_x - 16, 156), 32)
            .into_styled(PrimitiveStyle::with_fill(SHAPE)),
        Triangle::new(
            Point::new(120, current.triangle_tip_y),
            Point::new(94, 224),
            Point::new(146, 224),
        )
        .into_styled(PrimitiveStyle::with_fill(WARN)),
    );

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
    // Note: The original firmware runs at 240MHz:
    // let config = esp_hal::Config::default().with_cpu_clock(esp_hal::clock::CpuClock::_240MHz);
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

    let pins = Pins::new();
    let leds = TypedLeds::new(
        tildagon.rmt,
        tildagon.led_data_pin,
        pins.led,
        system_i2c_bus(shared_i2c),
    )
    .await
    .expect("Typed LED init failed");

    // Start the background button service
    let button_manager = TildagonHardware::init_button_manager(&spawner, shared_i2c);

    spawner.spawn(status_led_task(
        leds,
        button_manager.subscribe(),
        STATUS_CHANNEL.subscriber().unwrap(),
    ).unwrap());

    let mut battery = Battery::new(system_i2c_bus(shared_i2c));

    if let Some(display) = display {
        spawner.spawn(display_task(
            display,
            Battery::new(system_i2c_bus(shared_i2c)),
        ).unwrap());
    }

    // WiFi Init
    let (wifi_controller, _wifi_interfaces) = radio
        .init_wifi(Default::default())
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

    // spawner.spawn(net_task(runner).unwrap());
    spawner.spawn(wifi_scan_task(wifi_controller).unwrap());
    spawner.spawn(ble_task(ble_runner).unwrap());
    spawner.spawn(ble_scan_task(central).unwrap());

    println!("[BUTTON] Waiting for button events...");
    let mut sub = button_manager.subscribe();

    loop {
        match sub.next_message_pure().await {
            ButtonEvent::Pressed(Button::A) => {
                BUTTON_A_PRESSED.store(true, Ordering::Relaxed);
            }
            ButtonEvent::Released(Button::A) => {
                BUTTON_A_PRESSED.store(false, Ordering::Relaxed);
            }
            ButtonEvent::Pressed(Button::F) => {
                println!("[BUTTON] Hold F for 2s to power off");
                BUTTON_F_PRESSED.store(true, Ordering::Relaxed);
                match embassy_time::with_timeout(Duration::from_secs(2), async {
                    loop {
                        match sub.next_message_pure().await {
                            ButtonEvent::Released(Button::F) => {
                                BUTTON_F_PRESSED.store(false, Ordering::Relaxed);
                                break;
                            }
                            event => println!("[BUTTON] Event: {:?}", event),
                        }
                    }
                })
                .await
                {
                    Ok(()) => {
                        println!("[BUTTON] Power-off cancelled");
                    }
                    Err(_) => {
                        println!("[BUTTON] Long press detected, release to power off");
                        BUTTON_F_PRESSED.store(false, Ordering::Relaxed);
                        let _ = STATUS_CHANNEL.publisher().unwrap().publish_immediate(
                            StatusUpdate::BleCount(BLE_SEEN_COUNT.load(Ordering::Relaxed)),
                        );

                        // Wait for button release before disconnecting battery.
                        // Holding the button (QON) prevents entering ship mode.
                        loop {
                            if let ButtonEvent::Released(Button::F) = sub.next_message_pure().await
                            {
                                break;
                            }
                        }

                        println!("[BUTTON] Released! Powering off...");
                        SHUTTING_DOWN.store(true, Ordering::Relaxed);
                        // Brief delay for tasks to exit and I2C to settle
                        Timer::after(Duration::from_millis(100)).await;

                        // Turn off VBUS switch (Expander 0x5a, Reg 0x02, Bit 4 = 0) and LED power (Bit 2 = 0)
                        let mut bus = system_i2c_bus(shared_i2c);
                        let _ = bus.write(0x5au8, &[0x02, 0x00]).await;

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
            event => println!("[BUTTON] Event: {:?}", event),
        }
    }
}
