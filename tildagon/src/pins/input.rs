use super::{
    async_digital,
    aw9523b::{
        GpioDirection, PinMode, Port, Register,
        read_register, set_io_direction, set_pin_mode, write_register,
    },
    output::OutputPin,
    pin::{PinExt, TypeErasedPin},
};
use embedded_hal::digital::ErrorKind;

/// An AW9523B GPIO pin configured as an input.
pub struct InputPin<I2C> {
    bus: I2C,
    pin: TypeErasedPin,
}

impl<I2C, E> InputPin<I2C>
where
    I2C: embedded_hal_async::i2c::I2c<Error = E>,
{
    pub(crate) async fn try_new(mut bus: I2C, pin: TypeErasedPin) -> Result<Self, E> {
        set_pin_mode(&mut bus, &pin, PinMode::Gpio).await?;
        set_io_direction(&mut bus, &pin, GpioDirection::Input).await?;
        Ok(Self { bus, pin })
    }

    /// Reconfigure this pin as an output.
    pub async fn into_output(self) -> Result<OutputPin<I2C>, E> {
        OutputPin::try_new(self.bus, self.pin).await
    }

    /// Enable or disable the interrupt for this pin on the AW9523B.
    ///
    /// `true` = interrupt fires on state change; `false` = interrupt masked.
    pub async fn set_interrupt(&mut self, enable: bool) -> Result<(), E> {
        let register = match self.pin.port() {
            Port::Port0 => Register::INT_P0,
            Port::Port1 => Register::INT_P1,
        };
        let current = read_register(&mut self.bus, self.pin.address(), register).await?;
        let updated = if enable {
            current & !self.pin.bit()
        } else {
            current | self.pin.bit()
        };
        write_register(&mut self.bus, self.pin.address(), register, updated).await?;
        log::debug!(
            "AW9523B 0x{:02x} pin{}: interrupt -> {}",
            self.pin.address(), self.pin.pin(), enable
        );
        Ok(())
    }
}

impl<I2C> embedded_hal::digital::ErrorType for InputPin<I2C> {
    type Error = ErrorKind;
}

impl<I2C, E> async_digital::InputPin for InputPin<I2C>
where
    I2C: embedded_hal_async::i2c::I2c<Error = E>,
{
    async fn is_high(&mut self) -> Result<bool, Self::Error> {
        Ok(!self.is_low().await?)
    }

    async fn is_low(&mut self) -> Result<bool, Self::Error> {
        let register = match self.pin.port() {
            Port::Port0 => Register::INPUT_P0,
            Port::Port1 => Register::INPUT_P1,
        };
        let value = read_register(&mut self.bus, self.pin.address(), register)
            .await
            .map_err(|_| ErrorKind::Other)?;
        Ok(value & self.pin.bit() == 0)
    }
}
