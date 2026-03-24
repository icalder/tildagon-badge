#[derive(Debug)]
pub enum Error {
    I2c(esp_hal::i2c::master::Error),
    I2cConfig(esp_hal::i2c::master::ConfigError),
    Leds(esp_hal_smartled::LedAdapterError),
    Pins(embedded_hal::digital::ErrorKind),
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
