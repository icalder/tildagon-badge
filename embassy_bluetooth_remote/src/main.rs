#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use core::fmt::Write as _;

use bt_hci::controller::ExternalController;
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::{PubSubChannel, Publisher, Subscriber};
use embassy_time::{Duration, Timer};
use embassy_sync_08::mutex::Mutex as AsyncMutex;
use esp_backtrace as _;
use esp_hal::rng::Rng;
use esp_println::println;
use esp_radio::ble::controller::BleConnector;
use smart_leds::{RGB8, colors::*};
use static_cell::StaticCell;
use tildagon::battery::Battery;
use tildagon::buttons::{Button, ButtonEvent};
use tildagon::hardware::TildagonHardware;
use tildagon::i2c::system_i2c_bus;
use tildagon::leds::{NUM_LEDS, TypedLeds};
use tildagon::pins::Pins;
use trouble_host::prelude::*;

type BleExternalController = ExternalController<BleConnector<'static>, 1>;
type BadgeI2c = esp_hal::i2c::master::I2c<'static, esp_hal::Async>;
type BadgeLeds = TypedLeds<BadgeI2c>;
type BleStack = Stack<'static, BleExternalController, DefaultPacketPool>;
type BleRunner = Runner<'static, BleExternalController, DefaultPacketPool>;
type BlePeripheral = Peripheral<'static, BleExternalController, DefaultPacketPool>;
type ButtonSubscriber = embassy_sync_08::pubsub::Subscriber<'static, embassy_sync_08::blocking_mutex::raw::CriticalSectionRawMutex, ButtonEvent, 16, 4, 1>;
type LedSubscriber = Subscriber<'static, CriticalSectionRawMutex, LedCommand, 8, 1, 2>;
type LedPublisher = Publisher<'static, CriticalSectionRawMutex, LedCommand, 8, 1, 2>;

const LED_COMMAND_LEN: usize = 8;
const BLE_NAME_CAPACITY: usize = 13;
const MAX_BLINK_REPEATS: u8 = 10;
const MAX_CHASE_ROUNDS: u8 = 8;
const MAX_PATTERN_DELAY_UNITS: u8 = 100;
const BLE_CONN_INTERVAL_MIN_US: u64 = 10_000;
const BLE_CONN_INTERVAL_MAX_US: u64 = 10_000;
const BLE_CONN_MAX_LATENCY: u16 = 0;
const BLE_CONN_SUPERVISION_TIMEOUT_MS: u64 = 5_000;
const LED_SERVICE_UUID_ADV: [u8; 16] = [
    0xf0, 0xde, 0xbc, 0x9a, 0x78, 0x56, 0x34, 0x12, 0x78, 0x56, 0x34, 0x12, 0x78, 0x56, 0x34,
    0x12,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LedCommand {
    Clear,
    SetLed { index: u8, color: RGB8 },
    Fill { color: RGB8 },
    Blink {
        color: RGB8,
        repeats: u8,
        on_time: Duration,
        off_time: Duration,
    },
    Chase {
        color: RGB8,
        rounds: u8,
        step_time: Duration,
    },
}

#[gatt_server]
struct BadgeServer {
    led_service: LedService,
}

#[gatt_service(uuid = "12345678-1234-5678-1234-56789abcdef0")]
struct LedService {
    #[characteristic(
        uuid = "12345678-1234-5678-1234-56789abcdef1",
        read,
        write,
        value = [0; LED_COMMAND_LEN]
    )]
    command: [u8; LED_COMMAND_LEN],
}

fn random_ble_address() -> Address {
    let rng = Rng::new();
    let mut bytes = [0u8; 6];
    rng.read(&mut bytes);
    // Force a valid static random address.
    bytes[5] = (bytes[5] & 0x3f) | 0xc0;
    Address::random(bytes)
}

fn ble_name_from_address(address: &Address) -> HeaplessString<BLE_NAME_CAPACITY> {
    let bytes = address.addr.into_inner();
    let mut name = HeaplessString::new();
    write!(&mut name, "Tildagon-{:02X}{:02X}", bytes[1], bytes[0]).unwrap();
    name
}

fn color(r: u8, g: u8, b: u8) -> RGB8 {
    RGB8 { r, g, b }
}

fn solid_frame(color: RGB8) -> [RGB8; NUM_LEDS] {
    [color; NUM_LEDS]
}

fn single_led_frame(index: usize, color: RGB8) -> [RGB8; NUM_LEDS] {
    let mut frame = [RGB8::default(); NUM_LEDS];
    frame[index] = color;
    frame
}

fn require_zeroed(bytes: &[u8]) -> Result<(), AttErrorCode> {
    if bytes.iter().all(|byte| *byte == 0) {
        Ok(())
    } else {
        Err(AttErrorCode::INVALID_PDU)
    }
}

fn parse_led_command(data: &[u8]) -> Result<LedCommand, AttErrorCode> {
    if data.len() != LED_COMMAND_LEN {
        return Err(AttErrorCode::INVALID_ATTRIBUTE_VALUE_LENGTH);
    }

    match data[0] {
        0x00 => {
            require_zeroed(&data[1..])?;
            Ok(LedCommand::Clear)
        }
        0x01 => {
            if data[1] as usize >= NUM_LEDS {
                return Err(AttErrorCode::INVALID_PDU);
            }
            require_zeroed(&data[5..])?;
            Ok(LedCommand::SetLed {
                index: data[1],
                color: color(data[2], data[3], data[4]),
            })
        }
        0x02 => {
            require_zeroed(&data[4..])?;
            Ok(LedCommand::Fill {
                color: color(data[1], data[2], data[3]),
            })
        }
        0x03 => {
            if data[4] == 0
                || data[4] > MAX_BLINK_REPEATS
                || data[5] == 0
                || data[6] == 0
                || data[5] > MAX_PATTERN_DELAY_UNITS
                || data[6] > MAX_PATTERN_DELAY_UNITS
            {
                return Err(AttErrorCode::INVALID_PDU);
            }
            if data[7] != 0 {
                return Err(AttErrorCode::INVALID_PDU);
            }
            Ok(LedCommand::Blink {
                color: color(data[1], data[2], data[3]),
                repeats: data[4],
                on_time: Duration::from_millis((data[5] as u64) * 100),
                off_time: Duration::from_millis((data[6] as u64) * 100),
            })
        }
        0x04 => {
            if data[4] == 0
                || data[4] > MAX_CHASE_ROUNDS
                || data[5] == 0
                || data[5] > MAX_PATTERN_DELAY_UNITS
            {
                return Err(AttErrorCode::INVALID_PDU);
            }
            require_zeroed(&data[6..])?;
            Ok(LedCommand::Chase {
                color: color(data[1], data[2], data[3]),
                rounds: data[4],
                step_time: Duration::from_millis((data[5] as u64) * 10),
            })
        }
        _ => Err(AttErrorCode::INVALID_PDU),
    }
}

fn encode_advertisement(name: &str) -> Result<([u8; 31], usize, [u8; 31], usize), Error> {
    let mut adv_data = [0; 31];
    let adv_len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::ServiceUuids128(&[LED_SERVICE_UUID_ADV]),
        ],
        &mut adv_data[..],
    )?;

    let mut scan_data = [0; 31];
    let scan_len = AdStructure::encode_slice(
        &[AdStructure::CompleteLocalName(name.as_bytes())],
        &mut scan_data[..],
    )?;

    Ok((adv_data, adv_len, scan_data, scan_len))
}

fn preferred_connection_params() -> RequestedConnParams {
    RequestedConnParams {
        min_connection_interval: Duration::from_micros(BLE_CONN_INTERVAL_MIN_US),
        max_connection_interval: Duration::from_micros(BLE_CONN_INTERVAL_MAX_US),
        max_latency: BLE_CONN_MAX_LATENCY,
        min_event_length: Duration::from_millis(0),
        max_event_length: Duration::from_millis(0),
        supervision_timeout: Duration::from_millis(BLE_CONN_SUPERVISION_TIMEOUT_MS),
    }
}

async fn request_preferred_connection_params(
    stack: &'static BleStack,
    conn: &GattConnection<'static, 'static, DefaultPacketPool>,
) {
    let params = preferred_connection_params();
    println!(
        "[BLE] requesting conn params: interval={:?}..{:?} latency={} timeout={:?}",
        params.min_connection_interval,
        params.max_connection_interval,
        params.max_latency,
        params.supervision_timeout
    );

    match conn.raw().update_connection_params(stack, &params).await {
        Ok(()) => println!("[BLE] connection parameter update requested"),
        Err(e) => println!("[BLE] connection parameter update request failed: {:?}", e),
    }
}

#[embassy_executor::task]
async fn ble_runner_task(mut runner: BleRunner) {
    loop {
        if let Err(e) = runner.run().await {
            println!("[BLE] runner error: {:?}", e);
            Timer::after(Duration::from_millis(250)).await;
        }
    }
}

#[embassy_executor::task]
async fn led_task(mut sub: LedSubscriber, mut leds: BadgeLeds) {
    if let Err(e) = leds.clear().await {
        println!("[LED] startup clear error: {:?}", e);
    }

    loop {
        let command = sub.next_message_pure().await;
        if let Err(e) = apply_led_command(&mut leds, command).await {
            println!("[LED] command error: {:?}", e);
        }
    }
}

async fn apply_led_command(leds: &mut BadgeLeds, command: LedCommand) -> Result<(), tildagon::Error> {
    match command {
        LedCommand::Clear => leds.clear().await,
        LedCommand::SetLed { index, color } => {
            let frame = single_led_frame(index as usize, color);
            leds.write(frame.iter().cloned()).await
        }
        LedCommand::Fill { color } => {
            let frame = solid_frame(color);
            leds.write(frame.iter().cloned()).await
        }
        LedCommand::Blink {
            color,
            repeats,
            on_time,
            off_time,
        } => {
            let frame = solid_frame(color);
            for _ in 0..repeats {
                leds.write(frame.iter().cloned()).await?;
                Timer::after(on_time).await;
                leds.clear().await?;
                Timer::after(off_time).await;
            }
            Ok(())
        }
        LedCommand::Chase {
            color,
            rounds,
            step_time,
        } => {
            for _ in 0..rounds {
                for index in 0..NUM_LEDS {
                    let frame = single_led_frame(index, color);
                    leds.write(frame.iter().cloned()).await?;
                    Timer::after(step_time).await;
                }
            }
            leds.clear().await
        }
    }
}

#[embassy_executor::task]
async fn button_handler_task(
    mut sub: ButtonSubscriber,
    led_pub: LedPublisher,
    mut battery: Battery<BadgeI2c>,
) {
    loop {
        let event = sub.next_message_pure().await;
        match event {
            ButtonEvent::Pressed(Button::F) => {
                println!("[BUTTON] Hold F for 2s to power off");
                match embassy_time::with_timeout(Duration::from_secs(2), async {
                    loop {
                        let event = sub.next_message_pure().await;
                        if event == ButtonEvent::Released(Button::F) {
                            break;
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
                        let red = RED;
                        let dim = RGB8 {
                            r: red.r / 2,
                            g: red.g / 2,
                            b: red.b / 2,
                        };
                        led_pub.publish_immediate(LedCommand::Fill { color: dim });
                        Timer::after(Duration::from_secs(1)).await;
                        led_pub.publish_immediate(LedCommand::Clear);

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
            ButtonEvent::Pressed(Button::C) => {
                println!("[BUTTON] Button {:?} pressed", Button::C);
                led_pub.publish_immediate(LedCommand::SetLed {
                    index: 5,
                    color: GREEN,
                });
                Timer::after(Duration::from_secs(1)).await;
                led_pub.publish_immediate(LedCommand::Clear);
            }
            other => println!("[BUTTON] Event: {:?}", other),
        }
    }
}

async fn gatt_connection_task(
    stack: &'static BleStack,
    server: &'static BadgeServer<'static>,
    conn: &GattConnection<'static, 'static, DefaultPacketPool>,
    led_pub: &LedPublisher,
) -> Result<(), Error> {
    let command = server.led_service.command;

    loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => {
                println!("[BLE] disconnected: {:?}", reason);
                break;
            }
            GattConnectionEvent::Gatt { event } => {
                let reply = match &event {
                    GattEvent::Read(read) if read.handle() == command.handle => {
                        match server.get(&command) {
                            Ok(value) => println!("[GATT] read command: {:?}", value),
                            Err(e) => println!("[GATT] read state error: {:?}", e),
                        }
                        event.accept()
                    }
                    GattEvent::Write(write) if write.handle() == command.handle => {
                        println!("[GATT] write command: {:?}", write.data());
                        match parse_led_command(write.data()) {
                            Ok(parsed) => {
                                led_pub.publish_immediate(parsed);
                                event.accept()
                            }
                            Err(att_error) => {
                                println!("[GATT] rejected write: {:?}", att_error);
                                event.reject(att_error)
                            }
                        }
                    }
                    _ => event.accept(),
                };

                match reply {
                    Ok(reply) => reply.send().await,
                    Err(e) => println!("[GATT] reply error: {:?}", e),
                }
            }
            GattConnectionEvent::PhyUpdated { tx_phy, rx_phy } => {
                println!("[BLE] phy updated: tx={:?} rx={:?}", tx_phy, rx_phy);
            }
            GattConnectionEvent::ConnectionParamsUpdated {
                conn_interval,
                peripheral_latency,
                supervision_timeout,
            } => {
                println!(
                    "[BLE] conn params: interval={:?} latency={} timeout={:?}",
                    conn_interval, peripheral_latency, supervision_timeout
                );
            }
            GattConnectionEvent::RequestConnectionParams(req) => {
                let p = req.params();
                println!(
                    "[BLE] remote requested conn params: interval={:?}..{:?} latency={} timeout={:?}",
                    p.min_connection_interval,
                    p.max_connection_interval,
                    p.max_latency,
                    p.supervision_timeout
                );
                if let Err(e) = req.accept(None, stack).await {
                    println!("[BLE] failed to accept connection params: {:?}", e);
                }
            }
            GattConnectionEvent::DataLengthUpdated { .. } => {}
        }
    }

    Ok(())
}

#[embassy_executor::task]
async fn ble_peripheral_task(
    mut peripheral: BlePeripheral,
    stack: &'static BleStack,
    server: &'static BadgeServer<'static>,
    led_pub: LedPublisher,
    name: &'static str,
) {
    loop {
        let (adv_data, adv_len, scan_data, scan_len) = match encode_advertisement(name) {
            Ok(encoded) => encoded,
            Err(e) => {
                println!("[BLE] advertisement encode error: {:?}", e);
                Timer::after(Duration::from_millis(250)).await;
                continue;
            }
        };

        let advertiser = match peripheral
            .advertise(
                &Default::default(),
                Advertisement::ConnectableScannableUndirected {
                    adv_data: &adv_data[..adv_len],
                    scan_data: &scan_data[..scan_len],
                },
            )
            .await
        {
            Ok(advertiser) => advertiser,
            Err(e) => {
                println!("[BLE] advertise/connect error: {:?}", e);
                Timer::after(Duration::from_millis(250)).await;
                continue;
            }
        };

        println!("[BLE] advertising as {}", name);

        let conn = match advertiser.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                println!("[BLE] accept error: {:?}", e);
                Timer::after(Duration::from_millis(250)).await;
                continue;
            }
        };

        let conn = match conn.with_attribute_server(server) {
            Ok(conn) => conn,
            Err(e) => {
                println!("[BLE] attribute server attach error: {:?}", e);
                Timer::after(Duration::from_millis(250)).await;
                continue;
            }
        };

        println!("[BLE] connection established");
        request_preferred_connection_params(stack, &conn).await;

        if let Err(e) = gatt_connection_task(stack, server, &conn, &led_pub).await {
            println!("[GATT] connection task error: {:?}", e);
        }
        led_pub.publish_immediate(LedCommand::Clear);
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    println!("Init!");
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let mut tildagon = TildagonHardware::new(peripherals)
        .await
        .expect("Tildagon hardware init failed");

    let mut radio = tildagon.init_radio().expect("Tildagon radio init failed");

    static SHARED_I2C: StaticCell<tildagon::i2c::SharedI2cBus<BadgeI2c>> = StaticCell::new();
    let shared_i2c = SHARED_I2C.init(AsyncMutex::new(tildagon.i2c.into_async()));
    
    // Start the background button service
    let button_manager = TildagonHardware::init_button_manager(&spawner, shared_i2c);
    
    let pins = Pins::new();

    let leds = TypedLeds::new(
        tildagon.rmt,
        tildagon.led_data_pin,
        pins.led,
        system_i2c_bus(shared_i2c),
    )
    .await
    .expect("Typed LED init failed");

    let connector = radio
        .init_ble_connector(Default::default())
        .expect("BLE connector init failed");

    let controller: BleExternalController = ExternalController::new(connector);

    static BLE_RESOURCES: StaticCell<HostResources<DefaultPacketPool, 1, 2, 1>> = StaticCell::new();
    let ble_resources = BLE_RESOURCES.init(HostResources::new());

    let address = random_ble_address();
    println!("[BLE] address: {}", address);

    static BADGE_NAME: StaticCell<HeaplessString<BLE_NAME_CAPACITY>> = StaticCell::new();
    let badge_name = BADGE_NAME.init(ble_name_from_address(&address));

    static BLE_STACK: StaticCell<BleStack> = StaticCell::new();
    let ble_stack =
        BLE_STACK.init(trouble_host::new(controller, ble_resources).set_random_address(address));

    let Host {
        peripheral,
        runner: ble_runner,
        ..
    } = ble_stack.build();

    static SERVER: StaticCell<BadgeServer<'static>> = StaticCell::new();
    let server = SERVER.init(
        BadgeServer::new_with_config(GapConfig::Peripheral(PeripheralConfig {
            name: badge_name.as_str(),
            appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
        }))
        .expect("BLE server init failed"),
    );

    static LED_CHANNEL: StaticCell<PubSubChannel<CriticalSectionRawMutex, LedCommand, 8, 1, 2>> =
        StaticCell::new();
    let led_channel = LED_CHANNEL.init(PubSubChannel::new());

    spawner.spawn(ble_runner_task(ble_runner).unwrap());
    spawner.spawn(led_task(led_channel.subscriber().unwrap(), leds).unwrap());
    spawner.spawn(button_handler_task(
        button_manager.subscribe(),
        led_channel.publisher().unwrap(),
        Battery::new(system_i2c_bus(shared_i2c)),
    ).unwrap());
    spawner.spawn(ble_peripheral_task(
        peripheral,
        ble_stack,
        server,
        led_channel.publisher().unwrap(),
        badge_name.as_str(),
    ).unwrap());

    println!("[BUTTON] All tasks started, background polling active.");
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
