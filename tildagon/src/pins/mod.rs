//! I2C expander pin types and async digital traits.
//!
//! # Attribution
//! Architecture and pin mapping ported from
//! [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs).

mod assignment;
pub(crate) mod aw9523b;
mod input;
mod output;
pub(crate) mod pin;

pub use assignment::{
    ButtonPins, HexpansionDetectPins, LedPins, OtherPins, Pins, TopBoardPins,
};
pub use aw9523b::Port;
pub use input::InputPin;
pub use output::OutputPin;
pub use pin::Pin;

/// Async versions of `embedded-hal` digital traits.
///
/// # Attribution
/// Trait definitions ported from
/// [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs).
pub mod async_digital {
    use embedded_hal::digital::{ErrorType, PinState};

    #[allow(async_fn_in_trait)]
    pub trait OutputPin: ErrorType {
        async fn set_low(&mut self) -> Result<(), Self::Error>;
        async fn set_high(&mut self) -> Result<(), Self::Error>;

        #[inline]
        async fn set_state(&mut self, state: PinState) -> Result<(), Self::Error> {
            match state {
                PinState::Low  => self.set_low().await,
                PinState::High => self.set_high().await,
            }
        }
    }

    #[allow(async_fn_in_trait)]
    pub trait InputPin: ErrorType {
        async fn is_high(&mut self) -> Result<bool, Self::Error>;
        async fn is_low(&mut self) -> Result<bool, Self::Error>;
    }
}
