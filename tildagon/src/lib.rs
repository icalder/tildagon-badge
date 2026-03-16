#![no_std]
#![cfg_attr(test, no_main)]

pub mod hardware;
pub mod leds;
pub mod buttons;
pub mod error;

pub use error::Error;

#[cfg(test)]
#[unsafe(no_mangle)]
fn main() -> ! {
    loop {}
}

#[cfg(test)]
use esp_backtrace as _;
