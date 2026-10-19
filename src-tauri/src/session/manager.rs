use chrono::Utc;
use log::info;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::metrics::MetricsCalculator;
use super::types::*;
use crate::device::types::SensorReading;

use crate::config;

pub struct SessionManager {
    current_session: Arc<Mutex<Option<ActiveSession>>>,
    /// Serializes autosave writes with stop. Lock order: `autosave_gate` before
    /// `current_session`, always. Holding it across snapshot + file write means
    /// no autosave can be written for a session after stop has taken it, so stop
    /// can never be followed by a stale autosave reappearing on disk.
    autosave_gate: Mutex<()>,
}

/// A snapshot of the active session for autosave. While it is alive, stop is
/// blocked; drop it only after the autosave file has been written.
pub struct AutosaveTicket<'a> {
    pub gate: tokio::sync::MutexGuard<'a, ()>,
    pub session_id: String,
    pub summary: SessionSummary,
    pub delta: Vec<SensorReading>,
}

// Maximum gap between readings before we stop counting elapsed time
// (shared gap rule; prevents pauses, sensor drops or reconnects inflating duration).
use super::gap::MAX_READING_GAP_MS;

struct ActiveSession {
    id: String,
    config: SessionConfig,
    status: SessionStatus,
    metrics: MetricsCalculator,
    sensor_log: Vec<SensorReading>,
    start_time: chrono::DateTime<chrono::Utc>,
    /// Accumulated active riding time (excludes pauses and gaps > MAX_READING_GAP_SECS)
    active_elapsed_ms: u64,
    /// Wall-clock time of last processed reading (for computing deltas)
    last_reading_time: Option<Instant>,
    last_power: Option<Instant>,
    last_hr: Option<Instant>,
    last_cadence: Option<Instant>,
    last_speed: Option<Instant>,
    /// Index up to which sensor_log has been snapshotted for autosave
    autosave_cursor: usize,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            current_session: Arc::new(Mutex::new(None)),
            autosave_gate: Mutex::new(()),
        }
    }

    pub async fn start_session(&self, config: SessionConfig) -> Result<String, crate::error::AppError> {
        let mut lock = self.current_session.lock().await;
        if lock.is_some() {
            return Err(crate::error::AppError::Session("Session already active".into()));
        }
        let id = Uuid::new_v4().to_string();
        let session = ActiveSession {
            id: id.clone(),
            metrics: MetricsCalculator::new(config.ftp),
            config,
            status: SessionStatus::Running,
            sensor_log: Vec::new(),
            start_time: Utc::now(),
            active_elapsed_ms: 0,
            last_reading_time: None,
            last_power: None,
            last_hr: None,
            last_cadence: None,
            last_speed: None,
            autosave_cursor: 0,
        };
        *lock = Some(session);
        info!("Session started: {}", id);
        Ok(id)
    }

    #[allow(dead_code)]
    pub async fn stop_session(&self) -> Option<SessionSummary> {
        self.stop_session_with_log().await.map(|(summary, _)| summary)
    }

    pub async fn stop_session_with_log(
        &self,
    ) -> Option<(SessionSummary, Vec<SensorReading>)> {
        // Wait for any in-flight autosave write to finish (see `autosave_gate`).
        let _gate = self.autosave_gate.lock().await;
        let mut lock = self.current_session.lock().await;
        let session = lock.take()?;
        info!("Session stopped: {}", session.id);
        let summary = build_summary(&session);
        Some((summary, session.sensor_log))
    }

    pub async fn pause_session(&self) {
        if let Some(session) = self.current_session.lock().await.as_mut() {
            info!("Session paused: {}", session.id);
            session.status = SessionStatus::Paused;
            // Clear last_reading_time so resume doesn't count the pause gap
            session.last_reading_time = None;
        }
    }

    pub async fn resume_session(&self) {
        if let Some(session) = self.current_session.lock().await.as_mut() {
            info!("Session resumed: {}", session.id);
            session.status = SessionStatus::Running;
        }
    }

    pub async fn process_reading(&self, reading: SensorReading) {
        let mut lock = self.current_session.lock().await;
        let Some(session) = lock.as_mut() else {
            return;
        };
        if session.status != SessionStatus::Running {
            return;
        }

        // Accumulate active elapsed time (any reading type counts)
        let now = Instant::now();
        if let Some(prev) = session.last_reading_time {
            let delta_ms = prev.elapsed().as_millis() as u64;
            // Cap gap to avoid counting sensor dropouts or reconnects
            let capped = delta_ms.min(MAX_READING_GAP_MS);
            session.active_elapsed_ms += capped;
        }
        session.last_reading_time = Some(now);

        // Same entry point the one-time repair uses when replaying the raw log.
        session.metrics.record(&reading);
        match &reading {
            SensorReading::Power { .. } => session.last_power = Some(now),
            SensorReading::HeartRate { .. } => session.last_hr = Some(now),
            SensorReading::Cadence { .. } => session.last_cadence = Some(now),
            SensorReading::Speed { .. } => session.last_speed = Some(now),
            SensorReading::TrainerCommand { .. } => {} // logged to sensor_log below
        }
        session.sensor_log.push(reading);
    }

    pub async fn get_live_metrics(&self) -> Option<LiveMetrics> {
        let lock = self.current_session.lock().await;
        let session = lock.as_ref()?;
        let stale_threshold = std::time::Duration::from_secs(config::READING_FRESHNESS_SECS);
        let is_stale = |last: Option<Instant>| -> bool {
            last.is_some_and(|t| t.elapsed() > stale_threshold)
        };
        let active_secs = session.active_elapsed_ms / 1000;
        Some(LiveMetrics {
            elapsed_secs: active_secs,
            current_power: session.metrics.current_power(),
            avg_power_3s: session.metrics.avg_power(3),
            avg_power_10s: session.metrics.avg_power(10),
            avg_power_30s: session.metrics.avg_power(30),
            normalized_power: session.metrics.normalized_power(),
            tss: session.metrics.tss(active_secs),
            intensity_factor: session.metrics.intensity_factor(),
            current_hr: session.metrics.current_hr(),
            current_cadence: session.metrics.current_cadence(),
            current_speed: session.metrics.current_speed(),
            hr_zone: session.metrics.hr_zone(&session.config.hr_zones),
            power_zone: session.metrics.power_zone(session.config.ftp, &session.config.power_zones),
            stale_power: is_stale(session.last_power),
            stale_hr: is_stale(session.last_hr),
            stale_cadence: is_stale(session.last_cadence),
            stale_speed: is_stale(session.last_speed),
        })
    }

    /// Snapshot the active session for autosave without stopping it.
    /// Returns (session_id, summary, new_readings_since_last_snapshot) or None
    /// if no active session. Only clones the delta to minimize time under lock.
    pub async fn snapshot_for_autosave(&self) -> Option<(String, SessionSummary, Vec<SensorReading>)> {
        let mut lock = self.current_session.lock().await;
        let session = lock.as_mut()?;
        let summary = build_summary(session);
        let delta = session.sensor_log[session.autosave_cursor..].to_vec();
        session.autosave_cursor = session.sensor_log.len();
        Some((session.id.clone(), summary, delta))
    }

    /// Take the autosave gate and snapshot the active session. None if no
    /// session is active (e.g. stop already ran) — then nothing must be written.
    pub async fn autosave_ticket(&self) -> Option<AutosaveTicket<'_>> {
        let gate = self.autosave_gate.lock().await;
        let (session_id, summary, delta) = self.snapshot_for_autosave().await?;
        Some(AutosaveTicket {
            gate,
            session_id,
            summary,
            delta,
        })
    }

    #[allow(dead_code)]
    pub async fn is_active(&self) -> bool {
        self.current_session.lock().await.is_some()
    }

    #[allow(dead_code)]
    pub async fn get_sensor_log(&self) -> Vec<SensorReading> {
        self.current_session
            .lock()
            .await
            .as_ref()
            .map(|s| s.sensor_log.clone())
            .unwrap_or_default()
    }
}

/// Build the stored summary for a session (used by stop and autosave alike).
/// Metric fields come only from `MetricsCalculator::summary_metrics`.
fn build_summary(session: &ActiveSession) -> SessionSummary {
    let active_secs = session.active_elapsed_ms / 1000;
    let m = session.metrics.summary_metrics(active_secs);
    SessionSummary {
        id: session.id.clone(),
        start_time: session.start_time,
        duration_secs: active_secs,
        ftp: Some(session.config.ftp),
        avg_power: m.avg_power,
        max_power: m.max_power,
        normalized_power: m.normalized_power,
        tss: m.tss,
        intensity_factor: m.intensity_factor,
        avg_hr: m.avg_hr,
        max_hr: m.max_hr,
        avg_cadence: m.avg_cadence,
        avg_speed: m.avg_speed,
        work_kj: m.work_kj,
        variability_index: m.variability_index,
        distance_km: m.distance_km,
        title: None,
        activity_type: None,
        rpe: None,
        notes: None,
        zones: Some(ZoneSnapshot::from(&session.config)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::types::SessionConfig;

    fn default_config() -> SessionConfig {
        SessionConfig::default()
    }

    fn power_reading(watts: u16) -> SensorReading {
        SensorReading::Power {
            watts,
            timestamp: None,
            epoch_ms: 0,
            device_id: "test".to_string(),
            pedal_balance: None,
        }
    }

    fn power_reading_at(watts: u16, epoch_ms: u64) -> SensorReading {
        SensorReading::Power {
            watts,
            timestamp: None,
            epoch_ms,
            device_id: "test".to_string(),
            pedal_balance: None,
        }
    }

    fn hr_reading(bpm: u8) -> SensorReading {
        SensorReading::HeartRate {
            bpm,
            timestamp: None,
            epoch_ms: 0,
            device_id: "test".to_string(),
        }
    }

    #[tokio::test]
    async fn start_returns_session_id() {
        let mgr = SessionManager::new();
        let id = mgr.start_session(default_config()).await.unwrap();
        assert!(!id.is_empty());
        // UUID v4 format: 8-4-4-4-12
        assert_eq!(id.len(), 36);
    }

    #[tokio::test]
    async fn double_start_returns_error() {
        let mgr = SessionManager::new();
        mgr.start_session(default_config()).await.unwrap();
        let result = mgr.start_session(default_config()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn stop_without_start_returns_none() {
        let mgr = SessionManager::new();
        let result = mgr.stop_session().await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn process_power_and_stop_summary() {
        let mgr = SessionManager::new();
        mgr.start_session(default_config()).await.unwrap();

        mgr.process_reading(power_reading(200)).await;
        mgr.process_reading(power_reading(300)).await;

        let summary = mgr.stop_session().await.unwrap();
        // avg_power = (200 + 300) / 2 = 250
        assert_eq!(summary.avg_power, Some(250));
        assert_eq!(summary.max_power, Some(300));
    }

    #[tokio::test]
    async fn process_hr_and_stop_summary() {
        let mgr = SessionManager::new();
        mgr.start_session(default_config()).await.unwrap();

        mgr.process_reading(hr_reading(140)).await;
        mgr.process_reading(hr_reading(160)).await;

        let summary = mgr.stop_session().await.unwrap();
        assert_eq!(summary.avg_hr, Some(150));
        assert_eq!(summary.max_hr, Some(160));
    }

    #[tokio::test]
    async fn paused_session_ignores_readings() {
        let mgr = SessionManager::new();
        mgr.start_session(default_config()).await.unwrap();

        mgr.process_reading(power_reading(200)).await;
        mgr.pause_session().await;
        // These should be ignored
        mgr.process_reading(power_reading(999)).await;
        mgr.process_reading(power_reading(999)).await;
        mgr.resume_session().await;
        mgr.process_reading(power_reading(300)).await;

        let summary = mgr.stop_session().await.unwrap();
        // Only 200 and 300 counted → avg = 250
        assert_eq!(summary.avg_power, Some(250));
        assert_eq!(summary.max_power, Some(300));
    }

    #[tokio::test]
    async fn process_reading_no_session_is_noop() {
        let mgr = SessionManager::new();
        // Should not panic
        mgr.process_reading(power_reading(200)).await;
    }

    #[tokio::test]
    async fn is_active_lifecycle() {
        let mgr = SessionManager::new();
        assert!(!mgr.is_active().await);
        mgr.start_session(default_config()).await.unwrap();
        assert!(mgr.is_active().await);
        mgr.stop_session().await;
        assert!(!mgr.is_active().await);
    }

    // --- snapshot_for_autosave ---

    #[tokio::test]
    async fn snapshot_no_session_returns_none() {
        let mgr = SessionManager::new();
        assert!(mgr.snapshot_for_autosave().await.is_none());
    }

    #[tokio::test]
    async fn snapshot_returns_session_id_and_summary() {
        let mgr = SessionManager::new();
        let id = mgr.start_session(default_config()).await.unwrap();

        mgr.process_reading(power_reading(200)).await;
        mgr.process_reading(power_reading(300)).await;

        let (snap_id, summary, delta) = mgr.snapshot_for_autosave().await.unwrap();
        assert_eq!(snap_id, id);
        assert_eq!(summary.avg_power, Some(250));
        assert_eq!(summary.max_power, Some(300));
        assert_eq!(delta.len(), 2);
    }

    #[tokio::test]
    async fn snapshot_does_not_stop_session() {
        let mgr = SessionManager::new();
        mgr.start_session(default_config()).await.unwrap();
        mgr.process_reading(power_reading(200)).await;

        // Snapshot should not stop the session
        let _ = mgr.snapshot_for_autosave().await.unwrap();
        assert!(mgr.is_active().await);

        // Can still process readings after snapshot
        mgr.process_reading(power_reading(400)).await;
        let summary = mgr.stop_session().await.unwrap();
        assert_eq!(summary.max_power, Some(400));
    }

    #[tokio::test]
    async fn snapshot_returns_delta_only() {
        let mgr = SessionManager::new();
        mgr.start_session(default_config()).await.unwrap();

        mgr.process_reading(power_reading(100)).await;
        mgr.process_reading(power_reading(200)).await;

        // First snapshot: all 2 readings
        let (_, _, delta1) = mgr.snapshot_for_autosave().await.unwrap();
        assert_eq!(delta1.len(), 2);

        // Add one more reading — second snapshot returns only the new one
        mgr.process_reading(power_reading(300)).await;
        let (_, _, delta2) = mgr.snapshot_for_autosave().await.unwrap();
        assert_eq!(delta2.len(), 1);

        // No new readings — empty delta
        let (_, _, delta3) = mgr.snapshot_for_autosave().await.unwrap();
        assert_eq!(delta3.len(), 0);
    }

    #[tokio::test]
    async fn snapshot_while_paused_still_works() {
        let mgr = SessionManager::new();
        mgr.start_session(default_config()).await.unwrap();

        mgr.process_reading(power_reading(200)).await;
        mgr.pause_session().await;

        let (_, summary, delta) = mgr.snapshot_for_autosave().await.unwrap();
        assert_eq!(summary.avg_power, Some(200));
        assert_eq!(delta.len(), 1);
    }

    #[tokio::test]
    async fn snapshot_with_no_readings() {
        let mgr = SessionManager::new();
        mgr.start_session(default_config()).await.unwrap();

        let (_, summary, delta) = mgr.snapshot_for_autosave().await.unwrap();
        assert_eq!(summary.avg_power, None);
        assert_eq!(summary.max_power, None);
        assert!(delta.is_empty());
    }

    // --- NP / TSS / IF through SessionManager ---

    /// Feed 35 seconds of constant power with advancing epoch_ms timestamps.
    async fn feed_constant_power(mgr: &SessionManager, watts: u16, seconds: u64, start_sec: u64) {
        for s in 0..seconds {
            mgr.process_reading(power_reading_at(watts, (start_sec + s) * 1000))
                .await;
        }
    }

    #[tokio::test]
    async fn stop_summary_includes_np_tss_if() {
        let mgr = SessionManager::new();
        // FTP=200 in default config
        mgr.start_session(default_config()).await.unwrap();

        // 35 seconds of constant 200W — enough for NP buffer to fill
        feed_constant_power(&mgr, 200, 35, 0).await;

        let summary = mgr.stop_session().await.unwrap();

        // NP of constant 200W = 200 (cast to u16 in summary)
        assert_eq!(summary.normalized_power, Some(200));
        // IF = NP / FTP = 200 / 200 = 1.0
        let if_ = summary.intensity_factor.unwrap();
        assert!((if_ - 1.0).abs() < 0.01, "IF should be ~1.0, got {if_}");
        // TSS is Some (not None) — value is near 0 since wall-clock elapsed ≈ 0
        assert!(summary.tss.is_some(), "TSS should be computed when NP is available");
    }

    #[tokio::test]
    async fn live_metrics_include_np_tss_if() {
        let mgr = SessionManager::new();
        mgr.start_session(default_config()).await.unwrap();

        feed_constant_power(&mgr, 200, 35, 0).await;

        let live = mgr.get_live_metrics().await.unwrap();

        let np = live.normalized_power.unwrap();
        assert!((np - 200.0).abs() < 0.1, "live NP should be ~200.0, got {np}");
        let if_ = live.intensity_factor.unwrap();
        assert!((if_ - 1.0).abs() < 0.01, "live IF should be ~1.0, got {if_}");
        assert!(live.tss.is_some(), "live TSS should be computed when NP is available");
    }

    #[tokio::test]
    async fn snapshot_includes_np_tss_if() {
        let mgr = SessionManager::new();
        mgr.start_session(default_config()).await.unwrap();

        feed_constant_power(&mgr, 200, 35, 0).await;

        let (_, summary, _) = mgr.snapshot_for_autosave().await.unwrap();

        assert_eq!(summary.normalized_power, Some(200));
        let if_ = summary.intensity_factor.unwrap();
        assert!((if_ - 1.0).abs() < 0.01, "snapshot IF should be ~1.0, got {if_}");
        assert!(summary.tss.is_some());
    }

    #[tokio::test]
    async fn stop_summary_rounds_avg_power_half_up() {
        let mgr = SessionManager::new();
        mgr.start_session(default_config()).await.unwrap();
        mgr.process_reading(power_reading(199)).await;
        mgr.process_reading(power_reading(200)).await;
        // (199 + 200) / 2 = 199.5 → 200 (truncation would give 199)
        assert_eq!(mgr.stop_session().await.unwrap().avg_power, Some(200));
    }

    #[tokio::test]
    async fn stop_summary_rounds_avg_power_down() {
        let mgr = SessionManager::new();
        mgr.start_session(default_config()).await.unwrap();
        for w in [199, 199, 199, 200, 200] {
            mgr.process_reading(power_reading(w)).await;
        }
        // 997 / 5 = 199.4 → 199
        assert_eq!(mgr.stop_session().await.unwrap().avg_power, Some(199));
    }

    #[tokio::test]
    async fn stop_summary_rounds_np_to_nearest() {
        let mgr = SessionManager::new();
        mgr.start_session(default_config()).await.unwrap();
        // Seconds 0..=17 at 200W, 18..=29 at 199W, second 30 flushes second 29:
        // one rolling value = (18·200 + 12·199) / 30 = 199.6 → NP 199.6 → 200
        feed_constant_power(&mgr, 200, 18, 0).await;
        feed_constant_power(&mgr, 199, 12, 18).await;
        mgr.process_reading(power_reading_at(199, 30_000)).await;
        assert_eq!(mgr.stop_session().await.unwrap().normalized_power, Some(200));
    }

    #[tokio::test]
    async fn stop_summary_captures_zones_from_session_config() {
        let mgr = SessionManager::new();
        let mut config = default_config();
        config.power_zones = [50, 70, 85, 100, 115, 140];
        config.hr_zones = [110, 130, 150, 165, 180];
        mgr.start_session(config).await.unwrap();
        mgr.process_reading(power_reading(200)).await;
        let summary = mgr.stop_session().await.unwrap();
        assert_eq!(
            summary.zones,
            Some(crate::session::types::ZoneSnapshot {
                power_zones: [50, 70, 85, 100, 115, 140],
                hr_zones: [110, 130, 150, 165, 180],
            })
        );
    }

    #[tokio::test]
    async fn live_summary_equals_replayed_summary() {
        let mgr = SessionManager::new();
        mgr.start_session(default_config()).await.unwrap();
        // Mixed channels, a short hole and a pause-sized gap
        for s in 0..120u64 {
            if (60..70).contains(&s) {
                continue; // 10 s gap
            }
            if s == 30 || s == 31 {
                continue; // 2 s hole
            }
            let ms = s * 1000;
            mgr.process_reading(power_reading_at(150 + (s % 7) as u16 * 20, ms)).await;
            mgr.process_reading(SensorReading::HeartRate {
                bpm: 120 + (s % 11) as u8, timestamp: None, epoch_ms: ms + 300, device_id: "t".into(),
            }).await;
            mgr.process_reading(SensorReading::Cadence {
                rpm: 85.0 + (s % 5) as f32, timestamp: None, epoch_ms: ms + 500, device_id: "t".into(),
            }).await;
            mgr.process_reading(SensorReading::Speed {
                kmh: 30.0 + (s % 3) as f32, timestamp: None, epoch_ms: ms + 700, device_id: "t".into(),
            }).await;
        }
        let (summary, log) = mgr.stop_session_with_log().await.unwrap();
        let replay = crate::session::metrics::summarize_readings(
            &log, summary.ftp.unwrap(), summary.duration_secs,
        );
        let live = crate::session::metrics::SummaryMetrics {
            avg_power: summary.avg_power,
            max_power: summary.max_power,
            normalized_power: summary.normalized_power,
            tss: summary.tss,
            intensity_factor: summary.intensity_factor,
            variability_index: summary.variability_index,
            work_kj: summary.work_kj,
            distance_km: summary.distance_km,
            avg_hr: summary.avg_hr,
            max_hr: summary.max_hr,
            avg_cadence: summary.avg_cadence,
            avg_speed: summary.avg_speed,
        };
        assert!(live.normalized_power.is_some() && live.distance_km.is_some());
        assert_eq!(live, replay, "live stop summary must equal raw-log replay");
    }

    #[tokio::test]
    async fn np_none_with_insufficient_readings() {
        let mgr = SessionManager::new();
        mgr.start_session(default_config()).await.unwrap();

        // Only 25 seconds — not enough for 30-entry NP buffer
        feed_constant_power(&mgr, 200, 25, 0).await;

        let summary = mgr.stop_session().await.unwrap();
        assert!(summary.normalized_power.is_none(), "NP should be None with <31 seconds");
        assert!(summary.intensity_factor.is_none());
        assert!(summary.tss.is_none());
    }
}
