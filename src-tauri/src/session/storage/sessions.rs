use log::{debug, error, info, warn};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use super::fsio::{io_error, remove_durable, write_durable};
use super::Storage;
use crate::device::types::{CommandSource, SensorReading};
use crate::error::AppError;
use crate::session::types::SessionSummary;

/// Legacy sensor reading format: Power variant lacked pedal_balance field because
/// #[serde(skip_serializing_if)] silently dropped it from bincode output.
#[derive(Deserialize)]
enum LegacySensorReading {
    Power {
        watts: u16,
        epoch_ms: u64,
        #[serde(default)]
        device_id: String,
    },
    HeartRate {
        bpm: u8,
        epoch_ms: u64,
        #[serde(default)]
        device_id: String,
    },
    Cadence {
        rpm: f32,
        epoch_ms: u64,
        #[serde(default)]
        device_id: String,
    },
    Speed {
        kmh: f32,
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

impl From<LegacySensorReading> for SensorReading {
    fn from(legacy: LegacySensorReading) -> Self {
        match legacy {
            LegacySensorReading::Power { watts, epoch_ms, device_id } => SensorReading::Power {
                watts,
                timestamp: None,
                epoch_ms,
                device_id,
                pedal_balance: None,
            },
            LegacySensorReading::HeartRate { bpm, epoch_ms, device_id } => {
                SensorReading::HeartRate { bpm, timestamp: None, epoch_ms, device_id }
            }
            LegacySensorReading::Cadence { rpm, epoch_ms, device_id } => {
                SensorReading::Cadence { rpm, timestamp: None, epoch_ms, device_id }
            }
            LegacySensorReading::Speed { kmh, epoch_ms, device_id } => {
                SensorReading::Speed { kmh, timestamp: None, epoch_ms, device_id }
            }
            LegacySensorReading::TrainerCommand { target_watts, epoch_ms, source } => {
                SensorReading::TrainerCommand { target_watts, epoch_ms, source }
            }
        }
    }
}

/// Result of [`Storage::save_session`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    /// New row inserted (raw file written or already identical on disk).
    Inserted,
    /// Row and raw file already existed; nothing was written.
    AlreadyExisted,
    /// Row existed without a raw file (legacy crash); file written, metrics updated.
    RepairedRaw,
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: String,
    start_time: String,
    duration_secs: i64,
    ftp: Option<i32>,
    avg_power: Option<i32>,
    max_power: Option<i32>,
    normalized_power: Option<i32>,
    tss: Option<f64>,
    intensity_factor: Option<f64>,
    avg_hr: Option<i32>,
    max_hr: Option<i32>,
    avg_cadence: Option<f64>,
    avg_speed: Option<f64>,
    work_kj: Option<f64>,
    variability_index: Option<f64>,
    distance_km: Option<f64>,
    title: Option<String>,
    activity_type: Option<String>,
    rpe: Option<i32>,
    notes: Option<String>,
}

impl TryFrom<SessionRow> for SessionSummary {
    type Error = AppError;

    fn try_from(row: SessionRow) -> Result<Self, Self::Error> {
        let start_time = chrono::DateTime::parse_from_rfc3339(&row.start_time)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .map_err(|e| {
                AppError::Session(format!(
                    "Invalid start_time '{}' for session {}: {}",
                    row.start_time, row.id, e
                ))
            })?;
        Ok(Self {
            id: row.id,
            start_time,
            duration_secs: row.duration_secs as u64,
            ftp: row.ftp.map(|v| v as u16),
            avg_power: row.avg_power.map(|v| v as u16),
            max_power: row.max_power.map(|v| v as u16),
            normalized_power: row.normalized_power.map(|v| v as u16),
            tss: row.tss.map(|v| v as f32),
            intensity_factor: row.intensity_factor.map(|v| v as f32),
            avg_hr: row.avg_hr.map(|v| v as u8),
            max_hr: row.max_hr.map(|v| v as u8),
            avg_cadence: row.avg_cadence.map(|v| v as f32),
            avg_speed: row.avg_speed.map(|v| v as f32),
            work_kj: row.work_kj.map(|v| v as f32),
            variability_index: row.variability_index.map(|v| v as f32),
            distance_km: row.distance_km.map(|v| v as f32),
            title: row.title,
            activity_type: row.activity_type,
            rpe: row.rpe.map(|v| v as u8),
            notes: row.notes,
        })
    }
}

impl Storage {
    pub(crate) fn sessions_dir(&self) -> PathBuf {
        Path::new(&self.data_dir).join("sessions")
    }

    pub(crate) fn raw_path(&self, session_id: &str) -> PathBuf {
        self.sessions_dir().join(format!("{}.bin", session_id))
    }

    pub(crate) async fn session_exists(&self, session_id: &str) -> Result<bool, AppError> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM sessions WHERE id = ?")
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(row.is_some())
    }

    /// Persist a finished session so that the row's summary always describes the
    /// raw file on disk. Never overwrites an existing session's raw log.
    ///
    /// Ordering: raw file is made durable first, then the row is inserted. So a
    /// row implies its raw file exists. Cases (single writer per id is guaranteed:
    /// recovery runs before any command; live sessions have fresh UUIDs):
    ///
    /// | row    | raw file           | action                                        |
    /// |--------|--------------------|-----------------------------------------------|
    /// | exists | exists             | nothing (`AlreadyExisted`)                    |
    /// | exists | absent (legacy)    | write raw, update row metrics (`RepairedRaw`) |
    /// | absent | absent / identical | write raw if needed, insert row (`Inserted`)  |
    /// | absent | differs            | keep old as `.orphan-<ms>`, write, insert     |
    pub async fn save_session(
        &self,
        summary: &SessionSummary,
        raw_data: &[u8],
    ) -> Result<SaveOutcome, AppError> {
        info!(
            "Saving session: id={}, duration={}s",
            summary.id, summary.duration_secs
        );
        let raw_file = self.raw_path(&summary.id);

        if self.session_exists(&summary.id).await? {
            let raw_present = tokio::fs::try_exists(&raw_file)
                .await
                .map_err(|e| io_error("Failed to stat", &raw_file, e))?;
            if raw_present {
                return Ok(SaveOutcome::AlreadyExisted);
            }
            warn!(
                "Session {} has a row but no raw file; restoring it from the supplied data",
                summary.id
            );
            write_durable(&raw_file, raw_data).await?;
            self.update_session_summary(summary).await?;
            return Ok(SaveOutcome::RepairedRaw);
        }

        match tokio::fs::read(&raw_file).await {
            Ok(existing) if existing == raw_data => {}
            Ok(_) => {
                let ms = chrono::Utc::now().timestamp_millis();
                let orphan = self
                    .sessions_dir()
                    .join(format!("{}.bin.orphan-{}", summary.id, ms));
                warn!(
                    "Raw file for unsaved session {} differs from the data being saved; \
                     keeping it as {}",
                    summary.id,
                    orphan.display()
                );
                tokio::fs::rename(&raw_file, &orphan)
                    .await
                    .map_err(|e| io_error("Failed to move aside", &raw_file, e))?;
                write_durable(&raw_file, raw_data).await?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                write_durable(&raw_file, raw_data).await?;
            }
            Err(e) => return Err(io_error("Failed to read", &raw_file, e)),
        }

        let raw_file_path = raw_file.to_string_lossy().to_string();
        let result = sqlx::query(
            "INSERT OR IGNORE INTO sessions (id, start_time, duration_secs, ftp, avg_power, max_power, \
             normalized_power, tss, intensity_factor, avg_hr, max_hr, avg_cadence, avg_speed, \
             work_kj, variability_index, distance_km, \
             raw_file_path, title, activity_type, rpe, notes) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&summary.id)
        .bind(summary.start_time.to_rfc3339())
        .bind(summary.duration_secs as i64)
        .bind(summary.ftp.map(|v| v as i32))
        .bind(summary.avg_power.map(|v| v as i32))
        .bind(summary.max_power.map(|v| v as i32))
        .bind(summary.normalized_power.map(|v| v as i32))
        .bind(summary.tss)
        .bind(summary.intensity_factor)
        .bind(summary.avg_hr.map(|v| v as i32))
        .bind(summary.max_hr.map(|v| v as i32))
        .bind(summary.avg_cadence)
        .bind(summary.avg_speed)
        .bind(summary.work_kj.map(|v| v as f64))
        .bind(summary.variability_index.map(|v| v as f64))
        .bind(summary.distance_km.map(|v| v as f64))
        .bind(&raw_file_path)
        .bind(&summary.title)
        .bind(&summary.activity_type)
        .bind(summary.rpe.map(|v| v as i32))
        .bind(&summary.notes)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(if result.rows_affected() == 1 {
            SaveOutcome::Inserted
        } else {
            SaveOutcome::AlreadyExisted
        })
    }

    /// Overwrite a row's computed metrics (not user metadata) from `summary`.
    async fn update_session_summary(&self, summary: &SessionSummary) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE sessions SET start_time = ?, duration_secs = ?, ftp = ?, avg_power = ?, \
             max_power = ?, normalized_power = ?, tss = ?, intensity_factor = ?, avg_hr = ?, \
             max_hr = ?, avg_cadence = ?, avg_speed = ?, work_kj = ?, variability_index = ?, \
             distance_km = ?, raw_file_path = ? WHERE id = ?",
        )
        .bind(summary.start_time.to_rfc3339())
        .bind(summary.duration_secs as i64)
        .bind(summary.ftp.map(|v| v as i32))
        .bind(summary.avg_power.map(|v| v as i32))
        .bind(summary.max_power.map(|v| v as i32))
        .bind(summary.normalized_power.map(|v| v as i32))
        .bind(summary.tss)
        .bind(summary.intensity_factor)
        .bind(summary.avg_hr.map(|v| v as i32))
        .bind(summary.max_hr.map(|v| v as i32))
        .bind(summary.avg_cadence)
        .bind(summary.avg_speed)
        .bind(summary.work_kj.map(|v| v as f64))
        .bind(summary.variability_index.map(|v| v as f64))
        .bind(summary.distance_km.map(|v| v as f64))
        .bind(self.raw_path(&summary.id).to_string_lossy().to_string())
        .bind(&summary.id)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionSummary>, AppError> {
        let rows = sqlx::query_as::<_, SessionRow>(
            "SELECT id, start_time, duration_secs, ftp, avg_power, max_power, normalized_power, tss, \
             intensity_factor, avg_hr, max_hr, avg_cadence, avg_speed, work_kj, variability_index, \
             distance_km, title, activity_type, rpe, notes FROM sessions ORDER BY start_time DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;
        rows.into_iter().map(|r| r.try_into()).collect()
    }

    pub async fn get_session(&self, session_id: &str) -> Result<SessionSummary, AppError> {
        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT id, start_time, duration_secs, ftp, avg_power, max_power, normalized_power, tss, \
             intensity_factor, avg_hr, max_hr, avg_cadence, avg_speed, work_kj, variability_index, \
             distance_km, title, activity_type, rpe, notes FROM sessions WHERE id = ?",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await
        .map_err(AppError::Database)?;
        row.try_into()
    }

    pub fn load_sensor_data(&self, session_id: &str) -> Result<Vec<SensorReading>, AppError> {
        let raw_file = Path::new(&self.data_dir)
            .join("sessions")
            .join(format!("{}.bin", session_id));
        let data = std::fs::read(&raw_file)
            .map_err(|e| AppError::Serialization(format!("Failed to read sensor data: {}", e)))?;

        // Try current format first; fall back to legacy format (before pedal_balance
        // was added to Power). The old code used #[serde(skip_serializing_if)] on
        // pedal_balance which is broken with bincode — it omitted the field during
        // serialization but the deserializer always expected it.
        bincode::deserialize::<Vec<SensorReading>>(&data).or_else(|_| {
            debug!("Using legacy format fallback for session {}", session_id);
            let legacy: Vec<LegacySensorReading> = bincode::deserialize(&data)
                .map_err(|e| {
                    AppError::Serialization(format!(
                        "Failed to deserialize sensor data: {}",
                        e
                    ))
                })?;
            Ok(legacy.into_iter().map(SensorReading::from).collect())
        })
    }

    pub async fn update_session_metadata(
        &self,
        session_id: &str,
        title: Option<String>,
        activity_type: Option<String>,
        rpe: Option<u8>,
        notes: Option<String>,
    ) -> Result<(), AppError> {
        let result = sqlx::query(
            "UPDATE sessions SET \
               title = COALESCE(?, title), \
               activity_type = COALESCE(?, activity_type), \
               rpe = COALESCE(?, rpe), \
               notes = COALESCE(?, notes) \
             WHERE id = ?",
        )
        .bind(&title)
        .bind(&activity_type)
        .bind(rpe.map(|v| v as i32))
        .bind(&notes)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        if result.rows_affected() == 0 {
            return Err(AppError::Session(format!("Session not found: {}", session_id)));
        }
        Ok(())
    }

    /// Delete a session. Order matters (see design ADR-3):
    /// 1. remove any leftover autosave for the id — if that fails, abort with
    ///    nothing changed, otherwise a later recovery could resurrect the ride;
    /// 2. delete power-curve rows and the session row in ONE transaction;
    /// 3. remove the raw file — failure here only leaves an invisible orphan file,
    ///    so it is logged and the delete still succeeds.
    pub async fn delete_session(&self, session_id: &str) -> Result<(), AppError> {
        info!("Deleting session: {}", session_id);
        self.remove_autosave(session_id).await?;

        let mut tx = self.pool.begin().await.map_err(AppError::Database)?;
        sqlx::query("DELETE FROM session_power_curves WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)?;

        let path = self.raw_path(session_id);
        if let Err(e) = remove_durable(&path).await {
            error!(
                "Session {} deleted from the database but its raw file remains: {}",
                session_id, e
            );
        }
        Ok(())
    }

    pub async fn save_zone_config(
        &self,
        session_id: &str,
        zone_config: &str,
    ) -> Result<(), AppError> {
        let result = sqlx::query("UPDATE sessions SET zone_config = ? WHERE id = ?")
            .bind(zone_config)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        if result.rows_affected() == 0 {
            return Err(AppError::Session(format!("Session not found: {}", session_id)));
        }
        Ok(())
    }

    pub async fn get_zone_config(&self, session_id: &str) -> Result<Option<String>, AppError> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT zone_config FROM sessions WHERE id = ?")
                .bind(session_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(AppError::Database)?;
        Ok(row.and_then(|(v,)| v))
    }
}
