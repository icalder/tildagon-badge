//! AW9523B GPIO expander register definitions and low-level I2C helpers.
//!
//! # Attribution
//! Port types, register map, and pin-state helpers ported from
//! [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs)
//! (MIT / Apache-2.0), adapted to use `log` instead of `defmt`.

use core::marker::ConstParamTy;

use super::pin::PinExt;
use embedded_hal_async::i2c::I2c;

/// Which port of the AW9523B the pin belongs to.
///
/// # Attribution
/// Ported from [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs).
#[derive(Debug, Copy, Clone, PartialEq, Eq, ConstParamTy)]
pub enum Port {
    Port0,
    Port1,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum PinMode {
    Gpio,
    Led,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum GpioDirection {
    Input,
    Output,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
#[allow(dead_code, non_camel_case_types, clippy::upper_case_acronyms)]
pub(super) enum Register {
    INPUT_P0  = 0x00,
    INPUT_P1  = 0x01,
    OUTPUT_P0 = 0x02,
    OUTPUT_P1 = 0x03,
    CONFIG_P0 = 0x04,
    CONFIG_P1 = 0x05,
    INT_P0    = 0x06,
    INT_P1    = 0x07,
    ID        = 0x10,
    CTL       = 0x11,
    LEDMS_P0  = 0x12,
    LEDMS_P1  = 0x13,
    SW_RSTN   = 0x7F,
}

pub(super) async fn write_register<I2C, E>(
    bus: &mut I2C,
    addr: u8,
    register: Register,
    value: u8,
) -> Result<(), E>
where
    I2C: I2c<Error = E>,
{
    bus.write(addr, &[register as u8, value]).await
}

pub(super) async fn read_register<I2C, E>(
    bus: &mut I2C,
    addr: u8,
    register: Register,
) -> Result<u8, E>
where
    I2C: I2c<Error = E>,
{
    let mut val = [0u8; 1];
    bus.write_read(addr, &[register as u8], &mut val)
        .await
        .and(Ok(val[0]))
}

pub(crate) async fn set_pin_mode<I2C, E, PIN: PinExt>(
    bus: &mut I2C,
    pin: &PIN,
    mode: PinMode,
) -> Result<(), E>
where
    I2C: I2c<Error = E>,
{
    let register = match pin.port() {
        Port::Port0 => Register::LEDMS_P0,
        Port::Port1 => Register::LEDMS_P1,
    };
    let current = read_register(bus, pin.address(), register).await?;
    let updated = match mode {
        PinMode::Gpio => current | pin.bit(),
        PinMode::Led  => current & !pin.bit(),
    };
    write_register(bus, pin.address(), register, updated).await?;
    log::debug!(
        "AW9523B 0x{:02x} port{} pin{}: mode -> {:?}",
        pin.address(), pin.port() as u8, pin.pin(), mode
    );
    Ok(())
}

pub(crate) async fn set_io_direction<I2C, E, PIN: PinExt + ?Sized>(
    bus: &mut I2C,
    pin: &PIN,
    direction: GpioDirection,
) -> Result<(), E>
where
    I2C: I2c<Error = E>,
{
    let register = match pin.port() {
        Port::Port0 => Register::CONFIG_P0,
        Port::Port1 => Register::CONFIG_P1,
    };
    let current = read_register(bus, pin.address(), register).await?;
    let updated = match direction {
        GpioDirection::Input  => current | pin.bit(),
        GpioDirection::Output => current & !pin.bit(),
    };
    write_register(bus, pin.address(), register, updated).await?;
    log::debug!(
        "AW9523B 0x{:02x} port{} pin{}: direction -> {:?}",
        pin.address(), pin.port() as u8, pin.pin(), direction
    );
    Ok(())
}

pub(crate) async fn set_io_state<I2C, E, PIN: PinExt + ?Sized>(
    bus: &mut I2C,
    pin: &PIN,
    state: embedded_hal::digital::PinState,
) -> Result<(), E>
where
    I2C: I2c<Error = E>,
{
    let register = match pin.port() {
        Port::Port0 => Register::OUTPUT_P0,
        Port::Port1 => Register::OUTPUT_P1,
    };
    let current = read_register(bus, pin.address(), register).await?;
    let updated = match state {
        embedded_hal::digital::PinState::Low  => current & !pin.bit(),
        embedded_hal::digital::PinState::High => current | pin.bit(),
    };
    write_register(bus, pin.address(), register, updated).await?;
    log::debug!(
        "AW9523B 0x{:02x} port{} pin{}: state -> {:?}",
        pin.address(), pin.port() as u8, pin.pin(), state
    );
    Ok(())
}
