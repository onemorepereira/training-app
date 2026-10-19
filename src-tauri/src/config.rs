/// Connection watchdog interval — how often we check for stale/disconnected devices.
pub const CONNECTION_CHECK_INTERVAL_SECS: u64 = 5;

/// Autosave interval — how often we snapshot the active session to disk.
pub const AUTOSAVE_INTERVAL_SECS: u64 = 30;

/// Live metrics push interval — how often we emit metrics to the frontend.
pub const LIVE_METRICS_PUSH_MS: u64 = 250;

/// BLE scan duration — how long a BLE scan runs before returning results.
pub const BLE_SCAN_DURATION_SECS: u64 = 3;

/// Device disconnect timeout — ANT+ device considered lost after this many seconds
/// without data. Intentionally longer than READING_FRESHNESS_SECS: we stop using
/// stale data for metrics quickly (5s) but give the device more time (10s) before
/// triggering disconnect/reconnect logic.
pub const DEVICE_DISCONNECT_TIMEOUT_SECS: u64 = 10;

/// Reading freshness window — readings older than this are ignored by the session
/// metrics engine. Shorter than DEVICE_DISCONNECT_TIMEOUT_SECS so metrics stay
/// responsive even while the watchdog still considers the device connected.
pub const READING_FRESHNESS_SECS: u64 = 5;

/// Reconnect initial backoff — delay before first reconnect attempt.
pub const RECONNECT_INITIAL_BACKOFF_MS: u64 = 2000;

/// Reconnect max backoff — ceiling for exponential backoff.
pub const RECONNECT_MAX_BACKOFF_MS: u64 = 30000;

/// Reconnect backoff multiplier.
pub const RECONNECT_BACKOFF_MULTIPLIER: u64 = 2;

/// Highest target power ever sent to a trainer (watts). Every target-power
/// command is clamped to [0, MAX_TRAINER_WATTS] in `DeviceManager::set_target_power`.
pub const MAX_TRAINER_WATTS: u16 = 2000;

/// Clamp a requested target power to [0, MAX_TRAINER_WATTS]. Takes i32 so
/// callers never need a wrapping `as i16` cast.
pub fn clamp_trainer_watts(watts: i32) -> i16 {
    watts.clamp(0, MAX_TRAINER_WATTS as i32) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_trainer_watts_caps_at_2000() {
        assert_eq!(clamp_trainer_watts(2001), 2000);
        assert_eq!(clamp_trainer_watts(2000), 2000);
        assert_eq!(clamp_trainer_watts(250), 250);
    }

    #[test]
    fn clamp_trainer_watts_negative_is_zero() {
        assert_eq!(clamp_trainer_watts(-50), 0);
    }

    #[test]
    fn clamp_trainer_watts_u16_max_does_not_wrap() {
        // 65535 as i16 would be -1; 40000 as i16 would be -25536
        assert_eq!(clamp_trainer_watts(u16::MAX as i32), 2000);
        assert_eq!(clamp_trainer_watts(40000), 2000);
    }
}
