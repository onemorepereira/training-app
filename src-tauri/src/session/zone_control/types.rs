use serde::{Deserialize, Serialize};

use crate::config::MAX_TRAINER_WATTS;
use crate::error::AppError;

/// Highest HR bound accepted for an HR target (bpm)
pub const MAX_HR_BOUND: u16 = 250;
/// Highest zone number per mode; 0 means custom bounds
pub const MAX_POWER_ZONE: u8 = 7;
pub const MAX_HR_ZONE: u8 = 5;

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

impl ZoneTarget {
    /// Reject targets the controller cannot safely run (design ADR-9).
    pub fn validate(&self) -> Result<(), AppError> {
        let (max_zone, max_bound, label, unit) = match self.mode {
            ZoneMode::Power => (MAX_POWER_ZONE, MAX_TRAINER_WATTS, "Power", "W"),
            ZoneMode::HeartRate => (MAX_HR_ZONE, MAX_HR_BOUND, "HR", "bpm"),
        };
        if self.zone > max_zone {
            return Err(AppError::Session(format!(
                "{label} zone must be 0 (custom) or 1-{max_zone}, got {}",
                self.zone
            )));
        }
        if self.lower_bound >= self.upper_bound {
            return Err(AppError::Session(format!(
                "Zone lower bound ({}) must be less than upper bound ({})",
                self.lower_bound, self.upper_bound
            )));
        }
        if self.upper_bound > max_bound {
            return Err(AppError::Session(format!(
                "{label} zone upper bound must be at most {max_bound} {unit}, got {}",
                self.upper_bound
            )));
        }
        Ok(())
    }

    /// Midpoint of the bounds, computed without u16 overflow.
    pub fn midpoint(&self) -> u16 {
        ((self.lower_bound as u32 + self.upper_bound as u32) / 2) as u16
    }
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
    /// Why the latest run ended; None while active or before any run
    pub stop_reason: Option<StopReason>,
}

/// Serialized as plain strings; the frontend (Area D) depends on these names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    UserStopped,
    DurationComplete,
    SafetyStop,
    TrainerDisconnected,
    SensorLost,
    /// Backend stop_session
    SessionStopped,
    /// Window close / app exit
    AppClosing,
    /// Control task panicked or failed
    ControllerFault,
    /// A safety power cut could not be delivered (ADR-15)
    TrainerUnresponsive,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(mode: ZoneMode, zone: u8, lower: u16, upper: u16) -> ZoneTarget {
        ZoneTarget { mode, zone, lower_bound: lower, upper_bound: upper, duration_secs: None }
    }

    fn err_msg(t: &ZoneTarget) -> String {
        t.validate().unwrap_err().to_string()
    }

    #[test]
    fn power_zone_8_rejected() {
        let msg = err_msg(&target(ZoneMode::Power, 8, 100, 200));
        assert!(msg.contains("Power zone must be 0 (custom) or 1-7, got 8"), "{msg}");
    }

    #[test]
    fn power_zone_7_accepted() {
        assert!(target(ZoneMode::Power, 7, 300, 400).validate().is_ok());
    }

    #[test]
    fn hr_zone_6_rejected() {
        let msg = err_msg(&target(ZoneMode::HeartRate, 6, 150, 160));
        assert!(msg.contains("HR zone must be 0 (custom) or 1-5, got 6"), "{msg}");
    }

    #[test]
    fn zone_0_custom_accepted() {
        assert!(target(ZoneMode::HeartRate, 0, 150, 160).validate().is_ok());
        assert!(target(ZoneMode::Power, 0, 150, 160).validate().is_ok());
    }

    #[test]
    fn lower_equal_upper_rejected() {
        let msg = err_msg(&target(ZoneMode::Power, 3, 200, 200));
        assert!(msg.contains("lower bound (200) must be less than upper bound (200)"), "{msg}");
    }

    #[test]
    fn power_bound_above_2000_rejected() {
        let msg = err_msg(&target(ZoneMode::Power, 0, 1000, 2001));
        assert!(msg.contains("at most 2000 W, got 2001"), "{msg}");
        assert!(target(ZoneMode::Power, 0, 1000, 2000).validate().is_ok());
    }

    #[test]
    fn hr_bound_above_250_rejected() {
        let msg = err_msg(&target(ZoneMode::HeartRate, 0, 200, 251));
        assert!(msg.contains("at most 250 bpm, got 251"), "{msg}");
        assert!(target(ZoneMode::HeartRate, 0, 200, 250).validate().is_ok());
    }

    #[test]
    fn new_stop_reasons_serialize_as_plain_strings() {
        for (r, name) in [
            (StopReason::SessionStopped, "SessionStopped"),
            (StopReason::AppClosing, "AppClosing"),
            (StopReason::ControllerFault, "ControllerFault"),
            (StopReason::TrainerUnresponsive, "TrainerUnresponsive"),
        ] {
            assert_eq!(serde_json::to_string(&r).unwrap(), format!("\"{name}\""));
        }
    }

    #[test]
    fn midpoint_of_max_bounds_does_not_overflow() {
        assert_eq!(target(ZoneMode::Power, 0, u16::MAX - 1, u16::MAX).midpoint(), 65534);
        assert_eq!(target(ZoneMode::HeartRate, 0, 130, 141).midpoint(), 135);
    }
}
