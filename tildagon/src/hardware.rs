use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::i2c::master::I2c;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::delay::Delay;
use esp_hal::peripherals::{GPIO21, RMT};
use esp_hal::Blocking;
use embassy_time::{Duration, Timer};
use crate::Error;
use crate::display::{DisplayInitError, TildagonDisplay};
use crate::pins::{Pins, pin::PinExt};

/// Central hardware handle for the Tildagon badge.
///
/// This type owns the non-radio badge peripherals and, when the `radio`
/// feature is enabled, can later hand the WiFi/BLE peripherals to
/// [`crate::radio::TildagonRadio`].
pub struct TildagonHardware {
    pub i2c: I2c<'static, Blocking>,
    pub rmt: RMT<'static>,
    pub led_data_pin: GPIO21<'static>,
    top_board: Option<crate::resources::TopBoardResources<'static>>,
    display: Option<crate::resources::DisplayResources<'static>>,
    #[cfg(feature = "radio")]
    radio_res: Option<crate::resources::RadioResources<'static>>,
}

impl TildagonHardware {
    /// Initialise all Tildagon badge hardware.
    ///
    /// Performs, in order:
    /// 1. Embassy/timer setup via `esp_rtos::start`.
    /// 2. Peripherals are split into typed resource groups via [`split_resources!`].
    /// 3. I2C bus init (SDA=GPIO45, SCL=GPIO46).
    /// 4. **Secure USB Serial** — drives 0x5a pin 4 LOW immediately.
    /// 5. Basic peripheral initialization (Charger, LED power).
    /// 6. Disables all interrupts on GPIO expanders (polling-only mode).
    pub async fn new(
        peripherals: esp_hal::peripherals::Peripherals,
    ) -> Result<Self, Error> {
        esp_println::logger::init_logger_from_env();
        let delay = Delay::new();

        let timg0 = TimerGroup::new(peripherals.TIMG0);
        let sw_interrupt = esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
        esp_rtos::start(
            timg0.timer0,
            sw_interrupt.software_interrupt0,
        );

        let i2c_res = crate::resources::I2cResources {
            sda:   peripherals.GPIO45,
            scl:   peripherals.GPIO46,
            i2c:   peripherals.I2C0,
            reset: peripherals.GPIO9,
        };
        let led_res = crate::resources::LedResources {
            data: peripherals.GPIO21,
            rmt:  peripherals.RMT,
        };
        let top_board_res = crate::resources::TopBoardResources {
            hs_1: peripherals.GPIO8,
            hs_2: peripherals.GPIO7,
            hs_3: peripherals.GPIO2,
            hs_4: peripherals.GPIO1,
        };
        let display_res = crate::resources::DisplayResources {
            spi: peripherals.SPI2,
            dma: peripherals.DMA_CH0,
        };

        // Tildagon Badge I2C Reset/Enable Pin (GPIO 9) - releases expanders from reset.
        let _i2c_reset = Output::new(i2c_res.reset, Level::High, OutputConfig::default());
        delay.delay_millis(5);

        // Tildagon Badge I2C initialization (SDA=GPIO45, SCL=GPIO46)
        let i2c_config = esp_hal::i2c::master::Config::default()
            .with_timeout(esp_hal::i2c::master::BusTimeout::Maximum);
        let mut i2c = I2c::new(i2c_res.i2c, i2c_config)?
            .with_sda(i2c_res.sda)
            .with_scl(i2c_res.scl);

        let pins = Pins::new();

        // Enable I2C Mux Channel 7 (System Bus)
        i2c.write(0x77u8, &[1 << 7])?;

        // --- CRITICAL: SECURE USB SERIAL IMMEDIATELY ---
        // 0x5a AW9523B pin 4 controls the USB mux. Drive it LOW immediately.
        i2c.write(0x5au8, &[0x02, 0x00])?;
        // Set direction: pins 2, 4, 5 as outputs (bit = 0), rest as inputs (bit = 1)
        i2c.write(0x5au8, &[0x04, 0xCB, 0xFF])?;

        // Initialize Battery Charger
        crate::battery::init_bq25895(&mut i2c).await?;

        // Initialize GPIO Expanders (0x58, 0x59, 0x5a)
        for addr in [0x58u8, 0x59u8, 0x5au8] {
            if addr != 0x5au8 {
                i2c.write(addr, &[0x7F, 0x00])?; // Soft Reset
                Timer::after(Duration::from_millis(2)).await;
            }
            i2c.write(addr, &[0x06, 0xFF, 0xFF])?; // Mask all interrupts (Port 0 & 1)
            
            // Set direction: pins 2, 4, 5 as outputs (bit = 0) on 0x5a, rest as inputs (bit = 1)
            let dir_p0 = if addr == 0x5au8 { 0xCB } else { 0xFF };
            i2c.write(addr, &[0x04, dir_p0, 0xFF])?; 

            i2c.write(addr, &[0x11, 0x10])?;       // Push-pull output mode
        }

        // Enable LED power (Pin 2 on 0x5a) while keeping Pin 4 and 5 LOW
        i2c.write(0x5au8, &[0x02, pins.led.power_enable.bit()])?;

        Ok(Self {
            i2c,
            rmt: led_res.rmt,
            led_data_pin: led_res.data,
            top_board: Some(top_board_res),
            display: Some(display_res),
            #[cfg(feature = "radio")]
            radio_res: Some(crate::resources::RadioResources {
                wifi: peripherals.WIFI,
                bt:   peripherals.BT,
            }),
        })
    }

    /// Initialize the badge display while retaining access to other hardware resources.
    pub fn init_display<'a>(
        &mut self,
        buffer: &'a mut [u8],
    ) -> Result<TildagonDisplay<'a>, DisplayInitError> {
        let top_board = self
            .top_board
            .take()
            .ok_or(DisplayInitError::ResourcesUnavailable)?;
        let display = self
            .display
            .take()
            .ok_or(DisplayInitError::ResourcesUnavailable)?;

        crate::display::init(top_board, display, buffer)
    }

    /// Initialize the shared radio handle and take ownership of the WiFi/BLE peripherals.
    #[cfg(feature = "radio")]
    pub fn init_radio(&mut self) -> Result<crate::radio::TildagonRadio, Error> {
        crate::radio::init_radio_heap_once();
        let radio_res = self.radio_res.take().ok_or(Error::RadioUnavailable)?;
        Ok(crate::radio::TildagonRadio::new(radio_res))
    }

    /// Start the background button polling service.
    ///
    /// This spawns a high-priority task that polls the button expanders every 20ms
    /// and broadcasts events to all subscribers. This is much more reliable than
    /// interrupt-driven reads during radio activity.
    pub fn init_button_manager(
        spawner: &embassy_executor::Spawner,
        shared_i2c: &'static crate::i2c::SharedI2cBus<esp_hal::i2c::master::I2c<'static, esp_hal::Async>>,
    ) -> crate::buttons::ButtonManager {
        let buttons = crate::buttons::TypedButtons::new(
            crate::i2c::system_i2c_bus(shared_i2c),
        );

        spawner.spawn(crate::buttons::button_manager_task(buttons).unwrap());

        crate::buttons::ButtonManager
    }
}
