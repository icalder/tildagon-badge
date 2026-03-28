#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use bt_hci::controller::ExternalController;
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex as AsyncMutex;
use embassy_sync::pubsub::{PubSubChannel, Subscriber};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::rng::Rng;
use esp_println::println;
use esp_radio::ble::controller::BleConnector;
use smart_leds::colors::*;
use static_cell::StaticCell;
use tildagon::battery::Battery;
use tildagon::buttons::{Button, ButtonEvent, TypedButtons};
use tildagon::hardware::TildagonHardware;
use tildagon::i2c::{system_i2c_bus, top_i2c_bus};
use tildagon::leds::{NUM_LEDS, TypedLeds};
use tildagon::pins::Pins;
use trouble_host::prelude::*;

type BleExternalController = ExternalController<BleConnector<'static>, 1>;

fn random_ble_address() -> Address {
    let rng = Rng::new();
    let mut bytes = [0u8; 6];
    rng.read(&mut bytes);
    Address::random(bytes)
}

#[embassy_executor::task]
async fn button_handler_task(
    mut sub: Subscriber<'static, CriticalSectionRawMutex, ButtonEvent, 8, 4, 1>,
    mut leds: TypedLeds<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>,
    mut battery: Battery<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>,
) {
    let _ = leds.clear().await;
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
                        let color = RED;
                        let dim = smart_leds::RGB8 {
                            r: color.r / 2,
                            g: color.g / 2,
                            b: color.b / 2,
                        };
                        let data = [dim; NUM_LEDS];
                        if let Err(e) = leds.write(data.iter().cloned()).await {
                            println!("[LED] write error: {:?}", e);
                        }
                        Timer::after(Duration::from_secs(1)).await;
                        if let Err(e) = leds.clear().await {
                            println!("[LED] clear error: {:?}", e);
                        }

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
                let mut data = [smart_leds::RGB8::default(); NUM_LEDS];
                data[5] = GREEN;
                if let Err(e) = leds.write(data.iter().cloned()).await {
                    println!("[LED] write error: {:?}", e);
                }
                Timer::after(Duration::from_secs(1)).await;
                if let Err(e) = leds.clear().await {
                    println!("[LED] clear error: {:?}", e);
                }
            }
            event => println!("[BUTTON] Event: {:?}", event),
        }
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

    static SHARED_I2C: StaticCell<
        tildagon::i2c::SharedI2cBus<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>,
    > = StaticCell::new();
    let shared_i2c = SHARED_I2C.init(AsyncMutex::new(tildagon.i2c.into_async()));
    let mut button_int = tildagon.button_int;
    let mut buttons = TypedButtons::new(system_i2c_bus(shared_i2c), top_i2c_bus(shared_i2c));
    let pins = Pins::new();

    let leds = TypedLeds::new(
        tildagon.rmt,
        tildagon.led_pin,
        pins.led,
        system_i2c_bus(shared_i2c),
    )
    .await
    .expect("Typed LED init failed");

    // BLE Init


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
        central: _,
        runner: _ble_runner,
        ..
    } = ble_stack.build();

    static BUTTON_CHANNEL: StaticCell<
        PubSubChannel<CriticalSectionRawMutex, ButtonEvent, 8, 4, 1>,
    > = StaticCell::new();
    let channel = BUTTON_CHANNEL.init(PubSubChannel::new());

    spawner
        .spawn(button_handler_task(
            channel.subscriber().unwrap(),
            leds,
            Battery::new(system_i2c_bus(shared_i2c)),
        ))
        .expect("Failed to spawn button_handler_task");

    let publisher = channel.publisher().unwrap();

    println!("[BUTTON] Waiting for button events...");
    loop {
        match buttons.wait_for_event(&mut button_int).await {
            Ok(Some(event)) => {
                publisher.publish_immediate(event);
            }
            Ok(None) => {}
            Err(e) => println!("[BUTTON] Error reading buttons: {:?}", e),
        }
    }
}
