use thiserror::Error;

#[derive(Error, Debug)]
pub enum BleError {
    #[error("BLE not initialized")]
    NotInitialized,
    #[error("No BLE adapter found")]
    NoAdapter,
    #[error("No recognized services on device {0}")]
    UnrecognizedDevice(String),
    #[error("Characteristic not found: {0}")]
    CharacteristicNotFound(String),
    #[error("{0}")]
    Btleplug(String),
}

#[derive(Error, Debug)]
pub enum AntError {
    #[error("No ANT+ USB stick found")]
    NoUsbStick,
    #[error("All ANT+ channels in use ({0})")]
    NoFreeChannel(String),
    #[error("Not supported: {0}")]
    NotSupported(String),
    #[error("ANT+ task panicked: {0}")]
    TaskPanicked(String),
    #[error("{0}")]
    Usb(String),
    #[error("{0}")]
    Channel(String),
}

#[derive(Error, Debug)]
pub enum AppError {
    #[error("BLE error: {0}")]
    Ble(#[from] BleError),
    #[error("ANT+ error: {0}")]
    AntPlus(#[from] AntError),
    #[error("Device not found: {0}")]
    DeviceNotFound(String),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Session error: {0}")]
    Session(String),
}

impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        let code = match self {
            AppError::Ble(_) => "ble_error",
            AppError::AntPlus(_) => "ant_error",
            AppError::DeviceNotFound(_) => "device_not_found",
            AppError::Database(_) => "database_error",
            AppError::Serialization(_) => "serialization_error",
            AppError::Session(_) => "session_error",
        };
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("code", code)?;
        map.serialize_entry("message", &self.to_string())?;
        map.end()
    }
}
