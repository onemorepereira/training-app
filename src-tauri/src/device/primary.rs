//! Primary-source selection per metric (design ADR-3, 3a, 3b).
//!
//! Several connected devices can report the same metric (a trainer and a power
//! meter both report power; a pod, a power meter and a trainer can all report
//! cadence). Exactly one source per metric reaches the session: the best *fresh*
//! source, ranked
//!
//!   primary (4) > other dedicated sensor (3) > power meter (2) > trainer (1)
//!
//! A device becomes primary for a metric it can claim on its first reading of that
//! metric, if no primary exists. Primaries, capabilities and freshness live under a
//! single lock so claim and release are atomic: once a device is unregistered its
//! late readings are dropped and can never re-claim.

use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use super::types::{DeviceType, SensorReading};

/// A source counts as fresh if it reported this metric within this window.
pub const SOURCE_FRESH: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Metric {
    Power,
    HeartRate,
    Cadence,
    Speed,
    Trainer,
}

/// Fallback rank for metrics a device reports but cannot claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceClass {
    Dedicated,
    PowerMeter,
    Trainer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCaps {
    pub claimable: HashSet<Metric>,
    pub class: SourceClass,
}

impl DeviceCaps {
    /// Registered before the transport connect, while the device type may not be
    /// known yet (BLE classifies on connect): forwards as a low-rank source, claims nothing.
    pub fn pending() -> Self {
        Self { claimable: HashSet::new(), class: SourceClass::Trainer }
    }

    /// Capabilities by device type; ANT+ speed-only (123) and cadence-only (122)
    /// sensors claim only their metric.
    pub fn for_device(device_id: &str, device_type: DeviceType) -> Self {
        use Metric::*;
        let (claimable, class): (&[Metric], SourceClass) = match device_type {
            DeviceType::HeartRate => (&[HeartRate], SourceClass::Dedicated),
            DeviceType::Power => (&[Power], SourceClass::PowerMeter),
            DeviceType::FitnessTrainer => (&[Trainer], SourceClass::Trainer),
            DeviceType::CadenceSpeed => {
                let m: &[Metric] = if device_id.starts_with("ant:122:") {
                    &[Cadence]
                } else if device_id.starts_with("ant:123:") {
                    &[Speed]
                } else {
                    &[Cadence, Speed]
                };
                (m, SourceClass::Dedicated)
            }
        };
        Self { claimable: claimable.iter().copied().collect(), class }
    }
}

impl SensorReading {
    pub fn metric(&self) -> Option<Metric> {
        match self {
            SensorReading::Power { .. } => Some(Metric::Power),
            SensorReading::HeartRate { .. } => Some(Metric::HeartRate),
            SensorReading::Cadence { .. } => Some(Metric::Cadence),
            SensorReading::Speed { .. } => Some(Metric::Speed),
            SensorReading::TrainerCommand { .. } => None,
        }
    }
}

/// Key accepted by `set_primary_device`: the legacy `DeviceType` names plus the
/// per-metric names (backward compatible superset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum PrimaryKey {
    HeartRate,
    Power,
    CadenceSpeed,
    FitnessTrainer,
    Cadence,
    Speed,
}

#[derive(Debug, Default)]
pub struct PrimaryState {
    primaries: HashMap<Metric, String>,
    caps: HashMap<String, DeviceCaps>,
    last_reading: HashMap<(Metric, String), Instant>,
}

pub type SharedPrimaryState = Arc<RwLock<PrimaryState>>;

impl PrimaryState {
    pub fn shared() -> SharedPrimaryState {
        Arc::new(RwLock::new(Self::default()))
    }

    pub fn register(&mut self, device_id: &str, caps: DeviceCaps) {
        self.caps.insert(device_id.to_string(), caps);
    }

    pub fn is_registered(&self, device_id: &str) -> bool {
        self.caps.contains_key(device_id)
    }

    /// Disconnect: capabilities first, then claims, then freshness, so a late
    /// reading finds no capabilities and is dropped without claiming.
    pub fn unregister(&mut self, device_id: &str) {
        self.caps.remove(device_id);
        self.release_claims(device_id);
        self.last_reading.retain(|(_, d), _| d != device_id);
    }

    /// Stale/lost device: free its primary slots so a backup can take over; it
    /// stays registered and may re-claim a free slot when it returns.
    pub fn release_claims(&mut self, device_id: &str) {
        self.primaries.retain(|_, d| d != device_id);
    }

    /// Claim a metric if nobody holds it (used for the trainer at connect commit).
    pub fn claim_if_free(&mut self, metric: Metric, device_id: &str) {
        self.primaries.entry(metric).or_insert_with(|| device_id.to_string());
    }

    pub fn primary(&self, metric: Metric) -> Option<&str> {
        self.primaries.get(&metric).map(String::as_str)
    }

    /// User selection. Legacy `CadenceSpeed` applies to the metrics the device can
    /// claim (both when unknown), so picking one of two pods doesn't steal the other's.
    pub fn set_primary(&mut self, key: PrimaryKey, device_id: &str) {
        let metrics: Vec<Metric> = match key {
            PrimaryKey::HeartRate => vec![Metric::HeartRate],
            PrimaryKey::Power => vec![Metric::Power],
            PrimaryKey::FitnessTrainer => vec![Metric::Trainer],
            PrimaryKey::Cadence => vec![Metric::Cadence],
            PrimaryKey::Speed => vec![Metric::Speed],
            PrimaryKey::CadenceSpeed => {
                let both = [Metric::Cadence, Metric::Speed];
                match self.caps.get(device_id) {
                    Some(c) if both.iter().any(|m| c.claimable.contains(m)) => {
                        both.into_iter().filter(|m| c.claimable.contains(m)).collect()
                    }
                    _ => both.to_vec(),
                }
            }
        };
        for m in metrics {
            self.primaries.insert(m, device_id.to_string());
        }
    }

    /// `get_primary_devices` payload: legacy keys (`CadenceSpeed` = cadence primary,
    /// else speed primary) plus per-metric `Cadence` / `Speed`.
    pub fn legacy_map(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        let mut put = |k: &str, m: Metric| {
            if let Some(d) = self.primaries.get(&m) {
                out.insert(k.to_string(), d.clone());
            }
        };
        put("HeartRate", Metric::HeartRate);
        put("Power", Metric::Power);
        put("FitnessTrainer", Metric::Trainer);
        put("Cadence", Metric::Cadence);
        put("Speed", Metric::Speed);
        if let Some(d) = self.primaries.get(&Metric::Cadence).or_else(|| self.primaries.get(&Metric::Speed)) {
            out.insert("CadenceSpeed".to_string(), d.clone());
        }
        out
    }

    fn rank(&self, metric: Metric, device_id: &str) -> u8 {
        if self.primaries.get(&metric).is_some_and(|p| p == device_id) {
            return 4;
        }
        let Some(caps) = self.caps.get(device_id) else {
            return 0;
        };
        if caps.claimable.contains(&metric) {
            return 3;
        }
        match caps.class {
            SourceClass::PowerMeter => 2,
            SourceClass::Dedicated | SourceClass::Trainer => 1,
        }
    }

    /// Decide whether `reading` reaches the session. See module docs.
    pub fn admit(&mut self, reading: &SensorReading, now: Instant) -> bool {
        let device_id = reading.device_id();
        let Some(metric) = reading.metric() else {
            return true;
        };
        if device_id.is_empty() {
            return true;
        }
        let Some(caps) = self.caps.get(device_id) else {
            return false; // unregistered (disconnected) device
        };
        if caps.claimable.contains(&metric) && !self.primaries.contains_key(&metric) {
            self.primaries.insert(metric, device_id.to_string());
        }
        self.last_reading.insert((metric, device_id.to_string()), now);

        let best = self
            .last_reading
            .iter()
            .filter(|((m, d), at)| {
                *m == metric && self.caps.contains_key(d) && now.saturating_duration_since(**at) <= SOURCE_FRESH
            })
            .map(|((_, d), _)| d.as_str())
            // Highest rank wins; ties go to the smallest id so streams never interleave.
            .max_by(|a, b| self.rank(metric, a).cmp(&self.rank(metric, b)).then_with(|| b.cmp(a)));
        best == Some(device_id)
    }
}

/// Listener entry point: one write lock per reading.
pub fn admit_reading(state: &SharedPrimaryState, reading: &SensorReading, now: Instant) -> bool {
    state.write().unwrap_or_else(|e| e.into_inner()).admit(reading, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn power(id: &str) -> SensorReading {
        SensorReading::Power { watts: 200, timestamp: None, epoch_ms: 0, device_id: id.into(), pedal_balance: None }
    }
    fn cadence(id: &str) -> SensorReading {
        SensorReading::Cadence { rpm: 90.0, timestamp: None, epoch_ms: 0, device_id: id.into() }
    }
    fn speed(id: &str) -> SensorReading {
        SensorReading::Speed { kmh: 30.0, timestamp: None, epoch_ms: 0, device_id: id.into() }
    }

    fn state(devices: &[(&str, DeviceType)]) -> PrimaryState {
        let mut s = PrimaryState::default();
        for (id, t) in devices {
            s.register(id, DeviceCaps::for_device(id, *t));
        }
        s
    }

    #[test]
    fn admit_speed_only_and_cadence_only_pods_both_forwarded() {
        // Two separate CadenceSpeed pods (C3): one only ever sends speed, one cadence.
        let mut s = state(&[("pod-speed", DeviceType::CadenceSpeed), ("pod-cad", DeviceType::CadenceSpeed)]);
        let t = Instant::now();
        assert!(s.admit(&speed("pod-speed"), t));
        assert!(s.admit(&cadence("pod-cad"), t));
        assert!(s.admit(&speed("pod-speed"), t + Duration::from_secs(1)));
        assert!(s.admit(&cadence("pod-cad"), t + Duration::from_secs(1)));
        assert_eq!(s.primary(Metric::Speed), Some("pod-speed"));
        assert_eq!(s.primary(Metric::Cadence), Some("pod-cad"));
    }

    #[test]
    fn admit_second_cadence_device_dominated() {
        let mut s = state(&[("pod-a", DeviceType::CadenceSpeed), ("pod-b", DeviceType::CadenceSpeed)]);
        let t = Instant::now();
        assert!(s.admit(&cadence("pod-a"), t));
        assert!(!s.admit(&cadence("pod-b"), t), "same metric from a second pod is de-duplicated");
    }

    #[test]
    fn admit_power_meter_cadence_not_claimable_but_forwarded() {
        let mut s = state(&[("pm", DeviceType::Power)]);
        let t = Instant::now();
        assert!(s.admit(&cadence("pm"), t));
        assert_eq!(s.primary(Metric::Cadence), None, "power meter cadence never claims");
    }

    #[test]
    fn admit_trainer_power_dominated_by_power_meter_primary() {
        let mut s = state(&[("pm", DeviceType::Power), ("trainer", DeviceType::FitnessTrainer)]);
        let t = Instant::now();
        assert!(s.admit(&power("pm"), t));
        assert!(!s.admit(&power("trainer"), t));
    }

    #[test]
    fn cadence_pod_outranks_power_meter_outranks_trainer() {
        let mut s = state(&[
            ("trainer", DeviceType::FitnessTrainer),
            ("pm", DeviceType::Power),
            ("pod", DeviceType::CadenceSpeed),
        ]);
        let t = Instant::now();
        assert!(s.admit(&cadence("trainer"), t), "only source so far");
        assert!(s.admit(&cadence("pm"), t), "power meter beats trainer");
        assert!(!s.admit(&cadence("trainer"), t), "no interleaving with the trainer");
        assert!(s.admit(&cadence("pod"), t), "pod claims and wins");
        assert!(!s.admit(&cadence("pm"), t));
        assert!(!s.admit(&cadence("trainer"), t));
    }

    #[test]
    fn stale_primary_falls_back_to_power_meter_cadence() {
        let mut s = state(&[("pm", DeviceType::Power), ("pod", DeviceType::CadenceSpeed)]);
        let t = Instant::now();
        assert!(s.admit(&cadence("pod"), t));
        assert!(!s.admit(&cadence("pm"), t + Duration::from_secs(1)));
        // Pod silent for > 5 s
        assert!(s.admit(&cadence("pm"), t + Duration::from_secs(6)));
    }

    #[test]
    fn primary_resume_reclaims_forwarding() {
        let mut s = state(&[("pm", DeviceType::Power), ("pod", DeviceType::CadenceSpeed)]);
        let t = Instant::now();
        s.admit(&cadence("pod"), t);
        assert!(s.admit(&cadence("pm"), t + Duration::from_secs(6)));
        assert!(s.admit(&cadence("pod"), t + Duration::from_secs(7)), "primary back");
        assert!(!s.admit(&cadence("pm"), t + Duration::from_secs(7)));
    }

    #[test]
    fn stale_primary_second_pod_outranks_power_meter() {
        let mut s = state(&[
            ("pod-a", DeviceType::CadenceSpeed),
            ("pod-b", DeviceType::CadenceSpeed),
            ("pm", DeviceType::Power),
        ]);
        let t = Instant::now();
        assert!(s.admit(&cadence("pod-a"), t)); // primary
        let later = t + Duration::from_secs(6); // pod-a stale
        s.admit(&cadence("pm"), later);
        assert!(s.admit(&cadence("pod-b"), later), "second dedicated pod ranks above the power meter");
        assert!(!s.admit(&cadence("pm"), later));
    }

    #[test]
    fn two_power_meters_cadence_never_interleave() {
        let mut s = state(&[("pm-b", DeviceType::Power), ("pm-a", DeviceType::Power)]);
        let t = Instant::now();
        let mut forwarded = Vec::new();
        for i in 0..6 {
            let at = t + Duration::from_millis(250 * i);
            if s.admit(&cadence("pm-b"), at) {
                forwarded.push("pm-b");
            }
            if s.admit(&cadence("pm-a"), at) {
                forwarded.push("pm-a");
            }
        }
        // pm-b alone at first, then pm-a (smaller id) wins every round after.
        assert_eq!(forwarded, vec!["pm-b", "pm-a", "pm-a", "pm-a", "pm-a", "pm-a", "pm-a"]);
    }

    #[test]
    fn reading_after_disconnect_cannot_reclaim_primary() {
        let mut s = state(&[("pm", DeviceType::Power)]);
        let t = Instant::now();
        assert!(s.admit(&power("pm"), t));
        s.unregister("pm");
        assert!(!s.admit(&power("pm"), t + Duration::from_millis(100)), "late reading dropped");
        assert_eq!(s.primary(Metric::Power), None, "and it did not re-claim");
    }

    #[test]
    fn disconnect_removes_caps_before_primaries() {
        let mut s = state(&[("pm", DeviceType::Power)]);
        s.admit(&power("pm"), Instant::now());
        s.unregister("pm");
        assert!(!s.is_registered("pm"));
        assert_eq!(s.primary(Metric::Power), None);
        assert!(s.last_reading.is_empty());
    }

    #[test]
    fn pending_caps_forward_without_claiming() {
        let mut s = PrimaryState::default();
        s.register("ble-x", DeviceCaps::pending());
        assert!(s.admit(&power("ble-x"), Instant::now()));
        assert_eq!(s.primary(Metric::Power), None);
    }

    #[test]
    fn trainer_command_always_admitted() {
        let mut s = PrimaryState::default();
        let cmd = SensorReading::TrainerCommand {
            target_watts: 200,
            epoch_ms: 0,
            source: super::super::types::CommandSource::Manual,
        };
        assert!(s.admit(&cmd, Instant::now()));
    }

    #[test]
    fn primary_key_legacy_cadence_speed_sets_device_capabilities() {
        let mut s = state(&[("ant:123:5", DeviceType::CadenceSpeed), ("ant:122:6", DeviceType::CadenceSpeed)]);
        s.set_primary(PrimaryKey::CadenceSpeed, "ant:123:5");
        assert_eq!(s.primary(Metric::Speed), Some("ant:123:5"));
        assert_eq!(s.primary(Metric::Cadence), None, "speed-only sensor must not take cadence");
        s.set_primary(PrimaryKey::CadenceSpeed, "ant:122:6");
        assert_eq!(s.primary(Metric::Cadence), Some("ant:122:6"));
        assert_eq!(s.primary(Metric::Speed), Some("ant:123:5"));
    }

    #[test]
    fn primary_key_deserializes_legacy_and_metric_names() {
        for (json, key) in [
            ("\"CadenceSpeed\"", PrimaryKey::CadenceSpeed),
            ("\"FitnessTrainer\"", PrimaryKey::FitnessTrainer),
            ("\"Cadence\"", PrimaryKey::Cadence),
            ("\"Speed\"", PrimaryKey::Speed),
        ] {
            assert_eq!(serde_json::from_str::<PrimaryKey>(json).unwrap(), key);
        }
    }

    #[test]
    fn get_primaries_includes_legacy_and_metric_keys() {
        let mut s = state(&[("pod-s", DeviceType::CadenceSpeed), ("trainer", DeviceType::FitnessTrainer)]);
        s.claim_if_free(Metric::Trainer, "trainer");
        s.admit(&speed("pod-s"), Instant::now());
        let m = s.legacy_map();
        assert_eq!(m.get("Speed").map(String::as_str), Some("pod-s"));
        assert_eq!(m.get("CadenceSpeed").map(String::as_str), Some("pod-s"), "falls back to speed");
        assert_eq!(m.get("FitnessTrainer").map(String::as_str), Some("trainer"));
        assert!(!m.contains_key("Cadence"));
    }
}
