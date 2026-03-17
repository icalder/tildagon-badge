//! Typed compile-time pin representation for AW9523B GPIO expander pins.
//!
//! # Attribution
//! Ported from [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs).

use super::{InputPin, OutputPin, aw9523b::Port};

pub(crate) trait PinExt {
    fn address(&self) -> u8;
    fn port(&self) -> Port;
    fn pin(&self) -> u8;
    fn bit(&self) -> u8;
}

fn pin_bit(pin: u8) -> u8 {
    match pin {
        0 => 0b00000001,
        1 => 0b00000010,
        2 => 0b00000100,
        3 => 0b00001000,
        4 => 0b00010000,
        5 => 0b00100000,
        6 => 0b01000000,
        7 => 0b10000000,
        _ => unreachable!(),
    }
}

pub(crate) struct TypeErasedPin {
    pub(crate) address: u8,
    pub(crate) port: Port,
    pub(crate) pin: u8,
}

impl PinExt for TypeErasedPin {
    fn address(&self) -> u8 { self.address }
    fn port(&self) -> Port  { self.port }
    fn pin(&self) -> u8     { self.pin }
    fn bit(&self) -> u8     { pin_bit(self.pin) }
}

impl<const ADDRESS: u8, const PORT: Port, const PIN: u8> From<Pin<ADDRESS, PORT, PIN>>
    for TypeErasedPin
{
    fn from(_: Pin<ADDRESS, PORT, PIN>) -> Self {
        Self { address: ADDRESS, port: PORT, pin: PIN }
    }
}

/// A compile-time typed I2C expander pin.
///
/// The I2C address, port, and bit number are const generic parameters, so any
/// hardware-mapping mistake is caught at compile time rather than at runtime.
///
/// # Attribution
/// Ported from [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs).
pub struct Pin<const ADDRESS: u8, const PORT: Port, const PIN: u8> {}

impl<const ADDRESS: u8, const PORT: Port, const PIN: u8> Pin<ADDRESS, PORT, PIN> {
    pub(super) fn new() -> Self {
        Self {}
    }

    /// Configure this pin as a GPIO output and return an [`OutputPin`].
    pub async fn into_output<I2C, E>(self, bus: I2C) -> Result<OutputPin<I2C>, E>
    where
        I2C: embedded_hal_async::i2c::I2c<Error = E>,
    {
        OutputPin::try_new(bus, self.into()).await
    }

    /// Configure this pin as a GPIO input and return an [`InputPin`].
    pub async fn into_input<I2C, E>(self, bus: I2C) -> Result<InputPin<I2C>, E>
    where
        I2C: embedded_hal_async::i2c::I2c<Error = E>,
    {
        InputPin::try_new(bus, self.into()).await
    }
}

impl<const ADDRESS: u8, const PORT: Port, const PIN: u8> PinExt for Pin<ADDRESS, PORT, PIN> {
    fn address(&self) -> u8 { ADDRESS }
    fn port(&self) -> Port  { PORT }
    fn pin(&self) -> u8     { PIN }
    fn bit(&self) -> u8     { pin_bit(PIN) }
}
