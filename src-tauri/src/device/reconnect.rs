use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::types::DeviceInfo;

const INITIAL_BACKOFF_MS: u64 = 2000;
const MAX_BACKOFF_MS: u64 = 30000;
const BACKOFF_MULTIPLIER: u64 = 2;

struct ReconnectTarget {
    info: DeviceInfo,
    next_retry: Instant,
    backoff_ms: u64,
    attempts: u32,
}

pub struct ReconnectManager {
    targets: HashMap<String, ReconnectTarget>,
}

impl ReconnectManager {
    pub fn new() -> Self {
        Self {
            targets: HashMap::new(),
        }
    }

    /// Register a device for auto-reconnect (called when watchdog detects disconnect)
    pub fn register(&mut self, info: DeviceInfo) {
        if self.targets.contains_key(&info.id) {
            return;
        }
        log::info!("[{}] Registered for auto-reconnect", info.id);
        self.targets.insert(
            info.id.clone(),
            ReconnectTarget {
                info,
                next_retry: Instant::now() + Duration::from_millis(INITIAL_BACKOFF_MS),
                backoff_ms: INITIAL_BACKOFF_MS,
                attempts: 0,
            },
        );
    }

    /// Remove a device from reconnect targets
    pub fn remove(&mut self, device_id: &str) {
        if self.targets.remove(device_id).is_some() {
            log::info!("[{}] Removed from auto-reconnect", device_id);
        }
    }

    /// Clear all targets
    pub fn clear(&mut self) {
        if !self.targets.is_empty() {
            log::info!("Cleared {} auto-reconnect targets", self.targets.len());
            self.targets.clear();
        }
    }

    /// Return devices due for a retry attempt and bump their backoff
    pub fn due_for_retry(&mut self) -> Vec<DeviceInfo> {
        let now = Instant::now();
        let mut due = Vec::new();
        for target in self.targets.values_mut() {
            if now >= target.next_retry {
                due.push(target.info.clone());
                target.attempts += 1;
                target.backoff_ms =
                    (target.backoff_ms * BACKOFF_MULTIPLIER).min(MAX_BACKOFF_MS);
                target.next_retry = now + Duration::from_millis(target.backoff_ms);
            }
        }
        due
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    pub fn attempt_count(&self, device_id: &str) -> u32 {
        self.targets
            .get(device_id)
            .map(|t| t.attempts)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::types::{ConnectionStatus, DeviceType, Transport};

    fn test_device(id: &str) -> DeviceInfo {
        DeviceInfo {
            id: id.to_string(),
            name: Some("Test Device".to_string()),
            device_type: DeviceType::HeartRate,
            status: ConnectionStatus::Connected,
            transport: Transport::Ble,
            rssi: None,
            battery_level: None,
            last_seen: None,
            manufacturer: None,
            model_number: None,
            serial_number: None,
            device_group: None,
        }
    }

    #[test]
    fn register_and_due_after_initial_backoff() {
        let mut rm = ReconnectManager::new();
        rm.register(test_device("dev1"));

        // Not immediately due (initial backoff is 2s)
        assert!(rm.due_for_retry().is_empty());

        // Force next_retry to now
        rm.targets.get_mut("dev1").unwrap().next_retry = Instant::now();
        let due = rm.due_for_retry();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, "dev1");
    }

    #[test]
    fn backoff_doubles_up_to_cap() {
        let mut rm = ReconnectManager::new();
        rm.register(test_device("dev1"));

        let expected_backoffs: Vec<u64> = vec![4000, 8000, 16000, 30000, 30000];
        for expected in expected_backoffs {
            rm.targets.get_mut("dev1").unwrap().next_retry = Instant::now();
            rm.due_for_retry();
            assert_eq!(rm.targets.get("dev1").unwrap().backoff_ms, expected);
        }
    }

    #[test]
    fn remove_clears_target() {
        let mut rm = ReconnectManager::new();
        rm.register(test_device("dev1"));
        assert!(!rm.is_empty());

        rm.remove("dev1");
        assert!(rm.is_empty());
    }

    #[test]
    fn duplicate_register_is_noop() {
        let mut rm = ReconnectManager::new();
        rm.register(test_device("dev1"));

        // Force some attempts
        rm.targets.get_mut("dev1").unwrap().next_retry = Instant::now();
        rm.due_for_retry(); // attempts = 1
        rm.targets.get_mut("dev1").unwrap().next_retry = Instant::now();
        rm.due_for_retry(); // attempts = 2

        assert_eq!(rm.attempt_count("dev1"), 2);

        // Re-register should be a no-op
        rm.register(test_device("dev1"));
        assert_eq!(rm.attempt_count("dev1"), 2);
    }
}
