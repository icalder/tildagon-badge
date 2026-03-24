use esp_hal::rmt::Rmt;
use esp_hal::time::Rate;
use esp_hal::peripherals::{RMT, GPIO21};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async};
use smart_leds::{SmartLedsWriteAsync, colors};
use static_cell::StaticCell;
use crate::Error;
use crate::pins::LedPins;
use crate::i2c::SystemI2cBus;

pub const NUM_LEDS: usize = 19;
const BUFFER_SIZE: usize = buffer_size_async(NUM_LEDS);

/// WS2812B LED ring driver using the ESP32-S3 RMT peripheral.
///
/// # Compatibility Baseline (Phase 0)
/// [`Leds::new`], [`Leds::write`], and [`Leds::clear`] are the stable surface
/// consumed by `embassy_blinky`. The constructor takes ownership of `RMT` and
/// `GPIO21` directly from `TildagonHardware` and must keep that shape through
/// Phases 1-3.
pub struct Leds {
    adapter: SmartLedsAdapterAsync<'static, BUFFER_SIZE>,
}

impl Leds {
    /// Initialise the RMT-backed LED adapter.
    ///
    /// Takes ownership of `RMT<'static>` and `GPIO21<'static>` (obtained from
    /// [`TildagonHardware`](crate::hardware::TildagonHardware)'s fields).
    ///
    /// # Compatibility Baseline (Phase 0)
    pub fn new(rmt_peripheral: RMT<'static>, led_pin: GPIO21<'static>) -> Self {
        let rmt = Rmt::new(rmt_peripheral, Rate::from_mhz(80))
            .unwrap()
            .into_async();
        let rmt_channel = rmt.channel0;

        static RMT_BUFFER: StaticCell<[esp_hal::rmt::PulseCode; BUFFER_SIZE]> = StaticCell::new();
        let rmt_buffer = RMT_BUFFER.init([esp_hal::rmt::PulseCode::default(); BUFFER_SIZE]);

        let adapter = SmartLedsAdapterAsync::new(rmt_channel, led_pin, rmt_buffer);

        Self { adapter }
    }

    /// Write a sequence of RGB values to all LEDs.
    ///
    /// # Compatibility Baseline (Phase 0)
    pub async fn write<I>(&mut self, iterator: I) -> Result<(), Error>
    where
        I: Iterator<Item = smart_leds::RGB8>,
    {
        self.adapter.write(iterator).await.map_err(Error::Leds)
    }

    /// Turn all LEDs off.
    ///
    /// # Compatibility Baseline (Phase 0)
    pub async fn clear(&mut self) -> Result<(), Error> {
        let black = [colors::BLACK; NUM_LEDS];
        self.write(black.iter().cloned()).await
    }
}

/// A more ergonomic LED driver that manages its own power pin.
///
/// # Attribution
/// Architecture ported from
/// [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs).
pub struct TypedLeds<BUS: 'static> {
    adapter: SmartLedsAdapterAsync<'static, BUFFER_SIZE>,
    power_pin: crate::pins::OutputPin<SystemI2cBus<BUS>>,
}

impl<BUS: 'static> TypedLeds<BUS>
where
    BUS: embedded_hal_async::i2c::I2c,
    crate::Error: From<BUS::Error>,
{
    /// Create a new `TypedLeds` handle.
    ///
    /// Takes ownership of the RMT peripheral, the data pin, and the I2C power pin.
    pub async fn new(
        rmt_peripheral: RMT<'static>,
        led_data_pin: GPIO21<'static>,
        power_pin: LedPins,
        i2c_bus: SystemI2cBus<BUS>,
    ) -> Result<Self, Error> {
        let rmt = Rmt::new(rmt_peripheral, Rate::from_mhz(80))
            .unwrap()
            .into_async();
        let rmt_channel = rmt.channel0;

        static RMT_BUFFER: StaticCell<[esp_hal::rmt::PulseCode; BUFFER_SIZE]> = StaticCell::new();
        let rmt_buffer = RMT_BUFFER.init([esp_hal::rmt::PulseCode::default(); BUFFER_SIZE]);

        let adapter = SmartLedsAdapterAsync::new(rmt_channel, led_data_pin, rmt_buffer);
        let power_pin = power_pin.power_enable.into_output(i2c_bus).await?;

        Ok(Self { adapter, power_pin })
    }

    /// Enable or disable power to the LED ring.
    pub async fn set_power(&mut self, enabled: bool) -> Result<(), Error> {
        use crate::pins::async_digital::OutputPin;
        if enabled {
            self.power_pin.set_high().await.map_err(|_| Error::Pins(embedded_hal::digital::ErrorKind::Other))
        } else {
            self.power_pin.set_low().await.map_err(|_| Error::Pins(embedded_hal::digital::ErrorKind::Other))
        }
    }

    /// Write a sequence of RGB values to all LEDs.
    pub async fn write<I>(&mut self, iterator: I) -> Result<(), Error>
    where
        I: Iterator<Item = smart_leds::RGB8>,
    {
        self.adapter.write(iterator).await.map_err(Error::Leds)
    }

    /// Turn all LEDs off.
    pub async fn clear(&mut self) -> Result<(), Error> {
        let black = [colors::BLACK; NUM_LEDS];
        self.write(black.iter().cloned()).await
    }
}
