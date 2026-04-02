use esp_hal::{Blocking, gpio::Level, rmt::{Channel, PulseCode, Tx, TxChannelConfig, TxChannelCreator, Error as RmtError}};
use esp_hal::rmt::Rmt;
use esp_hal::time::Rate;
use esp_hal::peripherals::{RMT, GPIO21};
use smart_leds::{RGB8, colors};
use static_cell::StaticCell;
use crate::Error;
use crate::pins::LedPins;
use crate::i2c::SystemI2cBus;

#[derive(Debug, Copy, Clone)]
pub enum LedAdapterError {
    TransmissionError(RmtError),
}

pub const NUM_LEDS: usize = 19;
const PULSES_PER_LED: usize = 24; 
const RESET_PULSES: usize = 4;
const TX_BUFFER_SIZE: usize = NUM_LEDS * PULSES_PER_LED + RESET_PULSES + 1;

// WS2812B timing at 80 MHz APB clock (1 tick = 12.5 ns)
const T0H: u16 = 32; // 400 ns high
const T0L: u16 = 68; // 850 ns low
const T1H: u16 = 68; // 850 ns high
const T1L: u16 = 32; // 400 ns low

fn led_channel_config() -> TxChannelConfig {
    TxChannelConfig::default()
        .with_clk_divider(1)
        .with_idle_output_level(Level::Low)
        .with_carrier_modulation(false)
        .with_idle_output(true)
        .with_memsize(8)
}

fn encode_frame(
    iterator: impl Iterator<Item = RGB8>,
    buffer: &mut [PulseCode; TX_BUFFER_SIZE],
) {
    let mut idx = 0;
    for color in iterator {
        for byte in [color.g, color.r, color.b] {
            for shift in (0..8).rev() {
                let (high, low) = if (byte >> shift) & 1 == 1 {
                    (T1H, T1L)
                } else {
                    (T0H, T0L)
                };
                buffer[idx] = PulseCode::new(Level::High, high, Level::Low, low);
                idx += 1;
                if idx >= NUM_LEDS * PULSES_PER_LED {
                    for _ in 0..RESET_PULSES {
                        buffer[idx] = PulseCode::new(Level::Low, 12000, Level::Low, 12000);
                        idx += 1;
                    }
                    buffer[idx] = PulseCode::end_marker();
                    return;
                }
            }
        }
    }
    for _ in 0..RESET_PULSES {
        buffer[idx] = PulseCode::new(Level::Low, 12000, Level::Low, 12000);
        idx += 1;
    }
    buffer[idx] = PulseCode::end_marker();
}

pub struct Leds {
    channel: Option<Channel<'static, Blocking, Tx>>,
    buffer: &'static mut [PulseCode; TX_BUFFER_SIZE],
}

impl Leds {
    pub fn new(rmt_peripheral: RMT<'static>, led_pin: GPIO21<'static>) -> Self {
        let rmt = Rmt::new(rmt_peripheral, Rate::from_mhz(80)).unwrap();
        let channel = rmt.channel0.configure_tx(&led_channel_config()).unwrap().with_pin(led_pin);
        static RMT_BUFFER: StaticCell<[PulseCode; TX_BUFFER_SIZE]> = StaticCell::new();
        let buffer = RMT_BUFFER.init([PulseCode::end_marker(); TX_BUFFER_SIZE]);
        Self { channel: Some(channel), buffer }
    }

    pub async fn write<I>(&mut self, iterator: I) -> Result<(), Error>
    where
        I: Iterator<Item = RGB8>,
    {
        encode_frame(iterator, self.buffer);
        let len = self.buffer.iter().position(|p| p.is_end_marker()).unwrap_or(TX_BUFFER_SIZE - 1) + 1;
        let channel = self.channel.take().unwrap();
        match channel.transmit(&self.buffer[..len]) {
            Ok(transaction) => {
                match transaction.wait() {
                    Ok(chan) => {
                        self.channel = Some(chan);
                        Ok(())
                    }
                    Err((e, chan)) => {
                        self.channel = Some(chan);
                        Err(Error::Leds(LedAdapterError::TransmissionError(e)))
                    }
                }
            }
            Err((e, chan)) => {
                self.channel = Some(chan);
                Err(Error::Leds(LedAdapterError::TransmissionError(e)))
            }
        }
    }

    pub async fn clear(&mut self) -> Result<(), Error> {
        self.write([colors::BLACK; NUM_LEDS].iter().cloned()).await
    }
}

pub struct TypedLeds<BUS: 'static> {
    leds: Leds,
    power_pin: crate::pins::OutputPin<SystemI2cBus<BUS>>,
}

impl<BUS: 'static> TypedLeds<BUS>
where
    BUS: embedded_hal_async::i2c::I2c,
    crate::Error: From<BUS::Error>,
{
    pub async fn new(
        rmt_peripheral: RMT<'static>,
        led_data_pin: GPIO21<'static>,
        power_pin: LedPins,
        i2c_bus: SystemI2cBus<BUS>,
    ) -> Result<Self, Error> {
        let leds = Leds::new(rmt_peripheral, led_data_pin);
        let power_pin = power_pin.power_enable.into_output(i2c_bus).await?;
        let mut typed_leds = Self { leds, power_pin };
        typed_leds.set_power(true).await?;
        typed_leds.clear().await?;
        Ok(typed_leds)
    }

    pub async fn set_power(&mut self, enabled: bool) -> Result<(), Error> {
        use crate::pins::async_digital::OutputPin;
        if enabled {
            self.power_pin.set_high().await.map_err(|_| Error::Pins(embedded_hal::digital::ErrorKind::Other))
        } else {
            self.power_pin.set_low().await.map_err(|_| Error::Pins(embedded_hal::digital::ErrorKind::Other))
        }
    }

    pub async fn write<I>(&mut self, iterator: I) -> Result<(), Error>
    where
        I: Iterator<Item = RGB8>,
    {
        self.leds.write(iterator).await
    }

    pub async fn clear(&mut self) -> Result<(), Error> {
        self.leds.clear().await
    }
}
