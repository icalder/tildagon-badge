//! Battery and charger access for the Tildagon badge.
//!
//! The 2024 badge uses a BQ25895 charger on the system I2C bus (mux channel 7).
//! This chip provides charger status plus ADC readings for VBAT, VSYS, VBUS, and
//! charging current. It is not a true fuel gauge, so the exposed percentage is an
//! estimate derived from the original badge firmware.

use embedded_hal_async::i2c::I2c as _;
use embassy_time::{Duration, Timer};

use crate::{Error, i2c::SystemI2cBus};

const ADDRESS: u8 = 0x6A;
const STATUS_START_REGISTER: u8 = 0x0B;
const STATUS_BLOCK_LEN: usize = 8;
const CHARGE_STATUS_MASK: u8 = 0x18;
const INPUT_SOURCE_CONTROL_REGISTER: u8 = 0x00;
const POWER_ON_CONFIG_REGISTER: u8 = 0x03;
const RESET_REGISTER: u8 = 0x14;
const RESET_REGISTER_VALUE: u8 = 0x80;
const CONFIG_START_REGISTER: u8 = 0x02;
const CONTROL_REGISTER: u8 = 0x07;
const CONTROL_REGISTER_VALUE: u8 = 0x8C;
const MISC_OPERATION_REGISTER: u8 = 0x09;
const BATFET_DISABLE_MASK: u8 = 0x20;
const BATFET_IMMEDIATE_MASK: u8 = 0x08;
const INPUT_HIZ_MASK: u8 = 0x80;
const OTG_BOOST_MASK: u8 = 0x20;

const VBAT_DISCHARGING_MAX: f32 = 4.14;
const VBAT_DISCHARGING_MIN: f32 = 3.5;
const VBAT_CHARGING_MAX: f32 = 4.2;
const VBAT_CHARGING_MIN: f32 = 3.6;
const CHARGE_TERMINATION_CURRENT_AMPS: f32 = 0.064;
const DEFAULT_CHARGE_MAX_CURRENT_AMPS: f32 = 1.536;
const DEFAULT_CHARGE_CV_PERCENT: f32 = 20.0;

/// High-level charging state reported by the BQ25895.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeStatus {
    NotCharging,
    PreCharging,
    FastCharging,
    ChargeTerminated,
    Unknown(u8),
}

impl ChargeStatus {
    fn from_status_register(status: u8) -> Self {
        match status & CHARGE_STATUS_MASK {
            0x00 => Self::NotCharging,
            0x08 => Self::PreCharging,
            0x10 => Self::FastCharging,
            0x18 => Self::ChargeTerminated,
            other => Self::Unknown(other),
        }
    }

    /// Returns `true` while the charger is actively charging the cell.
    pub fn is_charging(self) -> bool {
        matches!(self, Self::PreCharging | Self::FastCharging)
    }

    /// User-facing label suitable for logging or UI.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotCharging => "Not charging",
            Self::PreCharging => "Pre-charging",
            Self::FastCharging => "Charging",
            Self::ChargeTerminated => "Charged",
            Self::Unknown(_) => "Unknown",
        }
    }
}

/// Parsed charger and battery measurements.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatteryState {
    /// Raw charger status register (`0x0B`).
    pub raw_status: u8,
    /// Raw charger fault register (`0x0C`).
    pub raw_fault: u8,
    /// USB input voltage in volts.
    pub vbus_volts: f32,
    /// System rail voltage in volts.
    pub vsys_volts: f32,
    /// Battery voltage in volts.
    pub vbat_volts: f32,
    /// Charging current in amps.
    pub charge_current_amps: f32,
    /// Decoded charge state.
    pub charge_status: ChargeStatus,
}

/// Raw PMIC register snapshot useful for debugging power-path issues.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatteryDiagnostics {
    pub state: BatteryState,
    pub reg00_input_source: u8,
    pub reg03_power_on_config: u8,
    pub reg07_charge_timer: u8,
    pub reg09_misc_operation: u8,
}

impl BatteryDiagnostics {
    /// Returns true if input Hi-Z mode is enabled.
    pub fn input_hiz_enabled(&self) -> bool {
        self.reg00_input_source & INPUT_HIZ_MASK != 0
    }

    /// Returns true if OTG boost mode is enabled.
    pub fn boost_enabled(&self) -> bool {
        self.reg03_power_on_config & OTG_BOOST_MASK != 0
    }

    /// Returns true if the battery FET has been disabled.
    pub fn batfet_disabled(&self) -> bool {
        self.reg09_misc_operation & BATFET_DISABLE_MASK != 0
    }
}

impl BatteryState {
    fn from_registers(registers: [u8; STATUS_BLOCK_LEN]) -> Self {
        Self {
            raw_status: registers[0],
            raw_fault: registers[1],
            charge_status: ChargeStatus::from_status_register(registers[0]),
            vbat_volts: scaled_measurement(registers[3], 0.02, 2.304),
            vsys_volts: scaled_measurement(registers[4], 0.02, 2.304),
            vbus_volts: scaled_measurement(registers[6], 0.10, 2.6),
            charge_current_amps: (registers[7] & 0x7F) as f32 * 0.05,
        }
    }

    /// Estimated battery level percentage.
    ///
    /// This is derived from the original badge firmware's voltage/current based
    /// heuristic. The BQ25895 is a charger with ADCs, not a true coulomb counter.
    pub fn estimated_level_percent(&self) -> u8 {
        (self.estimated_level_percent_f32() + 0.5) as u8
    }

    /// Floating-point variant of [`Self::estimated_level_percent`].
    pub fn estimated_level_percent_f32(&self) -> f32 {
        let level = if matches!(
            self.charge_status,
            ChargeStatus::NotCharging | ChargeStatus::ChargeTerminated
        ) {
            ((self.vbat_volts - VBAT_DISCHARGING_MIN)
                / (VBAT_DISCHARGING_MAX - VBAT_DISCHARGING_MIN))
                * 100.0
        } else if self.vbat_volts < VBAT_CHARGING_MAX {
            let constant_current_percent = 100.0 - DEFAULT_CHARGE_CV_PERCENT;
            ((self.vbat_volts - VBAT_CHARGING_MIN)
                / (VBAT_CHARGING_MAX - VBAT_CHARGING_MIN))
                * constant_current_percent
        } else {
            100.0
                - ((self.charge_current_amps
                    / (DEFAULT_CHARGE_MAX_CURRENT_AMPS - CHARGE_TERMINATION_CURRENT_AMPS))
                    * DEFAULT_CHARGE_CV_PERCENT)
        };

        level.clamp(0.0, 100.0)
    }
}

fn scaled_measurement(raw: u8, lsb_scale: f32, offset: f32) -> f32 {
    let value = raw & 0x7F;
    if value == 0 {
        0.0
    } else {
        value as f32 * lsb_scale + offset
    }
}

/// Initialise the BQ25895 with the original badge firmware's configuration.
///
/// This resets the chip, applies the original register block at `0x02..0x05`,
/// writes `0x8C` to register `0x07`, then reads status once to clear pending
/// state on the shared interrupt line.
pub async fn init_bq25895<BUS>(i2c: &mut BUS) -> Result<(), Error>
where
    BUS: embedded_hal::i2c::I2c,
    Error: From<BUS::Error>,
{
    let mut dummy = [0u8; 2];

    i2c.write(ADDRESS, &[RESET_REGISTER, RESET_REGISTER_VALUE])
        .map_err(Error::from)?;
    Timer::after(Duration::from_millis(10)).await;
    // Config block: Reg 02=0x60, 03=0x00 (WD Disabled), 04=0x18, 05=0x00
    i2c.write(ADDRESS, &[CONFIG_START_REGISTER, 0x60, 0x00, 0x18, 0x00])
        .map_err(Error::from)?;
    i2c.write(ADDRESS, &[CONTROL_REGISTER, CONTROL_REGISTER_VALUE])
        .map_err(Error::from)?;
    i2c.write_read(ADDRESS, &[STATUS_START_REGISTER], &mut dummy)
        .map_err(Error::from)?;

    Ok(())
}

/// Reusable BQ25895 battery/charger reader bound to the system I2C bus.
pub struct Battery<BUS: 'static> {
    i2c: SystemI2cBus<BUS>,
}

impl<BUS: 'static> Battery<BUS>
where
    BUS: embedded_hal_async::i2c::I2c,
    Error: From<BUS::Error>,
{
    /// Create a new battery reader from a mux-aware system I2C handle.
    pub fn new(i2c: SystemI2cBus<BUS>) -> Self {
        Self { i2c }
    }

    /// Read the latest battery and charger state from the BQ25895.
    pub async fn read(&mut self) -> Result<BatteryState, Error> {
        let mut registers = [0u8; STATUS_BLOCK_LEN];
        self.i2c
            .write_read(ADDRESS, &[STATUS_START_REGISTER], &mut registers)
            .await
            .map_err(Error::from)?;
        Ok(BatteryState::from_registers(registers))
    }

    /// Read a small set of raw PMIC control registers plus the parsed battery state.
    pub async fn diagnostics(&mut self) -> Result<BatteryDiagnostics, Error> {
        let state = self.read().await?;
        let reg00_input_source = self.read_register(INPUT_SOURCE_CONTROL_REGISTER).await?;
        let reg03_power_on_config = self.read_register(POWER_ON_CONFIG_REGISTER).await?;
        let reg07_charge_timer = self.read_register(CONTROL_REGISTER).await?;
        let reg09_misc_operation = self.read_register(MISC_OPERATION_REGISTER).await?;

        Ok(BatteryDiagnostics {
            state,
            reg00_input_source,
            reg03_power_on_config,
            reg07_charge_timer,
            reg09_misc_operation,
        })
    }

    /// Request badge power-off by disconnecting the battery via the BATFET.
    pub async fn power_off(&mut self) -> Result<(), Error> {
        let misc = self.read_register(MISC_OPERATION_REGISTER).await?;
        self.write_register(
            MISC_OPERATION_REGISTER,
            misc | BATFET_DISABLE_MASK | BATFET_IMMEDIATE_MASK,
        )
        .await
    }

    async fn read_register(&mut self, register: u8) -> Result<u8, Error> {
        let mut value = [0u8; 1];
        self.i2c
            .write_read(ADDRESS, &[register], &mut value)
            .await
            .map_err(Error::from)?;
        Ok(value[0])
    }

    async fn write_register(&mut self, register: u8, value: u8) -> Result<(), Error> {
        self.i2c
            .write(ADDRESS, &[register, value])
            .await
            .map_err(Error::from)
    }
}
