use esp_hal::gpio::Input;
use esp_hal::i2c::master::I2c;
use esp_hal::Blocking;
use embassy_time::{Duration, Timer};
use embedded_hal_async::i2c::I2c as _;
use crate::Error;

/// A button press on the Tildagon badge hex-pad.
///
/// # Compatibility Baseline (Phase 0)
/// All six variants are part of the stable surface consumed by `embassy_blinky`.
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

/// Interrupt-driven button reader.
///
/// # Compatibility Baseline (Phase 0)
/// [`Buttons::new`] and [`Buttons::wait_for_press`] are the stable entry points
/// consumed by `embassy_blinky`. They must keep the same signatures through
/// Phases 1-3. An interrupt-driven async event API will be added as an
/// *additive* API in Phase 3, not as a replacement.
pub struct Buttons {
    last_state_59: u8,
    last_state_5a: u8,
}

impl Buttons {
    /// Create a new `Buttons` with no remembered state.
    ///
    /// # Compatibility Baseline (Phase 0)
    pub fn new() -> Self {
        Self {
            last_state_59: 0xFF,
            last_state_5a: 0xFF,
        }
    }

    /// Block until a button is pressed and return which one.
    ///
    /// Waits for a falling edge on the shared INT line, debounces it, then
    /// reads the AW9523B expanders (0x59, 0x5a) via the mux to determine
    /// which button changed. Also clears FUSB302B and BQ25895 interrupt
    /// sources so the INT line can de-assert.
    ///
    /// # Compatibility Baseline (Phase 0)
    /// This signature must remain stable through Phases 1-3.
    pub async fn wait_for_press(
        &mut self,
        i2c: &mut I2c<'_, Blocking>,
        button_int: &mut Input<'_>,
    ) -> Result<Option<Button>, Error> {
        loop {
            button_int.wait_for_falling_edge().await;

            if button_int.is_high() {
                continue;
            }

            Timer::after(Duration::from_millis(20)).await;
            if button_int.is_high() {
                continue;
            }

            let mut port0_58 = [0u8; 1];
            let mut port0_59 = [self.last_state_59; 1];
            let mut port1_59 = [0u8; 1];
            let mut port0_5a = [self.last_state_5a; 1];
            let mut port1_5a = [0u8; 1];
            let mut dummy = [0u8; 2];

            // 1. Check chips on Port 7 (System)
            i2c.write(0x77u8, &[1 << 7])?;
            if button_int.is_low() {
                i2c.write_read(0x58u8, &[0x00], &mut port0_58)?;
                i2c.write_read(0x58u8, &[0x01], &mut port0_58)?;
            }
            if button_int.is_low() {
                i2c.write_read(0x59u8, &[0x00], &mut port0_59)?;
                i2c.write_read(0x59u8, &[0x01], &mut port1_59)?;
            }
            if button_int.is_low() {
                i2c.write_read(0x5au8, &[0x00], &mut port0_5a)?;
                i2c.write_read(0x5au8, &[0x01], &mut port1_5a)?;
            }
            if button_int.is_low() {
                i2c.write_read(0x6Au8, &[0x0B], &mut dummy)?;
                i2c.write_read(0x22u8, &[0x3E], &mut dummy)?;
                i2c.write_read(0x22u8, &[0x42], &mut dummy[..1])?;
            }

            // 2. Check chips on Port 0 (USB Out)
            if button_int.is_low() {
                i2c.write(0x77u8, &[1 << 0])?;
                i2c.write_read(0x22u8, &[0x3E], &mut dummy)?;
                i2c.write_read(0x22u8, &[0x42], &mut dummy[..1])?;
                i2c.write(0x77u8, &[1 << 7])?;
            }

            let changed_59 = port0_59[0] ^ self.last_state_59;
            let pressed_59 = !port0_59[0] & changed_59;
            
            let changed_5a = port0_5a[0] ^ self.last_state_5a;
            let pressed_5a = !port0_5a[0] & changed_5a;

            self.last_state_59 = port0_59[0];
            self.last_state_5a = port0_5a[0];

            if pressed_59 & (1 << 0) != 0 {
                return Ok(Some(Button::C));
            }
            if pressed_59 & (1 << 1) != 0 {
                return Ok(Some(Button::D));
            }
            if pressed_59 & (1 << 2) != 0 {
                return Ok(Some(Button::E));
            }
            if pressed_59 & (1 << 3) != 0 {
                return Ok(Some(Button::F));
            }
            if pressed_5a & (1 << 6) != 0 {
                return Ok(Some(Button::A));
            }
            if pressed_5a & (1 << 7) != 0 {
                return Ok(Some(Button::B));
            }

            if button_int.is_low() {
                Timer::after(Duration::from_millis(10)).await;
            }
        }
    }
}

/// A button reader that uses the new mux-aware I2C bus and typed pins.
///
/// # Attribution
/// Architecture and event-handling pattern ported from
/// [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs).
pub struct TypedButtons<BUS: 'static> {
    system_i2c: crate::i2c::SystemI2cBus<BUS>,
    top_i2c: crate::i2c::TopBoardI2cBus<BUS>,
    last_state_59: u8,
    last_state_5a: u8,
}

impl<BUS: 'static> TypedButtons<BUS>
where
    BUS: embedded_hal_async::i2c::I2c,
    crate::Error: From<BUS::Error>,
{
    /// Create a new `TypedButtons` handle.
    ///
    /// Takes ownership of the system and top-board mux-aware I2C handles so it
    /// can preserve the badge's cross-bus interrupt silencing behaviour.
    pub fn new(
        system_i2c: crate::i2c::SystemI2cBus<BUS>,
        top_i2c: crate::i2c::TopBoardI2cBus<BUS>,
    ) -> Self {
        Self {
            system_i2c,
            top_i2c,
            last_state_59: 0xFF,
            last_state_5a: 0xFF,
        }
    }

    /// Block until a button event (press or release) occurs.
    ///
    /// # Attribution
    /// Logic ported from [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs),
    /// adapted to include the silence-interrupts logic required for the 2024 badge.
    pub async fn wait_for_event(
        &mut self,
        button_int: &mut Input<'_>,
    ) -> Result<Option<ButtonEvent>, Error> {
        loop {
            button_int.wait_for_falling_edge().await;

            if button_int.is_high() {
                continue;
            }

            Timer::after(Duration::from_millis(20)).await;
            if button_int.is_high() {
                continue;
            }

            let mut port0_58 = [0u8; 1];
            let mut port0_59 = [self.last_state_59; 1];
            let mut port1_59 = [0u8; 1];
            let mut port0_5a = [self.last_state_5a; 1];
            let mut port1_5a = [0u8; 1];
            let mut dummy = [0u8; 2];

            // 1. Check chips on Port 7 (System)
            // Note: SystemI2cBus already handles switching to mux port 7.
            if button_int.is_low() {
                self.system_i2c.write_read(0x58u8, &[0x00], &mut port0_58).await?;
                self.system_i2c.write_read(0x58u8, &[0x01], &mut port0_58).await?;
            }
            if button_int.is_low() {
                self.system_i2c.write_read(0x59u8, &[0x00], &mut port0_59).await?;
                self.system_i2c.write_read(0x59u8, &[0x01], &mut port1_59).await?;
            }
            if button_int.is_low() {
                self.system_i2c.write_read(0x5au8, &[0x00], &mut port0_5a).await?;
                self.system_i2c.write_read(0x5au8, &[0x01], &mut port1_5a).await?;
            }
            if button_int.is_low() {
                self.system_i2c.write_read(0x6Au8, &[0x0B], &mut dummy).await?;
                self.system_i2c.write_read(0x22u8, &[0x3E], &mut dummy).await?;
                self.system_i2c.write_read(0x22u8, &[0x42], &mut dummy[..1]).await?;
            }

            // 2. Check chips on Port 0 (USB Out)
            if button_int.is_low() {
                self.top_i2c.write_read(0x22u8, &[0x3E], &mut dummy).await?;
                self.top_i2c.write_read(0x22u8, &[0x42], &mut dummy[..1]).await?;
            }

            let changed_59 = port0_59[0] ^ self.last_state_59;
            let pressed_59 = !port0_59[0] & changed_59;
            let released_59 = port0_59[0] & changed_59;
            
            let changed_5a = port0_5a[0] ^ self.last_state_5a;
            let pressed_5a = !port0_5a[0] & changed_5a;
            let released_5a = port0_5a[0] & changed_5a;

            self.last_state_59 = port0_59[0];
            self.last_state_5a = port0_5a[0];

            if pressed_59 & (1 << 0) != 0 { return Ok(Some(ButtonEvent::Pressed(Button::C))); }
            if pressed_59 & (1 << 1) != 0 { return Ok(Some(ButtonEvent::Pressed(Button::D))); }
            if pressed_59 & (1 << 2) != 0 { return Ok(Some(ButtonEvent::Pressed(Button::E))); }
            if pressed_59 & (1 << 3) != 0 { return Ok(Some(ButtonEvent::Pressed(Button::F))); }
            if pressed_5a & (1 << 6) != 0 { return Ok(Some(ButtonEvent::Pressed(Button::A))); }
            if pressed_5a & (1 << 7) != 0 { return Ok(Some(ButtonEvent::Pressed(Button::B))); }

            if released_59 & (1 << 0) != 0 { return Ok(Some(ButtonEvent::Released(Button::C))); }
            if released_59 & (1 << 1) != 0 { return Ok(Some(ButtonEvent::Released(Button::D))); }
            if released_59 & (1 << 2) != 0 { return Ok(Some(ButtonEvent::Released(Button::E))); }
            if released_59 & (1 << 3) != 0 { return Ok(Some(ButtonEvent::Released(Button::F))); }
            if released_5a & (1 << 6) != 0 { return Ok(Some(ButtonEvent::Released(Button::A))); }
            if released_5a & (1 << 7) != 0 { return Ok(Some(ButtonEvent::Released(Button::B))); }

            if button_int.is_low() {
                Timer::after(Duration::from_millis(10)).await;
            }
        }
    }
}
