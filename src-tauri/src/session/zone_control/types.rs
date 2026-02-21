use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZoneMode {
    Power,
    HeartRate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneTarget {
    pub mode: ZoneMode,
    pub zone: u8,
    pub lower_bound: u16,
    pub upper_bound: u16,
    pub duration_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneControlStatus {
    pub active: bool,
    pub mode: Option<ZoneMode>,
    pub target_zone: Option<u8>,
    pub lower_bound: Option<u16>,
    pub upper_bound: Option<u16>,
    pub commanded_power: Option<u16>,
    pub time_in_zone_secs: u64,
    pub elapsed_secs: u64,
    pub duration_secs: Option<u64>,
    pub paused: bool,
    pub phase: String,
    pub safety_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StopReason {
    UserStopped,
    DurationComplete,
    SafetyStop,
    TrainerDisconnected,
    SensorLost,
}
