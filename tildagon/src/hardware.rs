use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::{I2c, Config as I2cConfig};
use esp_hal::timer::timg::TimerGroup;
use esp_hal::delay::Delay;
use esp_hal::peripherals::{RMT, GPIO21};
use esp_hal::Blocking;
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;
use crate::Error;
use crate::pins::{Pins, pin::PinExt};

static RADIO_CELL: StaticCell<esp_radio::Controller<'static>> = StaticCell::new();

/// Central hardware handle for the Tildagon badge.
///
/// # Compatibility Baseline (Phase 0)
///
/// The following public fields and constructor are the stable integration surface
/// used by `embassy_blinky`. They **must not change shape** during the refactor
/// (Phases 1-3). New APIs may be added alongside them, but these entry points
/// must keep compiling and behaving identically until Phase 4 opts into migration.
///
/// Stable surface:
/// - [`TildagonHardware::new`]
/// - [`TildagonHardware::i2c`]
/// - [`TildagonHardware::button_int`]
/// - [`TildagonHardware::rmt`]
/// - [`TildagonHardware::led_pin`]
pub struct TildagonHardware {
    pub i2c: I2c<'static, Blocking>,
    pub button_int: Input<'static>,
    pub rmt: RMT<'static>,
    pub led_pin: GPIO21<'static>,
    pub top_board: crate::resources::TopBoardResources<'static>,
    pub display: crate::resources::DisplayResources<'static>,
    pub radio: &'static esp_radio::Controller<'static>,
    pub radio_res: crate::resources::RadioResources<'static>,
}

impl TildagonHardware {
    /// Initialise all Tildagon badge hardware.
    ///
    /// Performs, in order:
    /// 1. Embassy/timer setup via `esp_rtos::start`.
    /// 2. Peripherals are split into typed resource groups via [`split_resources!`].
    /// 3. I2C bus init (SDA=GPIO45, SCL=GPIO46) using [`crate::resources::I2cResources`].
    /// 4. **Secure USB Serial** — drives 0x5a pin 4 LOW immediately.
    /// 5. **Silence pulsing interrupts** — configures FUSB302B (mux port 0) and
    ///    BQ25895 + FUSB302B (mux port 7) so they stop toggling the INT line.
    /// 6. Button-expander setup (0x58, 0x59, 0x5a) and LED-power enable.
    /// 7. Clears all pending interrupts, then re-enables button interrupts.
    ///
    /// # Compatibility Baseline (Phase 0)
    /// This signature (`async fn new(Peripherals) -> Result<Self, Error>`) is
    /// the stable entry point consumed by `embassy_blinky::main`. It must not
    /// change until Phase 4.
    pub async fn new(
        peripherals: esp_hal::peripherals::Peripherals,
    ) -> Result<Self, Error> {
        esp_println::logger::init_logger_from_env();
        let delay = Delay::new();

        // Move TIMG0 and SW_INTERRUPT out before the resource split so they
        // remain accessible as bare fields on the partially-moved `peripherals`.
        let timg0 = TimerGroup::new(peripherals.TIMG0);
        esp_rtos::start(
            timg0.timer0,
            #[cfg(target_arch = "riscv32")]
            esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT),
        );

        // Split peripherals into typed resource groups by constructing the resource
        // structs directly. This is equivalent to calling `split_resources!(peripherals)`
        // from downstream code, but avoids the path-resolution restriction on
        // macro-expanded #[macro_export] macros within the same crate.
        //
        // Attribution: resource-splitting pattern from tildagon-rs by Dan Nixon
        // (https://github.com/DanNixon/tildagon-rs).
        let i2c_res = crate::resources::I2cResources {
            sda:   peripherals.GPIO45,
            scl:   peripherals.GPIO46,
            i2c:   peripherals.I2C0,
            reset: peripherals.GPIO9,
        };
        let system_res = crate::resources::SystemResources {
            int: peripherals.GPIO10,
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
        let radio_res = crate::resources::RadioResources {
            wifi: peripherals.WIFI,
            bt:   peripherals.BT,
            rng:  peripherals.RNG,
            timer: peripherals.TIMG1,
        };
        
        let radio = RADIO_CELL.init(esp_radio::init().map_err(Error::Radio)?);

        // Tildagon Badge I2C Reset/Enable Pin (GPIO 9) - releases expanders from reset.
        // _i2c_reset is kept alive until end of fn to hold the pin HIGH throughout init.
        let _i2c_reset = Output::new(i2c_res.reset, Level::High, OutputConfig::default());
        delay.delay_millis(5);

        // Tildagon Badge I2C initialization (SDA=GPIO45, SCL=GPIO46)
        let mut i2c = I2c::new(i2c_res.i2c, I2cConfig::default())?
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

        // --- SILENCE PULSING INTERRUPTS ---
        let mut dummy = [0u8; 2];

        // 1. Silence FUSB302B on Mux Port 0 (usb_out)
        i2c.write(0x77u8, &[1 << 0])?;
        Timer::after(Duration::from_millis(2)).await;
        i2c.write(0x22u8, &[0x0C, 0x01])?;
        Timer::after(Duration::from_millis(2)).await;
        i2c.write(0x22u8, &[0x0B, 0x00])?;
        i2c.write(0x22u8, &[0x04, 0x02])?;
        i2c.write(0x22u8, &[0x0A, 0xFF])?;
        i2c.write(0x22u8, &[0x0E, 0xFF])?;
        i2c.write(0x22u8, &[0x0F, 0xFF])?;
        i2c.write_read(0x22u8, &[0x3E], &mut dummy)?;
        i2c.write_read(0x22u8, &[0x42], &mut dummy[..1])?;

        // 2. Silence BQ25895 and FUSB302B on Mux Port 7 (system)
        i2c.write(0x77u8, &[1 << 7])?;
        Timer::after(Duration::from_millis(2)).await;

        i2c.write(0x6Au8, &[0x14, 0x80])?;
        Timer::after(Duration::from_millis(10)).await;
        i2c.write(0x6Au8, &[0x02, 0x00])?;
        i2c.write(0x6Au8, &[0x07, 0x8C])?;
        i2c.write_read(0x6Au8, &[0x0B], &mut dummy)?;

        i2c.write(0x22u8, &[0x0C, 0x01])?;
        Timer::after(Duration::from_millis(2)).await;
        i2c.write(0x22u8, &[0x0B, 0x00])?;
        i2c.write(0x22u8, &[0x04, 0x02])?;
        i2c.write(0x22u8, &[0x0A, 0xFF])?;
        i2c.write(0x22u8, &[0x0E, 0xFF])?;
        i2c.write(0x22u8, &[0x0F, 0xFF])?;
        i2c.write_read(0x22u8, &[0x3E], &mut dummy)?;
        i2c.write_read(0x22u8, &[0x42], &mut dummy[..1])?;

        // Full init for 0x58 and 0x59
        for addr in [0x58u8, 0x59u8] {
            i2c.write(addr, &[0x7F, 0x00])?;
            Timer::after(Duration::from_millis(2)).await;
            i2c.write(addr, &[0x06, 0xFF, 0xFF])?;
            i2c.write(addr, &[0x04, 0xFF, 0xFF])?;
            i2c.write(addr, &[0x11, 0x10])?;
        }

        // Additional 0x5a config
        i2c.write(0x5au8, &[0x06, 0xFF, 0xFF])?;
        i2c.write(0x5au8, &[0x11, 0x10])?;
        Timer::after(Duration::from_millis(2)).await;

        // Enable LED power using typed pin info for clarity.
        // ws2812_power_en is 0x5a port 0 pin 2.
        i2c.write(pins.led.power_enable.address(), &[0x02, pins.led.power_enable.bit()])?;

        // Clear ALL pending interrupts
        {
            let mut d = [0u8; 1];
            for addr in [0x58u8, 0x59u8, 0x5au8] {
                i2c.write_read(addr, &[0x00], &mut d)?;
                i2c.write_read(addr, &[0x01], &mut d)?;
            }
            let mut fus2 = [0u8; 2];
            i2c.write_read(0x22u8, &[0x3E], &mut fus2)?;
            i2c.write_read(0x22u8, &[0x42], &mut fus2[..1])?;
            i2c.write_read(0x22u8, &[0x40], &mut fus2[..1])?;
        }

        let button_int = Input::new(
            system_res.int,
            InputConfig::default().with_pull(Pull::Up),
        );

        // Re-enable button interrupts
        i2c.write(0x59u8, &[0x06, 0xF0])?;
        i2c.write(0x5au8, &[0x06, 0x3F])?;
        Timer::after(Duration::from_millis(10)).await;

        Ok(Self {
            i2c,
            button_int,
            rmt: led_res.rmt,
            led_pin: led_res.data,
            top_board: top_board_res,
            display: display_res,
            radio,
            radio_res,
        })
    }
}
