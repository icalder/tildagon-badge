use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::{I2c, Config as I2cConfig};
use esp_hal::timer::timg::TimerGroup;
use esp_hal::delay::Delay;
use esp_hal::peripherals::{RMT, GPIO21};
use esp_hal::Blocking;
use embassy_time::{Duration, Timer};
use crate::Error;

pub struct TildagonHardware {
    pub i2c: I2c<'static, Blocking>,
    pub button_int: Input<'static>,
    pub rmt: RMT<'static>,
    pub led_pin: GPIO21<'static>,
}

impl TildagonHardware {
    pub async fn new(
        peripherals: esp_hal::peripherals::Peripherals,
    ) -> Result<Self, Error> {
        esp_println::logger::init_logger_from_env();
        let delay = Delay::new();

        let timg0 = TimerGroup::new(peripherals.TIMG0);
        esp_rtos::start(
            timg0.timer0,
            #[cfg(target_arch = "riscv32")]
            esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT),
        );

        // Tildagon Badge I2C Reset/Enable Pin (GPIO 9) - releases expanders from reset.
        let _i2c_reset = Output::new(peripherals.GPIO9, Level::High, OutputConfig::default());
        delay.delay_millis(5);

        // Tildagon Badge I2C initialization (SDA=GPIO45, SCL=GPIO46)
        let mut i2c = I2c::new(peripherals.I2C0, I2cConfig::default())?
            .with_sda(peripherals.GPIO45)
            .with_scl(peripherals.GPIO46);

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

        // Enable LED power: 0x5a pin 2 HIGH
        i2c.write(0x5au8, &[0x02, 1 << 2])?;

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
            peripherals.GPIO10,
            InputConfig::default().with_pull(Pull::Up),
        );

        // Re-enable button interrupts
        i2c.write(0x59u8, &[0x06, 0xF0])?;
        i2c.write(0x5au8, &[0x06, 0x3F])?;
        Timer::after(Duration::from_millis(10)).await;

        Ok(Self {
            i2c,
            button_int,
            rmt: peripherals.RMT,
            led_pin: peripherals.GPIO21,
        })
    }
}
