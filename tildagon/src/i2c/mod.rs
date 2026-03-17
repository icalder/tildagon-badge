//! Shared I2C bus types and mux-aware channel handles.
//!
//! # Architecture
//!
//! ```text
//! SharedI2cBus<I2c>         (Mutex wrapping the raw ESP I2c peripheral)
//!        │
//!        ├─ SystemI2cBus    (TCA9548A channel 7 — expanders, BQ25895, FUSB302B)
//!        ├─ TopBoardI2cBus  (TCA9548A channel 0 — top-board peripherals)
//!        ├─ HexpansionAI2cBus … HexpansionFI2cBus  (channels 1–6)
//! ```
//!
//! Each named bus type atomically locks the parent `Mutex`, selects the correct
//! mux channel, performs the I2C transaction, and releases the lock — so tasks
//! using different branches of the mux cannot interfere with each other.
//!
//! # Attribution
//! Architecture ported from
//! [tildagon-rs by Dan Nixon](https://github.com/DanNixon/tildagon-rs).

pub mod tca9548a;

pub use tca9548a::{Bus, BusNumber};

/// The raw mutex type used to protect the shared I2C bus.
pub type SharingRawMutex = embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

/// A shared, async-mutex-protected I2C bus.
///
/// Wrap your raw I2C peripheral in this before constructing mux-channel buses.
pub type SharedI2cBus<BUS> = embassy_sync::mutex::Mutex<SharingRawMutex, BUS>;

// ── Per-channel type aliases ──────────────────────────────────────────────────

/// TCA9548A channel 7 — system bus (AW9523B expanders, BQ25895, FUSB302B).
pub type SystemI2cBus<BUS> = tca9548a::Bus<BUS, { BusNumber::Bus7 }>;

/// TCA9548A channel 0 — top-board / USB-out bus.
pub type TopBoardI2cBus<BUS> = tca9548a::Bus<BUS, { BusNumber::Bus0 }>;

/// TCA9548A channel 1 — hexpansion slot A.
pub type HexpansionAI2cBus<BUS> = tca9548a::Bus<BUS, { BusNumber::Bus1 }>;
/// TCA9548A channel 2 — hexpansion slot B.
pub type HexpansionBI2cBus<BUS> = tca9548a::Bus<BUS, { BusNumber::Bus2 }>;
/// TCA9548A channel 3 — hexpansion slot C.
pub type HexpansionCI2cBus<BUS> = tca9548a::Bus<BUS, { BusNumber::Bus3 }>;
/// TCA9548A channel 4 — hexpansion slot D.
pub type HexpansionDI2cBus<BUS> = tca9548a::Bus<BUS, { BusNumber::Bus4 }>;
/// TCA9548A channel 5 — hexpansion slot E.
pub type HexpansionEI2cBus<BUS> = tca9548a::Bus<BUS, { BusNumber::Bus5 }>;
/// TCA9548A channel 6 — hexpansion slot F.
pub type HexpansionFI2cBus<BUS> = tca9548a::Bus<BUS, { BusNumber::Bus6 }>;

// ── Factory helpers ───────────────────────────────────────────────────────────

/// Create a [`SystemI2cBus`] handle from a `&'static SharedI2cBus`.
pub fn system_i2c_bus<BUS>(bus: &'static SharedI2cBus<BUS>) -> SystemI2cBus<BUS> {
    SystemI2cBus::new(bus)
}

/// Create a [`TopBoardI2cBus`] handle from a `&'static SharedI2cBus`.
pub fn top_i2c_bus<BUS>(bus: &'static SharedI2cBus<BUS>) -> TopBoardI2cBus<BUS> {
    TopBoardI2cBus::new(bus)
}

/// Create a [`HexpansionAI2cBus`] handle.
pub fn hexpansion_a_i2c_bus<BUS>(bus: &'static SharedI2cBus<BUS>) -> HexpansionAI2cBus<BUS> {
    HexpansionAI2cBus::new(bus)
}
/// Create a [`HexpansionBI2cBus`] handle.
pub fn hexpansion_b_i2c_bus<BUS>(bus: &'static SharedI2cBus<BUS>) -> HexpansionBI2cBus<BUS> {
    HexpansionBI2cBus::new(bus)
}
/// Create a [`HexpansionCI2cBus`] handle.
pub fn hexpansion_c_i2c_bus<BUS>(bus: &'static SharedI2cBus<BUS>) -> HexpansionCI2cBus<BUS> {
    HexpansionCI2cBus::new(bus)
}
/// Create a [`HexpansionDI2cBus`] handle.
pub fn hexpansion_d_i2c_bus<BUS>(bus: &'static SharedI2cBus<BUS>) -> HexpansionDI2cBus<BUS> {
    HexpansionDI2cBus::new(bus)
}
/// Create a [`HexpansionEI2cBus`] handle.
pub fn hexpansion_e_i2c_bus<BUS>(bus: &'static SharedI2cBus<BUS>) -> HexpansionEI2cBus<BUS> {
    HexpansionEI2cBus::new(bus)
}
/// Create a [`HexpansionFI2cBus`] handle.
pub fn hexpansion_f_i2c_bus<BUS>(bus: &'static SharedI2cBus<BUS>) -> HexpansionFI2cBus<BUS> {
    HexpansionFI2cBus::new(bus)
}
