#[derive(Debug)]
pub enum Error {
    I2c(esp_hal::i2c::master::Error),
    I2cConfig(esp_hal::i2c::master::ConfigError),
    Leds(esp_hal_smartled::LedAdapterError),
    Pins(embedded_hal::digital::ErrorKind),
    #[cfg(feature = "radio")]
    Radio(esp_radio::InitializationError),
    #[cfg(feature = "radio")]
    RadioAlreadyInitialized,
    #[cfg(feature = "radio")]
    RadioUnavailable,
    #[cfg(feature = "radio")]
    Wifi(esp_radio::wifi::WifiError),
    #[cfg(feature = "radio")]
    WifiAlreadyInitialized,
    #[cfg(feature = "radio")]
    BleConfig(esp_radio::ble::InvalidConfigError),
    #[cfg(feature = "radio")]
    BleAlreadyInitialized,
}

impl From<esp_hal::i2c::master::Error> for Error {
    fn from(e: esp_hal::i2c::master::Error) -> Self {
        Error::I2c(e)
    }
}

impl From<esp_hal::i2c::master::ConfigError> for Error {
    fn from(e: esp_hal::i2c::master::ConfigError) -> Self {
        Error::I2cConfig(e)
    }
}

impl From<esp_hal_smartled::LedAdapterError> for Error {
    fn from(e: esp_hal_smartled::LedAdapterError) -> Self {
        Error::Leds(e)
    }
}

#[cfg(feature = "radio")]
impl From<esp_radio::wifi::WifiError> for Error {
    fn from(e: esp_radio::wifi::WifiError) -> Self {
        Error::Wifi(e)
    }
}

#[cfg(feature = "radio")]
impl From<esp_radio::ble::InvalidConfigError> for Error {
    fn from(e: esp_radio::ble::InvalidConfigError) -> Self {
        Error::BleConfig(e)
    }
}
