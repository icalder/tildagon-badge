use crate::events::{BUTTON_EVENTS, ButtonEvent, DISPLAY_SIGNAL};
use crate::itag::{
    ALERT_LEVEL_CHARACTERISTIC_UUID, ALERT_LEVEL_MILD, ALERT_LEVEL_OFF, APP_STATE,
    IMMEDIATE_ALERT_SERVICE_UUID, Mode, disconnect_and_wait, write_alert_level,
};
use core::future::Future;
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_time::{Duration, Timer, with_timeout};
use esp_println::println;
use trouble_host::central::Central;
use trouble_host::prelude::*;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(12);
const GATT_CLIENT_TIMEOUT: Duration = Duration::from_secs(6);
const SERVICE_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);
const CHARACTERISTIC_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);
const ALERT_WRITE_TIMEOUT: Duration = Duration::from_secs(3);
const ALERT_START_DELAY: Duration = Duration::from_millis(500);
const ALARM_POLL_INTERVAL: Duration = Duration::from_secs(2);

enum ConnectWaitError {
    Cancelled,
    TimedOut,
}

pub async fn run_connecting(
    central: &mut Central<'static, crate::BleExternalController, DefaultPacketPool>,
    stack: &'static Stack<'static, crate::BleExternalController, DefaultPacketPool>,
) {
    let target_addr = {
        let state = APP_STATE.lock().await;
        state.target_addr
    };
    let Some(target_addr) = target_addr else {
        println!("BLE: No target address set, returning to scan");
        reset_to_scanning().await;
        return;
    };

    println!("BLE: Connecting to {:?}", target_addr.addr);
    let filter = [(target_addr.kind, &target_addr.addr)];
    let config = crate::ble::build_connect_config(&filter);

    match run_with_back_timeout(
        "Connection attempt",
        CONNECT_TIMEOUT,
        central.connect(&config),
    )
    .await
    {
        Ok(Ok(connection)) => {
            println!("BLE: Connected to {:?}", target_addr.addr);
            match run_with_back_timeout(
                "GATT client setup",
                GATT_CLIENT_TIMEOUT,
                GattClient::<crate::BleExternalController, DefaultPacketPool, 10>::new(
                    stack,
                    &connection,
                ),
            )
            .await
            {
                Ok(Ok(client)) => {
                    let gatt_task = client.task();
                    let alarm_task = async {
                        Timer::after(ALERT_START_DELAY).await;

                        let services = match run_with_back_timeout(
                            "Immediate Alert service discovery",
                            SERVICE_DISCOVERY_TIMEOUT,
                            client.services_by_uuid(&Uuid::new_short(IMMEDIATE_ALERT_SERVICE_UUID)),
                        )
                        .await
                        {
                            Ok(Ok(services)) => services,
                            Ok(Err(e)) => {
                                println!(
                                    "BLE: Service 0x{:04X} lookup failed: {:?}",
                                    IMMEDIATE_ALERT_SERVICE_UUID, e
                                );
                                return;
                            }
                            Err(ConnectWaitError::Cancelled | ConnectWaitError::TimedOut) => {
                                return;
                            }
                        };

                        if services.is_empty() {
                            println!(
                                "BLE: Service 0x{:04X} not found",
                                IMMEDIATE_ALERT_SERVICE_UUID
                            );
                            return;
                        }

                        let service = &services[0];
                        match run_with_back_timeout(
                            "Alert Level characteristic discovery",
                            CHARACTERISTIC_DISCOVERY_TIMEOUT,
                            client.characteristic_by_uuid::<[u8]>(
                                service,
                                &Uuid::new_short(ALERT_LEVEL_CHARACTERISTIC_UUID),
                            ),
                        )
                        .await
                        {
                            Ok(Ok(characteristic)) => {
                                println!("BLE: Triggering mild alert...");
                                if match run_with_back_timeout(
                                    "Alert start write",
                                    ALERT_WRITE_TIMEOUT,
                                    write_alert_level(&client, &characteristic, ALERT_LEVEL_MILD),
                                )
                                .await
                                {
                                    Ok(started) => started,
                                    Err(
                                        ConnectWaitError::Cancelled | ConnectWaitError::TimedOut,
                                    ) => return,
                                } {
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
                                        Timer::after(ALARM_POLL_INTERVAL),
                                        BUTTON_EVENTS.receive(),
                                    )
                                    .await
                                    {
                                        Either::First(_) => {}
                                        Either::Second(ButtonEvent::Back)
                                        | Either::Second(ButtonEvent::Select) => {
                                            println!("BLE: Stopping alarm");
                                            match run_with_timeout(
                                                "Alert stop write",
                                                ALERT_WRITE_TIMEOUT,
                                                write_alert_level(
                                                    &client,
                                                    &characteristic,
                                                    ALERT_LEVEL_OFF,
                                                ),
                                            )
                                            .await
                                            {
                                                Some(false) => {
                                                    println!(
                                                        "BLE: Failed to stop alert level {}",
                                                        ALERT_LEVEL_OFF
                                                    );
                                                }
                                                Some(true) | None => {}
                                            }
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            Ok(Err(e)) => println!(
                                "BLE: Characteristic 0x{:04X} not found: {:?}",
                                ALERT_LEVEL_CHARACTERISTIC_UUID, e
                            ),
                            Err(ConnectWaitError::Cancelled | ConnectWaitError::TimedOut) => {}
                        }
                    };

                    match select3(gatt_task, alarm_task, wait_for_back_button()).await {
                        Either3::First(_) => println!("BLE: GATT task finished"),
                        Either3::Second(_) => println!("BLE: Alarm task finished"),
                        Either3::Third(_) => {
                            println!("BLE: Back pressed, disconnecting")
                        }
                    }
                }
                Ok(Err(e)) => println!("BLE: GATT client failed: {:?}", e),
                Err(ConnectWaitError::Cancelled | ConnectWaitError::TimedOut) => {}
            }
            disconnect_and_wait(&connection).await;
            reset_to_scanning().await;
        }
        Ok(Err(e)) => {
            println!("BLE: Connection failed: {:?}", e);
            reset_to_scanning().await;
        }
        Err(ConnectWaitError::Cancelled | ConnectWaitError::TimedOut) => {
            reset_to_scanning().await;
        }
    }
}

pub async fn handle_alarming() {
    let mut state = APP_STATE.lock().await;
    state.mode = Mode::Scanning;
}

async fn wait_for_back_button() {
    loop {
        if BUTTON_EVENTS.receive().await == ButtonEvent::Back {
            return;
        }
    }
}

async fn run_with_back_timeout<T, F>(
    label: &'static str,
    timeout: Duration,
    future: F,
) -> Result<T, ConnectWaitError>
where
    F: Future<Output = T>,
{
    match select3(future, wait_for_back_button(), Timer::after(timeout)).await {
        Either3::First(value) => Ok(value),
        Either3::Second(_) => {
            println!("BLE: {} cancelled by back button", label);
            Err(ConnectWaitError::Cancelled)
        }
        Either3::Third(_) => {
            println!("BLE: {} timed out after {} ms", label, timeout.as_millis());
            Err(ConnectWaitError::TimedOut)
        }
    }
}

async fn run_with_timeout<T, F>(label: &'static str, timeout: Duration, future: F) -> Option<T>
where
    F: Future<Output = T>,
{
    match with_timeout(timeout, future).await {
        Ok(value) => Some(value),
        Err(_) => {
            println!("BLE: {} timed out after {} ms", label, timeout.as_millis());
            None
        }
    }
}

async fn reset_to_scanning() {
    let mut state = APP_STATE.lock().await;
    state.mode = Mode::Scanning;
    state.target_addr = None;
    DISPLAY_SIGNAL.signal(());
}
