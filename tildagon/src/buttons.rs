use esp_hal::gpio::Input;
use esp_hal::i2c::master::I2c;
use esp_hal::Blocking;
use embassy_time::{Duration, Timer};
use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    A, B, C, D, E, F
}

pub struct Buttons {
    last_state_59: u8,
    last_state_5a: u8,
}

impl Buttons {
    pub fn new() -> Self {
        Self {
            last_state_59: 0xFF,
            last_state_5a: 0xFF,
        }
    }

    pub async fn wait_for_press(
        &mut self,
        i2c: &mut I2c<'_, Blocking>,
        button_int: &mut Input<'_>,
    ) -> Result<Option<Button>, Error> {
        loop {
            button_int.wait_for_falling_edge().await;

            if button_int.is_high() {
                continue;
            }

            Timer::after(Duration::from_millis(20)).await;
            if button_int.is_high() {
                continue;
            }

            let mut port0_58 = [0u8; 1];
            let mut port0_59 = [self.last_state_59; 1];
            let mut port1_59 = [0u8; 1];
            let mut port0_5a = [self.last_state_5a; 1];
            let mut port1_5a = [0u8; 1];
            let mut dummy = [0u8; 2];

            // 1. Check chips on Port 7 (System)
            i2c.write(0x77u8, &[1 << 7])?;
            if button_int.is_low() {
                i2c.write_read(0x58u8, &[0x00], &mut port0_58)?;
                i2c.write_read(0x58u8, &[0x01], &mut port0_58)?;
            }
            if button_int.is_low() {
                i2c.write_read(0x59u8, &[0x00], &mut port0_59)?;
                i2c.write_read(0x59u8, &[0x01], &mut port1_59)?;
            }
            if button_int.is_low() {
                i2c.write_read(0x5au8, &[0x00], &mut port0_5a)?;
                i2c.write_read(0x5au8, &[0x01], &mut port1_5a)?;
            }
            if button_int.is_low() {
                i2c.write_read(0x6Au8, &[0x0B], &mut dummy)?;
                i2c.write_read(0x22u8, &[0x3E], &mut dummy)?;
                i2c.write_read(0x22u8, &[0x42], &mut dummy[..1])?;
            }

            // 2. Check chips on Port 0 (USB Out)
            if button_int.is_low() {
                i2c.write(0x77u8, &[1 << 0])?;
                i2c.write_read(0x22u8, &[0x3E], &mut dummy)?;
                i2c.write_read(0x22u8, &[0x42], &mut dummy[..1])?;
                i2c.write(0x77u8, &[1 << 7])?;
            }

            let changed_59 = port0_59[0] ^ self.last_state_59;
            let pressed_59 = !port0_59[0] & changed_59;
            
            let changed_5a = port0_5a[0] ^ self.last_state_5a;
            let pressed_5a = !port0_5a[0] & changed_5a;

            self.last_state_59 = port0_59[0];
            self.last_state_5a = port0_5a[0];

            if pressed_59 & (1 << 0) != 0 {
                return Ok(Some(Button::C));
            }
            if pressed_59 & (1 << 1) != 0 {
                return Ok(Some(Button::D));
            }
            if pressed_59 & (1 << 2) != 0 {
                return Ok(Some(Button::E));
            }
            if pressed_59 & (1 << 3) != 0 {
                return Ok(Some(Button::F));
            }
            if pressed_5a & (1 << 6) != 0 {
                return Ok(Some(Button::A));
            }
            if pressed_5a & (1 << 7) != 0 {
                return Ok(Some(Button::B));
            }

            if button_int.is_low() {
                Timer::after(Duration::from_millis(10)).await;
            }
        }
    }
}
