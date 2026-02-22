use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandSource {
    ZoneControl,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transport {
    Ble,
    AntPlus,
}

impl Transport {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ble => "Ble",
            Self::AntPlus => "AntPlus",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceType {
    HeartRate,
    Power,
    CadenceSpeed,
    FitnessTrainer,
}

impl DeviceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::HeartRate => "HeartRate",
            Self::Power => "Power",
            Self::CadenceSpeed => "CadenceSpeed",
            Self::FitnessTrainer => "FitnessTrainer",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: Option<String>,
    pub device_type: DeviceType,
    pub status: ConnectionStatus,
    pub transport: Transport,
    pub rssi: Option<i16>,
    pub battery_level: Option<u8>,
    pub last_seen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_group: Option<String>,
    /// Whether the device was found during the most recent scan.
    /// Defaults to `true` (optimistic before any scan has run).
    #[serde(default = "default_true")]
    pub in_range: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SensorReading {
    Power {
        watts: u16,
        #[serde(skip)]
        timestamp: Option<Instant>,
        epoch_ms: u64,
        #[serde(default)]
        device_id: String,
        /// Right pedal contribution %. Present when pedal differentiation is reported.
        /// ~50% = combined (L+R), ~100% = right pedal only.
        pedal_balance: Option<u8>,
    },
    HeartRate {
        bpm: u8,
        #[serde(skip)]
        timestamp: Option<Instant>,
        epoch_ms: u64,
        #[serde(default)]
        device_id: String,
    },
    Cadence {
        rpm: f32,
        #[serde(skip)]
        timestamp: Option<Instant>,
        epoch_ms: u64,
        #[serde(default)]
        device_id: String,
    },
    Speed {
        kmh: f32,
        #[serde(skip)]
        timestamp: Option<Instant>,
        epoch_ms: u64,
        #[serde(default)]
        device_id: String,
    },
    TrainerCommand {
        target_watts: u16,
        epoch_ms: u64,
        source: CommandSource,
    },
}

/// Detailed information about a connected device, including GATT services and characteristics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceDetails {
    pub id: String,
    pub name: Option<String>,
    pub device_type: DeviceType,
    pub transport: Transport,
    pub rssi: Option<i16>,
    pub battery_level: Option<u8>,
    pub manufacturer: Option<String>,
    pub model_number: Option<String>,
    pub serial_number: Option<String>,
    pub firmware_revision: Option<String>,
    pub hardware_revision: Option<String>,
    pub software_revision: Option<String>,
    pub services: Vec<ServiceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub uuid: String,
    pub name: Option<String>,
    pub characteristics: Vec<CharacteristicInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacteristicInfo {
    pub uuid: String,
    pub name: Option<String>,
    pub properties: Vec<String>,
}

/// Metadata decoded from ANT+ Common Data Pages (80, 81, 82)
#[derive(Debug, Clone, Default)]
pub struct AntDeviceMetadata {
    pub manufacturer_id: Option<u16>,
    pub model_number: Option<u16>,
    pub hw_revision: Option<u8>,
    pub sw_revision: Option<String>,
    pub serial_number: Option<u32>,
    pub battery_level: Option<u8>,
    pub battery_voltage: Option<f32>,
}

/// Returns true when the reading comes from a non-primary device for its type.
/// Used by listeners to drop dominated readings before they enter the broadcast channel.
pub fn is_dominated(
    primaries: &HashMap<DeviceType, String>,
    reading: &SensorReading,
) -> bool {
    if let Some(primary_id) = primaries.get(&reading.device_type()) {
        !reading.device_id().is_empty() && reading.device_id() != primary_id
    } else {
        false
    }
}

impl SensorReading {
    #[allow(dead_code)]
    pub fn epoch_ms(&self) -> u64 {
        match self {
            SensorReading::Power { epoch_ms, .. } => *epoch_ms,
            SensorReading::HeartRate { epoch_ms, .. } => *epoch_ms,
            SensorReading::Cadence { epoch_ms, .. } => *epoch_ms,
            SensorReading::Speed { epoch_ms, .. } => *epoch_ms,
            SensorReading::TrainerCommand { epoch_ms, .. } => *epoch_ms,
        }
    }

    pub fn device_id(&self) -> &str {
        match self {
            SensorReading::Power { device_id, .. } => device_id,
            SensorReading::HeartRate { device_id, .. } => device_id,
            SensorReading::Cadence { device_id, .. } => device_id,
            SensorReading::Speed { device_id, .. } => device_id,
            SensorReading::TrainerCommand { .. } => "",
        }
    }

    pub fn device_type(&self) -> DeviceType {
        match self {
            SensorReading::Power { .. } => DeviceType::Power,
            SensorReading::HeartRate { .. } => DeviceType::HeartRate,
            SensorReading::Cadence { .. } => DeviceType::CadenceSpeed,
            SensorReading::Speed { .. } => DeviceType::CadenceSpeed,
            SensorReading::TrainerCommand { .. } => DeviceType::FitnessTrainer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn power_reading(device_id: &str) -> SensorReading {
        SensorReading::Power {
            watts: 200,
            timestamp: None,
            epoch_ms: 0,
            device_id: device_id.to_string(),
            pedal_balance: None,
        }
    }

    fn hr_reading(device_id: &str) -> SensorReading {
        SensorReading::HeartRate {
            bpm: 140,
            timestamp: None,
            epoch_ms: 0,
            device_id: device_id.to_string(),
        }
    }

    #[test]
    fn is_dominated_non_primary_device_is_dominated() {
        let primaries = HashMap::from([(DeviceType::Power, "pm-1".to_string())]);
        assert!(is_dominated(&primaries, &power_reading("pm-2")));
    }

    #[test]
    fn is_dominated_primary_device_is_not_dominated() {
        let primaries = HashMap::from([(DeviceType::Power, "pm-1".to_string())]);
        assert!(!is_dominated(&primaries, &power_reading("pm-1")));
    }

    #[test]
    fn is_dominated_no_primary_set_is_not_dominated() {
        let primaries = HashMap::new();
        assert!(!is_dominated(&primaries, &power_reading("pm-1")));
    }

    #[test]
    fn is_dominated_empty_device_id_is_not_dominated() {
        let primaries = HashMap::from([(DeviceType::Power, "pm-1".to_string())]);
        assert!(!is_dominated(&primaries, &power_reading("")));
    }

    #[test]
    fn is_dominated_different_type_primary_does_not_affect() {
        let primaries = HashMap::from([(DeviceType::HeartRate, "hr-1".to_string())]);
        assert!(!is_dominated(&primaries, &power_reading("pm-1")));
    }

    #[test]
    fn is_dominated_trainer_command_has_empty_device_id() {
        let primaries = HashMap::from([(DeviceType::FitnessTrainer, "trainer-1".to_string())]);
        let cmd = SensorReading::TrainerCommand {
            target_watts: 200,
            epoch_ms: 0,
            source: CommandSource::Manual,
        };
        // TrainerCommand.device_id() returns "", so it should not be dominated
        assert!(!is_dominated(&primaries, &cmd));
    }

    #[test]
    fn is_dominated_each_type_independent() {
        let primaries = HashMap::from([
            (DeviceType::Power, "pm-1".to_string()),
            (DeviceType::HeartRate, "hr-1".to_string()),
        ]);
        // HR reading from non-primary HR device is dominated
        assert!(is_dominated(&primaries, &hr_reading("hr-2")));
        // Power reading from primary power device is not
        assert!(!is_dominated(&primaries, &power_reading("pm-1")));
    }
}
