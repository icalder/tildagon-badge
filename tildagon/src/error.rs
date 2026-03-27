#[derive(Debug)]
pub enum Error {
    I2c(esp_hal::i2c::master::Error),
    I2cConfig(esp_hal::i2c::master::ConfigError),
    Leds(esp_hal_smartled::LedAdapterError),
    Pins(embedded_hal::digital::ErrorKind),
    Radio(esp_radio::InitializationError),
    RadioAlreadyInitialized,
    RadioUnavailable,
    Wifi(esp_radio::wifi::WifiError),
    WifiAlreadyInitialized,
    BleConfig(esp_radio::ble::InvalidConfigError),
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

impl From<esp_radio::wifi::WifiError> for Error {
    fn from(e: esp_radio::wifi::WifiError) -> Self {
        Error::Wifi(e)
    }
}

impl From<esp_radio::ble::InvalidConfigError> for Error {
    fn from(e: esp_radio::ble::InvalidConfigError) -> Self {
        Error::BleConfig(e)
    }
}
