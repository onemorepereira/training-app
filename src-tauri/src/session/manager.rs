use chrono::Utc;
use log::info;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::metrics::MetricsCalculator;
use super::types::*;
use crate::device::types::SensorReading;

/// Data is considered stale after this many seconds without a new reading.
const STALE_THRESHOLD_SECS: u64 = 5;

pub struct SessionManager {
    current_session: Arc<Mutex<Option<ActiveSession>>>,
}

/// Maximum gap between readings before we stop counting elapsed time.
/// Prevents pauses, sensor drops, or reconnects from inflating duration.
const MAX_READING_GAP_SECS: u64 = 5;

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
        let mut lock = self.current_session.lock().await;
        let session = lock.take()?;
        info!("Session stopped: {}", session.id);
        let active_secs = session.active_elapsed_ms / 1000;
        let summary = SessionSummary {
            id: session.id,
            start_time: session.start_time,
            duration_secs: active_secs,
            ftp: Some(session.config.ftp),
            avg_power: session.metrics.avg_power(usize::MAX).map(|v| v as u16),
            max_power: session.metrics.max_power(),
            normalized_power: session.metrics.normalized_power().map(|v| v as u16),
            tss: session.metrics.tss(active_secs),
            intensity_factor: session.metrics.intensity_factor(),
            avg_hr: session.metrics.avg_hr(),
            max_hr: session.metrics.max_hr(),
            avg_cadence: session.metrics.avg_cadence(),
            avg_speed: session.metrics.avg_speed(),
            title: None,
            activity_type: None,
            rpe: None,
            notes: None,
        };
        Some((summary, session.sensor_log))
    }

    pub async fn pause_session(&self) {
        if let Some(session) = self.current_session.lock().await.as_mut() {
            session.status = SessionStatus::Paused;
            // Clear last_reading_time so resume doesn't count the pause gap
            session.last_reading_time = None;
        }
    }

    pub async fn resume_session(&self) {
        if let Some(session) = self.current_session.lock().await.as_mut() {
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
            let capped = delta_ms.min(MAX_READING_GAP_SECS * 1000);
            session.active_elapsed_ms += capped;
        }
        session.last_reading_time = Some(now);

        match &reading {
            SensorReading::Power {
                watts, epoch_ms, ..
            } => {
                session.metrics.record_power(*watts, *epoch_ms);
                session.last_power = Some(now);
            }
            SensorReading::HeartRate { bpm, .. } => {
                session.metrics.record_hr(*bpm);
                session.last_hr = Some(now);
            }
            SensorReading::Cadence { rpm, .. } => {
                session.metrics.record_cadence(*rpm);
                session.last_cadence = Some(now);
            }
            SensorReading::Speed { kmh, .. } => {
                session.metrics.record_speed(*kmh);
                session.last_speed = Some(now);
            }
        }
        session.sensor_log.push(reading);
    }

    pub async fn get_live_metrics(&self) -> Option<LiveMetrics> {
        let lock = self.current_session.lock().await;
        let session = lock.as_ref()?;
        let stale_threshold = std::time::Duration::from_secs(STALE_THRESHOLD_SECS);
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
        let active_secs = session.active_elapsed_ms / 1000;
        let summary = SessionSummary {
            id: session.id.clone(),
            start_time: session.start_time,
            duration_secs: active_secs,
            ftp: Some(session.config.ftp),
            avg_power: session.metrics.avg_power(usize::MAX).map(|v| v as u16),
            max_power: session.metrics.max_power(),
            normalized_power: session.metrics.normalized_power().map(|v| v as u16),
            tss: session.metrics.tss(active_secs),
            intensity_factor: session.metrics.intensity_factor(),
            avg_hr: session.metrics.avg_hr(),
            max_hr: session.metrics.max_hr(),
            avg_cadence: session.metrics.avg_cadence(),
            avg_speed: session.metrics.avg_speed(),
            title: None,
            activity_type: None,
            rpe: None,
            notes: None,
        };
        let delta = session.sensor_log[session.autosave_cursor..].to_vec();
        session.autosave_cursor = session.sensor_log.len();
        Some((session.id.clone(), summary, delta))
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
}
