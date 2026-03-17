//! TCA9548A I2C multiplexer driver.
//!
//! [`Bus<BUS, N>`] wraps a shared I2C bus and automatically selects the
//! correct mux channel before every transaction.
//!
//! # Attribution
//! Architecture ported from
//! [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs).

use core::marker::ConstParamTy;

use embedded_hal_async::i2c::{ErrorType, I2c, Operation};

use super::SharedI2cBus;

/// One of the eight branches of the TCA9548A I2C multiplexer.
///
/// The discriminant matches the single-byte channel-select register value
/// sent to the mux (address 0x77 by default).
///
/// # Attribution
/// Ported from [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs).
#[derive(Debug, Copy, Clone, PartialEq, Eq, ConstParamTy)]
#[repr(u8)]
pub enum BusNumber {
    Bus0 = 0b00000001,
    Bus1 = 0b00000010,
    Bus2 = 0b00000100,
    Bus3 = 0b00001000,
    Bus4 = 0b00010000,
    Bus5 = 0b00100000,
    Bus6 = 0b01000000,
    Bus7 = 0b10000000,
}

/// A mux-aware I2C bus handle that selects channel `N` on the TCA9548A before
/// every transaction.
///
/// Wraps a `&'static SharedI2cBus<BUS>` (an Embassy async `Mutex`) so it is
/// safe to hand to multiple async tasks simultaneously.
///
/// # Attribution
/// Ported from [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs).
pub struct Bus<BUS: 'static, const N: BusNumber> {
    parent_bus: &'static SharedI2cBus<BUS>,
    mux_address: u8,
}

impl<BUS: 'static, const N: BusNumber> Bus<BUS, N> {
    pub fn new(bus: &'static SharedI2cBus<BUS>) -> Self {
        Self { parent_bus: bus, mux_address: 0x77 }
    }
}

impl<BUS, const N: BusNumber> ErrorType for Bus<BUS, N>
where
    BUS: ErrorType,
{
    type Error = BUS::Error;
}

impl<BUS, const N: BusNumber> I2c for Bus<BUS, N>
where
    BUS: I2c,
{
    #[inline]
    async fn read(&mut self, address: u8, read: &mut [u8]) -> Result<(), Self::Error> {
        let mut bus = self.parent_bus.lock().await;
        bus.write(self.mux_address, &[N as u8]).await?;
        log::debug!("TCA9548A: selected bus {}", N as u8);
        bus.read(address, read).await
    }

    #[inline]
    async fn write(&mut self, address: u8, write: &[u8]) -> Result<(), Self::Error> {
        let mut bus = self.parent_bus.lock().await;
        bus.write(self.mux_address, &[N as u8]).await?;
        log::debug!("TCA9548A: selected bus {}", N as u8);
        bus.write(address, write).await
    }

    #[inline]
    async fn write_read(
        &mut self,
        address: u8,
        write: &[u8],
        read: &mut [u8],
    ) -> Result<(), Self::Error> {
        let mut bus = self.parent_bus.lock().await;
        bus.write(self.mux_address, &[N as u8]).await?;
        log::debug!("TCA9548A: selected bus {}", N as u8);
        bus.write_read(address, write, read).await
    }

    #[inline]
    async fn transaction(
        &mut self,
        address: u8,
        operations: &mut [Operation<'_>],
    ) -> Result<(), Self::Error> {
        let mut bus = self.parent_bus.lock().await;
        bus.write(self.mux_address, &[N as u8]).await?;
        log::debug!("TCA9548A: selected bus {}", N as u8);
        bus.transaction(address, operations).await
    }
}
