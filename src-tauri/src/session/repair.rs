//! One-time repair of stored session metrics after the pause/gap fixes.
//!
//! Replays every session's raw `.bin` log through the same code path the live
//! session uses (`metrics::summarize_readings`, `analysis::compute_power_curve_from_readings`)
//! so repaired and newly recorded numbers can't diverge. See
//! docs/design/session-metrics-pauses/design.md (ADR-8 and notes A–E).

use log::{info, warn};
use std::path::Path;
use std::sync::Arc;

use crate::error::AppError;
use crate::session::analysis;
use crate::session::metrics;
use crate::session::storage::Storage;
use crate::session::types::SessionSummary;

/// Version of the stored-metrics algorithms. Missing marker = 1 (legacy).
pub const CURRENT_METRICS_VERSION: u32 = 2;
pub const METRICS_VERSION_KEY: &str = "metrics_version";
pub const REPAIR_ATTEMPTS_KEY: &str = "metrics_repair_attempts";
/// After this many runs that still had write failures, mark done anyway.
const MAX_ATTEMPTS: u32 = 3;

#[derive(Debug, Default, PartialEq)]
pub struct RepairReport {
    pub repaired: usize,
    pub skipped: usize,
    pub failed: usize,
    /// True when the run-once marker was set by this run.
    pub completed: bool,
}

pub fn backup_file_name() -> String {
    format!("training.db.bak-metrics-v{}", CURRENT_METRICS_VERSION)
}

/// FTP to compute IF/TSS with, and whether IF/TSS should be written at all.
/// Sessions from before the `ftp` column: derive the FTP then in effect from
/// the stored NP and IF (IF = NP / FTP); if not derivable, keep IF/TSS as stored.
fn effective_ftp(session: &SessionSummary) -> (u16, bool) {
    if let Some(ftp) = session.ftp {
        return (ftp, true);
    }
    match (session.normalized_power, session.intensity_factor) {
        (Some(np), Some(if_)) if if_ > 0.0 => ((np as f32 / if_).round() as u16, true),
        // FTP only affects IF/TSS, which are not written in this case.
        _ => (1, false),
    }
}

/// Run the repair if it hasn't completed yet. Never panics; errors abort the
/// run without setting the marker so it is retried on the next launch.
pub async fn run_metrics_repair(storage: Arc<Storage>) -> Result<RepairReport, AppError> {
    let version: u32 = storage
        .get_meta(METRICS_VERSION_KEY)
        .await?
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    if version >= CURRENT_METRICS_VERSION {
        return Ok(RepairReport::default());
    }
    let marker = CURRENT_METRICS_VERSION.to_string();

    let sessions = storage.list_sessions().await?;
    if sessions.is_empty() {
        // Fresh install: nothing to repair, no backup needed.
        storage.set_meta(METRICS_VERSION_KEY, &marker).await?;
        return Ok(RepairReport { completed: true, ..Default::default() });
    }

    let attempts: u32 = storage
        .get_meta(REPAIR_ATTEMPTS_KEY)
        .await?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
        + 1;
    storage.set_meta(REPAIR_ATTEMPTS_KEY, &attempts.to_string()).await?;

    // Back up once; a resumed run keeps the original pre-repair copy.
    let backup = Path::new(storage.data_dir()).join(backup_file_name());
    if !backup.exists() {
        storage.backup_db_to(&backup).await?;
        info!("Metrics repair: backed up database to {}", backup.display());
    }

    info!(
        "Metrics repair v{}: recomputing {} session(s) (attempt {})",
        CURRENT_METRICS_VERSION,
        sessions.len(),
        attempts
    );
    let mut report = RepairReport::default();
    let mut failed_ids: Vec<String> = Vec::new();
    for session in &sessions {
        let loader = storage.clone();
        let sid = session.id.clone();
        let readings = match tokio::task::spawn_blocking(move || loader.load_sensor_data(&sid)).await {
            Ok(Ok(r)) if !r.is_empty() => r,
            Ok(Ok(_)) => {
                report.skipped += 1;
                continue;
            }
            Ok(Err(e)) => {
                warn!("Metrics repair: skipping session {} (raw log unreadable: {})", session.id, e);
                report.skipped += 1;
                continue;
            }
            Err(e) => {
                warn!("Metrics repair: loading session {} panicked: {}", session.id, e);
                report.failed += 1;
                failed_ids.push(session.id.clone());
                continue;
            }
        };

        let (ftp, update_if_and_tss) = effective_ftp(session);
        let m = metrics::summarize_readings(&readings, ftp, session.duration_secs);
        let curve = analysis::compute_power_curve_from_readings(&readings);
        match storage
            .replace_session_metrics(&session.id, &m, update_if_and_tss, &curve)
            .await
        {
            Ok(true) => report.repaired += 1,
            Ok(false) => report.skipped += 1, // deleted while the repair ran
            Err(e) => {
                warn!("Metrics repair: failed to update session {}: {}", session.id, e);
                report.failed += 1;
                failed_ids.push(session.id.clone());
            }
        }
    }

    if report.failed == 0 || attempts >= MAX_ATTEMPTS {
        if report.failed > 0 {
            warn!(
                "Metrics repair: giving up after {} attempts; sessions not repaired: {}",
                attempts,
                failed_ids.join(", ")
            );
        }
        storage.set_meta(METRICS_VERSION_KEY, &marker).await?;
        report.completed = true;
    }
    info!(
        "Metrics repair: repaired={} skipped={} failed={}{}",
        report.repaired,
        report.skipped,
        report.failed,
        if report.completed { "" } else { " (will retry on next launch)" }
    );
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::types::SensorReading;
    use crate::session::analysis::PowerCurvePoint;
    use crate::session::types::ZoneSnapshot;

    fn assert_approx(actual: f32, expected: f32, epsilon: f32, msg: &str) {
        assert!(
            (actual - expected).abs() <= epsilon,
            "{msg}: expected {expected} ± {epsilon}, got {actual}"
        );
    }

    async fn storage() -> (Arc<Storage>, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let s = Storage::new(&tmp.path().to_string_lossy()).await.unwrap();
        (Arc::new(s), tmp)
    }

    /// 200 W + 150 bpm at 0..=59 s and 70..=129 s (10 s gap); active 120 s.
    fn ride() -> Vec<SensorReading> {
        (0..60u64)
            .chain(70..130)
            .flat_map(|s| {
                [
                    SensorReading::Power {
                        watts: 200, timestamp: None, epoch_ms: s * 1000,
                        device_id: "t".into(), pedal_balance: None,
                    },
                    SensorReading::HeartRate {
                        bpm: 150, timestamp: None, epoch_ms: s * 1000 + 500, device_id: "t".into(),
                    },
                ]
            })
            .collect()
    }

    /// A row as an older build stored it: wrong (pre-fix) metrics.
    fn stale_summary(id: &str) -> SessionSummary {
        SessionSummary {
            id: id.into(),
            start_time: chrono::DateTime::parse_from_rfc3339("2024-06-15T10:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            duration_secs: 120,
            ftp: Some(200),
            avg_power: Some(150),
            max_power: Some(999),
            normalized_power: None,
            tss: None,
            intensity_factor: None,
            avg_hr: Some(100),
            max_hr: Some(201),
            avg_cadence: None,
            avg_speed: None,
            work_kj: Some(999.0),
            variability_index: None,
            distance_km: None,
            title: Some("Morning ride".into()),
            activity_type: Some("endurance".into()),
            rpe: Some(6),
            notes: Some("legs ok".into()),
            zones: Some(ZoneSnapshot {
                power_zones: [50, 70, 85, 100, 115, 140],
                hr_zones: [110, 130, 150, 165, 180],
            }),
        }
    }

    async fn save(storage: &Storage, summary: &SessionSummary, readings: &[SensorReading]) {
        let raw = bincode::serialize(readings).unwrap();
        storage.save_session(summary, &raw).await.unwrap();
    }

    async fn fail_updates_for(storage: &Storage, id: &str) {
        sqlx::raw_sql(&format!(
            "CREATE TRIGGER fail_{id} BEFORE UPDATE ON sessions WHEN NEW.id = '{id}' \
             BEGIN SELECT RAISE(ABORT, 'simulated write failure'); END;"
        ))
        .execute(&storage.pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn repair_recomputes_metrics_from_raw_log() {
        let (s, _tmp) = storage().await;
        save(&s, &stale_summary("r1"), &ride()).await;

        let report = run_metrics_repair(s.clone()).await.unwrap();

        assert_eq!(report.repaired, 1);
        let got = s.get_session("r1").await.unwrap();
        assert_eq!(got.avg_power, Some(200));
        assert_eq!(got.max_power, Some(200));
        assert_eq!(got.normalized_power, Some(200));
        assert_approx(got.intensity_factor.unwrap(), 1.0, 0.01, "IF");
        assert_approx(got.variability_index.unwrap(), 1.0, 0.01, "VI");
        // TSS = 120 × 200 × 1.0 / (200 × 3600) × 100 = 3.333
        assert_approx(got.tss.unwrap(), 3.333, 0.5, "TSS");
        // work = (59 + 5 capped + 59) s × 200 W = 24.6 kJ
        assert_approx(got.work_kj.unwrap(), 24.6, 0.1, "work");
        assert_eq!(got.avg_hr, Some(150));
        assert_eq!(got.max_hr, Some(150), "max_hr is recomputed too");
    }

    #[tokio::test]
    async fn repair_leaves_duration_ftp_and_metadata_untouched() {
        let (s, _tmp) = storage().await;
        let before = stale_summary("r2");
        save(&s, &before, &ride()).await;

        run_metrics_repair(s.clone()).await.unwrap();

        let got = s.get_session("r2").await.unwrap();
        assert_eq!(got.duration_secs, 120);
        assert_eq!(got.ftp, Some(200));
        assert_eq!(got.title, before.title);
        assert_eq!(got.activity_type, before.activity_type);
        assert_eq!(got.rpe, before.rpe);
        assert_eq!(got.notes, before.notes);
        assert_eq!(got.zones, before.zones);
    }

    #[tokio::test]
    async fn repair_rebuilds_power_curve_dropping_stale_durations() {
        let (s, _tmp) = storage().await;
        save(&s, &stale_summary("r3"), &ride()).await;
        // Fabricated 20-min best from the old hold-across-pause curve
        s.save_power_curve("r3", &[PowerCurvePoint { duration_secs: 1200, watts: 300 }])
            .await
            .unwrap();

        run_metrics_repair(s.clone()).await.unwrap();

        let best = s.get_best_power_curve(None).await.unwrap();
        assert!(best.iter().all(|p| p.duration_secs <= 60), "longest segment is 60 s");
        let p60 = best.iter().find(|p| p.duration_secs == 60).unwrap();
        assert_eq!(p60.watts, 200);
    }

    #[tokio::test]
    async fn repair_runs_once() {
        let (s, _tmp) = storage().await;
        save(&s, &stale_summary("r4"), &ride()).await;
        assert!(run_metrics_repair(s.clone()).await.unwrap().completed);
        assert_eq!(s.get_meta(METRICS_VERSION_KEY).await.unwrap().as_deref(), Some("2"));

        // Simulate a later edit; a second launch must not recompute
        sqlx::query("UPDATE sessions SET avg_power = 1 WHERE id = 'r4'")
            .execute(&s.pool)
            .await
            .unwrap();
        let second = run_metrics_repair(s.clone()).await.unwrap();

        assert_eq!(second, RepairReport::default());
        assert_eq!(s.get_session("r4").await.unwrap().avg_power, Some(1));
    }

    async fn backup_avg_power(dir: &Path, id: &str) -> Option<i64> {
        let url = format!("sqlite:{}?mode=ro", dir.join(backup_file_name()).display());
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        let row: (Option<i64>,) = sqlx::query_as("SELECT avg_power FROM sessions WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        pool.close().await;
        row.0
    }

    #[tokio::test]
    async fn repair_creates_backup_before_writing() {
        let (s, tmp) = storage().await;
        save(&s, &stale_summary("r5"), &ride()).await;

        run_metrics_repair(s.clone()).await.unwrap();

        assert_eq!(backup_avg_power(tmp.path(), "r5").await, Some(150), "pre-repair value");
        assert_eq!(s.get_session("r5").await.unwrap().avg_power, Some(200));
    }

    #[tokio::test]
    async fn repair_rerun_after_interruption_is_idempotent() {
        let (s, tmp) = storage().await;
        save(&s, &stale_summary("r6"), &ride()).await;
        run_metrics_repair(s.clone()).await.unwrap();
        let first = s.get_session("r6").await.unwrap();

        // Interrupted before the marker was written → next launch runs again
        sqlx::query("DELETE FROM app_meta").execute(&s.pool).await.unwrap();
        let again = run_metrics_repair(s.clone()).await.unwrap();

        assert!(again.completed);
        let second = s.get_session("r6").await.unwrap();
        assert_eq!(second.normalized_power, first.normalized_power);
        assert_eq!(second.work_kj, first.work_kj);
        assert_eq!(second.tss, first.tss);
        // Existing backup (pre-repair copy) is never overwritten
        assert_eq!(backup_avg_power(tmp.path(), "r6").await, Some(150));
    }

    #[tokio::test]
    async fn repair_skips_missing_raw_log_and_still_sets_marker() {
        let (s, tmp) = storage().await;
        save(&s, &stale_summary("r7"), &ride()).await;
        std::fs::remove_file(tmp.path().join("sessions").join("r7.bin")).unwrap();

        let report = run_metrics_repair(s.clone()).await.unwrap();

        assert_eq!((report.repaired, report.skipped, report.failed), (0, 1, 0));
        assert!(report.completed);
        assert_eq!(s.get_session("r7").await.unwrap().avg_power, Some(150), "kept as stored");
    }

    #[tokio::test]
    async fn repair_skips_corrupt_raw_log() {
        let (s, _tmp) = storage().await;
        s.save_session(&stale_summary("r8"), b"\x01not bincode").await.unwrap();
        save(&s, &stale_summary("r8b"), &ride()).await;

        let report = run_metrics_repair(s.clone()).await.unwrap();

        assert_eq!((report.repaired, report.skipped, report.failed), (1, 1, 0));
        assert_eq!(s.get_session("r8").await.unwrap().avg_power, Some(150));
    }

    #[tokio::test]
    async fn repair_does_not_set_marker_when_a_write_fails() {
        let (s, _tmp) = storage().await;
        save(&s, &stale_summary("ok1"), &ride()).await;
        save(&s, &stale_summary("bad1"), &ride()).await;
        fail_updates_for(&s, "bad1").await;

        let report = run_metrics_repair(s.clone()).await.unwrap();

        assert_eq!((report.repaired, report.failed), (1, 1));
        assert!(!report.completed);
        assert_eq!(s.get_meta(METRICS_VERSION_KEY).await.unwrap(), None);
        assert_eq!(s.get_meta(REPAIR_ATTEMPTS_KEY).await.unwrap().as_deref(), Some("1"));
    }

    #[tokio::test]
    async fn repair_gives_up_after_three_failed_attempts() {
        let (s, _tmp) = storage().await;
        save(&s, &stale_summary("bad2"), &ride()).await;
        fail_updates_for(&s, "bad2").await;

        assert!(!run_metrics_repair(s.clone()).await.unwrap().completed, "attempt 1");
        assert!(!run_metrics_repair(s.clone()).await.unwrap().completed, "attempt 2");
        let third = run_metrics_repair(s.clone()).await.unwrap();

        assert_eq!(third.failed, 1);
        assert!(third.completed, "3rd failed attempt sets the marker anyway");
        assert_eq!(s.get_meta(METRICS_VERSION_KEY).await.unwrap().as_deref(), Some("2"));
    }

    #[tokio::test]
    async fn repair_fresh_db_sets_marker_without_backup() {
        let (s, tmp) = storage().await;

        let report = run_metrics_repair(s.clone()).await.unwrap();

        assert!(report.completed);
        assert_eq!(s.get_meta(METRICS_VERSION_KEY).await.unwrap().as_deref(), Some("2"));
        assert!(!tmp.path().join(backup_file_name()).exists());
    }

    #[tokio::test]
    async fn repair_null_ftp_derives_ftp_from_stored_np_and_if() {
        let (s, _tmp) = storage().await;
        let mut old = stale_summary("r9");
        old.ftp = None;
        // Stored by the old code with FTP 250: IF = 250 / 250 → derived FTP 250
        old.normalized_power = Some(250);
        old.intensity_factor = Some(1.0);
        save(&s, &old, &ride()).await;

        run_metrics_repair(s.clone()).await.unwrap();

        let got = s.get_session("r9").await.unwrap();
        assert_eq!(got.ftp, None, "derived FTP is not written");
        assert_eq!(got.normalized_power, Some(200));
        // IF = 200 / 250 = 0.8; TSS = 120 × 200 × 0.8 / (250 × 3600) × 100 = 2.133
        assert_approx(got.intensity_factor.unwrap(), 0.8, 0.01, "IF with derived FTP");
        assert_approx(got.tss.unwrap(), 2.133, 0.5, "TSS with derived FTP");
    }

    #[tokio::test]
    async fn repair_null_ftp_without_if_keeps_stored_if_and_tss() {
        let (s, _tmp) = storage().await;
        let mut old = stale_summary("r10");
        old.ftp = None;
        old.intensity_factor = None;
        old.tss = Some(42.0);
        save(&s, &old, &ride()).await;

        run_metrics_repair(s.clone()).await.unwrap();

        let got = s.get_session("r10").await.unwrap();
        assert_eq!(got.intensity_factor, None, "IF kept as stored");
        assert_approx(got.tss.unwrap(), 42.0, 0.01, "TSS kept as stored");
        assert_eq!(got.normalized_power, Some(200), "other metrics still recomputed");
    }

    #[tokio::test]
    async fn replace_metrics_for_deleted_session_writes_nothing() {
        let (s, _tmp) = storage().await;
        let m = metrics::summarize_readings(&ride(), 200, 120);
        let curve = analysis::compute_power_curve_from_readings(&ride());

        let written = s.replace_session_metrics("gone", &m, true, &curve).await.unwrap();

        assert!(!written);
        assert!(!s.has_power_curve("gone").await.unwrap());
    }
}
