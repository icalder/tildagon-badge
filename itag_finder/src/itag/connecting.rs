use crate::events::{BUTTON_EVENTS, ButtonEvent, DISPLAY_SIGNAL};
use crate::itag::{
    ALERT_LEVEL_CHARACTERISTIC_UUID, ALERT_LEVEL_MILD, ALERT_LEVEL_OFF,
    IMMEDIATE_ALERT_SERVICE_UUID, APP_STATE, Mode, disconnect_and_wait, write_alert_level,
};
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_time::{Duration, Timer};
use esp_println::println;
use trouble_host::central::Central;
use trouble_host::prelude::*;

pub async fn run_connecting(
    central: &mut Central<'static, crate::BleExternalController, DefaultPacketPool>,
    stack: &'static Stack<'static, crate::BleExternalController, DefaultPacketPool>,
) {
    let target_addr = {
        let state = APP_STATE.lock().await;
        state.target_addr.unwrap()
    };

    println!("BLE: Connecting to {:?}", target_addr.addr);
    let filter = [(target_addr.kind, &target_addr.addr)];
    let config = crate::ble::build_connect_config(&filter);

    match select(central.connect(&config), BUTTON_EVENTS.receive()).await {
        Either::First(Ok(connection)) => {
            println!("BLE: Connected to {:?}", target_addr.addr);
            match GattClient::<crate::BleExternalController, DefaultPacketPool, 10>::new(
                stack,
                &connection,
            )
            .await
            {
                Ok(client) => {
                    let gatt_task = client.task();
                    let alarm_task = async {
                        Timer::after(Duration::from_millis(500)).await;
                        match client
                            .services_by_uuid(&Uuid::new_short(IMMEDIATE_ALERT_SERVICE_UUID))
                            .await
                        {
                            Ok(services) if !services.is_empty() => {
                                let service = &services[0];
                                match client
                                    .characteristic_by_uuid::<[u8]>(
                                        service,
                                        &Uuid::new_short(ALERT_LEVEL_CHARACTERISTIC_UUID),
                                    )
                                    .await
                                {
                                    Ok(characteristic) => {
                                        println!("BLE: Triggering mild alert...");
                                        if write_alert_level(
                                            &client,
                                            &characteristic,
                                            ALERT_LEVEL_MILD,
                                        )
                                        .await
                                        {
                                            let mut state = APP_STATE.lock().await;
                                            state.mode = Mode::Alarming;
                                            DISPLAY_SIGNAL.signal(());
                                        } else {
                                            println!(
                                                "BLE: Failed to trigger alert level {}",
                                                ALERT_LEVEL_MILD
                                            );
                                            return;
                                        }

                                        loop {
                                            match select(
                                                Timer::after(Duration::from_secs(2)),
                                                BUTTON_EVENTS.receive(),
                                            )
                                            .await
                                            {
                                                Either::First(_) => {}
                                                Either::Second(ButtonEvent::Back)
                                                | Either::Second(ButtonEvent::Select) => {
                                                    println!("BLE: Stopping alarm");
                                                    if !write_alert_level(
                                                        &client,
                                                        &characteristic,
                                                        ALERT_LEVEL_OFF,
                                                    )
                                                    .await
                                                    {
                                                        println!(
                                                            "BLE: Failed to stop alert level {}",
                                                            ALERT_LEVEL_OFF
                                                        );
                                                    }
                                                    break;
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                    Err(e) => println!(
                                        "BLE: Characteristic 0x{:04X} not found: {:?}",
                                        ALERT_LEVEL_CHARACTERISTIC_UUID, e
                                    ),
                                }
                            }
                            _ => println!(
                                "BLE: Service 0x{:04X} not found",
                                IMMEDIATE_ALERT_SERVICE_UUID
                            ),
                        }
                    };

                    match select3(gatt_task, alarm_task, BUTTON_EVENTS.receive()).await {
                        Either3::First(_) => println!("BLE: GATT task finished"),
                        Either3::Second(_) => println!("BLE: Alarm task finished"),
                        Either3::Third(ButtonEvent::Back) => {
                            println!("BLE: Back pressed, disconnecting")
                        }
                        Either3::Third(_) => {}
                    }
                }
                Err(e) => println!("BLE: GATT client failed: {:?}", e),
            }
            disconnect_and_wait(&connection).await;
            {
                let mut state = APP_STATE.lock().await;
                state.mode = Mode::Scanning;
                state.target_addr = None;
                DISPLAY_SIGNAL.signal(());
            }
        }
        Either::First(Err(e)) => {
            println!("BLE: Connection failed: {:?}", e);
            let mut state = APP_STATE.lock().await;
            state.mode = Mode::Scanning;
            DISPLAY_SIGNAL.signal(());
        }
        Either::Second(ButtonEvent::Back) => {
            println!("BLE: Connection cancelled");
            let mut state = APP_STATE.lock().await;
            state.mode = Mode::Scanning;
            DISPLAY_SIGNAL.signal(());
        }
        Either::Second(_) => {}
    }
}

pub async fn handle_alarming() {
    let mut state = APP_STATE.lock().await;
    state.mode = Mode::Scanning;
}
