use embassy_time::{Duration, Timer};
use embassy_sync::pubsub::{PubSubChannel, Subscriber};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use heapless::Vec;
use crate::Error;

/// A button press on the Tildagon badge hex-pad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    A, B, C, D, E, F
}

/// A button press or release event on the Tildagon badge.
///
/// # Attribution
/// Event type and variant names ported from
/// [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonEvent {
    Pressed(Button),
    Released(Button),
}

/// A button reader that uses the mux-aware I2C bus and typed pins.
///
/// This implementation is designed for reliable polling-based detection,
/// which is much more stable during high radio activity than the legacy
/// interrupt-driven approach.
///
/// # Attribution
/// Architecture and event-handling pattern ported from
/// [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs).
pub struct TypedButtons<BUS: 'static> {
    system_i2c: crate::i2c::SystemI2cBus<BUS>,
    last_state_59: u8,
    last_state_5a: u8,
}

impl<BUS: 'static> TypedButtons<BUS>
where
    BUS: embedded_hal_async::i2c::I2c,
    crate::Error: From<BUS::Error>,
{
    /// Create a new `TypedButtons` handle.
    pub fn new(
        system_i2c: crate::i2c::SystemI2cBus<BUS>,
    ) -> Self {
        Self {
            system_i2c,
            last_state_59: 0xFF,
            last_state_5a: 0xFF,
        }
    }

    /// Surgically poll for button state changes.
    ///
    /// This method ignores the shared interrupt line and only reads the
    /// registers for the two chips containing buttons. This is faster and
    /// much more reliable than interrupt-driven reads during high radio activity.
    pub async fn poll(&mut self) -> Result<Vec<ButtonEvent, 12>, Error> {
        let mut port0_59 = [self.last_state_59; 1];
        let mut port0_5a = [self.last_state_5a; 1];

        // Surgical 2-read check
        {
            let mut bus: embassy_sync::mutex::MutexGuard<'_, crate::i2c::SharingRawMutex, BUS> = 
                self.system_i2c.lock().await?;
            bus.write_read(0x59u8, &[0x00], &mut port0_59).await?;
            bus.write_read(0x5au8, &[0x00], &mut port0_5a).await?;
        }

        let changed_59 = port0_59[0] ^ self.last_state_59;
        let changed_5a = port0_5a[0] ^ self.last_state_5a;

        if changed_59 == 0 && changed_5a == 0 {
            return Ok(Vec::new());
        }

        let mut events = Vec::new();

        macro_rules! check_button {
            ($changed:expr, $current:expr, $bit:expr, $btn:expr) => {
                if $changed & (1 << $bit) != 0 {
                    let event = if $current & (1 << $bit) == 0 {
                        ButtonEvent::Pressed($btn)
                    } else {
                        ButtonEvent::Released($btn)
                    };
                    let _ = events.push(event);
                }
            };
        }

        check_button!(changed_59, port0_59[0], 0, Button::C);
        check_button!(changed_59, port0_59[0], 1, Button::D);
        check_button!(changed_59, port0_59[0], 2, Button::E);
        check_button!(changed_59, port0_59[0], 3, Button::F);
        check_button!(changed_5a, port0_5a[0], 6, Button::A);
        check_button!(changed_5a, port0_5a[0], 7, Button::B);

        self.last_state_59 = port0_59[0];
        self.last_state_5a = port0_5a[0];

        Ok(events)
    }
}

/// Global button event channel.
static BUTTON_CHANNEL: PubSubChannel<CriticalSectionRawMutex, ButtonEvent, 16, 4, 1> = 
    PubSubChannel::new();

/// Service that polls buttons in the background and broadcasts events.
pub struct ButtonManager;

impl ButtonManager {
    /// Subscribe to the button event stream.
    pub fn subscribe(&self) -> Subscriber<'static, CriticalSectionRawMutex, ButtonEvent, 16, 4, 1> {
        BUTTON_CHANNEL.subscriber().unwrap()
    }
}

/// Background task that performs surgical polling and broadcasts events.
#[embassy_executor::task]
pub async fn button_manager_task(mut buttons: TypedButtons<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>) {
    let publisher = BUTTON_CHANNEL.publisher().unwrap();
    
    // Give the system 100ms to settle (WiFi/BLE init, etc.) before starting the poll loop.
    // This prevents a single I2C timeout during the "startup storm".
    Timer::after(Duration::from_millis(100)).await;

    // Warm-up poll to synchronize last_state without broadcasting events.
    let _ = buttons.poll().await;

    let mut ticker = embassy_time::Ticker::every(Duration::from_millis(20));
    loop {
        ticker.next().await;
        match buttons.poll().await {
            Ok(events) => {
                for event in events {
                    publisher.publish(event).await;
                }
            }
            Err(e) => {
                // If we get an I2C error (like a timeout during a WiFi scan),
                // we just log it and try again on the next tick.
                esp_println::println!("[BUTTON] Poll error: {:?}", e);
            }
        }
    }
}
