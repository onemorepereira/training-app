use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    Running,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    pub ftp: u16,
    pub weight_kg: f32,
    pub hr_zones: [u8; 5],
    pub units: String,
    pub power_zones: [u16; 6],
    pub date_of_birth: Option<String>,
    pub sex: Option<String>,
    pub resting_hr: Option<u8>,
    pub max_hr: Option<u8>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            ftp: 200,
            weight_kg: 75.0,
            hr_zones: [120, 140, 160, 175, 190],
            units: "metric".to_string(),
            power_zones: [55, 75, 90, 105, 120, 150],
            date_of_birth: None,
            sex: None,
            resting_hr: None,
            max_hr: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub start_time: DateTime<Utc>,
    pub duration_secs: u64,
    pub ftp: Option<u16>,
    pub avg_power: Option<u16>,
    pub max_power: Option<u16>,
    pub normalized_power: Option<u16>,
    pub tss: Option<f32>,
    pub intensity_factor: Option<f32>,
    pub avg_hr: Option<u8>,
    pub max_hr: Option<u8>,
    pub avg_cadence: Option<f32>,
    pub avg_speed: Option<f32>,
    pub work_kj: Option<f32>,
    pub variability_index: Option<f32>,
    pub distance_km: Option<f32>,
    pub title: Option<String>,
    pub activity_type: Option<String>,
    pub rpe: Option<u8>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveMetrics {
    pub elapsed_secs: u64,
    pub current_power: Option<u16>,
    pub avg_power_3s: Option<f32>,
    pub avg_power_10s: Option<f32>,
    pub avg_power_30s: Option<f32>,
    pub normalized_power: Option<f32>,
    pub tss: Option<f32>,
    pub intensity_factor: Option<f32>,
    pub current_hr: Option<u8>,
    pub current_cadence: Option<f32>,
    pub current_speed: Option<f32>,
    pub hr_zone: Option<u8>,
    pub power_zone: Option<u8>,
    /// True when no power reading received for >5s
    pub stale_power: bool,
    /// True when no HR reading received for >5s
    pub stale_hr: bool,
    /// True when no cadence reading received for >5s
    pub stale_cadence: bool,
    /// True when no speed reading received for >5s
    pub stale_speed: bool,
}
