#![no_std]
// In no_std environments, standard cargo tests try to link the 'test' crate (part of std).
// We disable the main entry point for tests to avoid conflicts and satisfy the LSP/compiler
// when checking the library as a test target (which the compiler treats as an executable).
#![cfg_attr(test, no_main)]

pub mod hardware;
pub mod leds;
pub mod buttons;
pub mod error;

pub use error::Error;

// When building for tests (e.g., during cargo check --all-targets or LSP analysis),
// we provide a dummy main and link esp_backtrace to satisfy the requirement for a 
// panic handler and entry point when the library is compiled as a test executable.
#[cfg(test)]
#[unsafe(no_mangle)]
fn main() -> ! {
    loop {}
}

#[cfg(test)]
use esp_backtrace as _;
