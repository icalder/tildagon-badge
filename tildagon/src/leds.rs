use esp_hal::rmt::Rmt;
use esp_hal::time::Rate;
use esp_hal::peripherals::{RMT, GPIO21};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async};
use smart_leds::{SmartLedsWriteAsync, colors};
use static_cell::StaticCell;
use crate::Error;

pub const NUM_LEDS: usize = 19;
const BUFFER_SIZE: usize = buffer_size_async(NUM_LEDS);

pub struct Leds {
    adapter: SmartLedsAdapterAsync<'static, BUFFER_SIZE>,
}

impl Leds {
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

    pub async fn write<I>(&mut self, iterator: I) -> Result<(), Error>
    where
        I: Iterator<Item = smart_leds::RGB8>,
    {
        self.adapter.write(iterator).await.map_err(Error::Leds)
    }

    pub async fn clear(&mut self) -> Result<(), Error> {
        let black = [colors::BLACK; NUM_LEDS];
        self.write(black.iter().cloned()).await
    }
}
