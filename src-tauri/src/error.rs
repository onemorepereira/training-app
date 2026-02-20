use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("BLE error: {0}")]
    Ble(String),
    #[error("ANT+ error: {0}")]
    AntPlus(String),
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
