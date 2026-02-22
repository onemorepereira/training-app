use log::{info, warn};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;

use super::analysis::PowerCurvePoint;
use super::types::{SessionConfig, SessionSummary};
use serde::Deserialize;

use crate::commands::validate_session_id;
use crate::device::types::{CommandSource, ConnectionStatus, DeviceInfo, DeviceType, SensorReading, Transport};
use crate::error::AppError;

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

/// Execute an ALTER TABLE statement, ignoring "duplicate column" errors (expected
/// on re-run) but propagating all other errors (disk full, corruption, malformed SQL).
async fn run_alter_ignore_duplicate(pool: &SqlitePool, stmt: &str) -> Result<(), AppError> {
    match sqlx::raw_sql(stmt).execute(pool).await {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().contains("duplicate column name") => Ok(()),
        Err(e) => Err(AppError::Database(e)),
    }
}

pub struct Storage {
    pool: SqlitePool,
    data_dir: String,
}

impl Storage {
    pub async fn new(data_dir: &str) -> Result<Self, AppError> {
        std::fs::create_dir_all(data_dir).map_err(|e| AppError::Database(sqlx::Error::Io(e)))?;
        let db_path = Path::new(data_dir).join("training.db");
        let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
        let options = SqliteConnectOptions::from_str(&db_url)
            .map_err(AppError::Database)?
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .map_err(AppError::Database)?;
        let migration_sql = include_str!("../../migrations/001_init.sql");
        sqlx::raw_sql(migration_sql)
            .execute(&pool)
            .await
            .map_err(AppError::Database)?;
        // Run each ALTER TABLE individually, ignoring "duplicate column" errors on
        // re-run but propagating real failures (disk full, corruption, etc.)
        let migration_002_stmts = [
            "ALTER TABLE user_config ADD COLUMN units TEXT NOT NULL DEFAULT 'metric'",
            "ALTER TABLE user_config ADD COLUMN power_zone_1 INTEGER NOT NULL DEFAULT 55",
            "ALTER TABLE user_config ADD COLUMN power_zone_2 INTEGER NOT NULL DEFAULT 75",
            "ALTER TABLE user_config ADD COLUMN power_zone_3 INTEGER NOT NULL DEFAULT 90",
            "ALTER TABLE user_config ADD COLUMN power_zone_4 INTEGER NOT NULL DEFAULT 105",
            "ALTER TABLE user_config ADD COLUMN power_zone_5 INTEGER NOT NULL DEFAULT 120",
            "ALTER TABLE user_config ADD COLUMN power_zone_6 INTEGER NOT NULL DEFAULT 150",
        ];
        for stmt in migration_002_stmts {
            run_alter_ignore_duplicate(&pool, stmt).await?;
        }
        let migration_003_stmts = [
            "ALTER TABLE user_config ADD COLUMN date_of_birth TEXT",
            "ALTER TABLE user_config ADD COLUMN sex TEXT",
            "ALTER TABLE user_config ADD COLUMN resting_hr INTEGER",
            "ALTER TABLE user_config ADD COLUMN max_hr INTEGER",
        ];
        for stmt in migration_003_stmts {
            run_alter_ignore_duplicate(&pool, stmt).await?;
        }
        // Migration 004: store FTP used in each session for audit trail
        run_alter_ignore_duplicate(&pool, "ALTER TABLE sessions ADD COLUMN ftp INTEGER").await?;
        // Migration 005: device metadata for cross-transport deduplication
        let migration_005_stmts = [
            "ALTER TABLE known_devices ADD COLUMN device_group TEXT",
            "ALTER TABLE known_devices ADD COLUMN manufacturer TEXT",
            "ALTER TABLE known_devices ADD COLUMN model_number TEXT",
            "ALTER TABLE known_devices ADD COLUMN serial_number TEXT",
        ];
        for stmt in migration_005_stmts {
            run_alter_ignore_duplicate(&pool, stmt).await?;
        }
        // Migration 006: activity metadata on sessions
        let migration_006_stmts = [
            "ALTER TABLE sessions ADD COLUMN title TEXT",
            "ALTER TABLE sessions ADD COLUMN activity_type TEXT",
            "ALTER TABLE sessions ADD COLUMN rpe INTEGER",
            "ALTER TABLE sessions ADD COLUMN notes TEXT",
        ];
        for stmt in migration_006_stmts {
            run_alter_ignore_duplicate(&pool, stmt).await?;
        }
        // Migration 008: work (kJ) and variability index
        let migration_008_stmts = [
            "ALTER TABLE sessions ADD COLUMN work_kj REAL",
            "ALTER TABLE sessions ADD COLUMN variability_index REAL",
        ];
        for stmt in migration_008_stmts {
            run_alter_ignore_duplicate(&pool, stmt).await?;
        }
        // Migration 009: distance
        run_alter_ignore_duplicate(
            &pool,
            "ALTER TABLE sessions ADD COLUMN distance_km REAL",
        )
        .await?;
        // Power curve cache table (idempotent CREATE IF NOT EXISTS)
        sqlx::raw_sql(
            "CREATE TABLE IF NOT EXISTS session_power_curves (
                session_id TEXT NOT NULL,
                duration_secs INTEGER NOT NULL,
                watts INTEGER NOT NULL,
                PRIMARY KEY (session_id, duration_secs)
            )"
        )
        .execute(&pool)
        .await
        .map_err(AppError::Database)?;
        Ok(Self {
            pool,
            data_dir: data_dir.to_string(),
        })
    }

    pub async fn save_session(
        &self,
        summary: &SessionSummary,
        raw_data: &[u8],
    ) -> Result<(), AppError> {
        let raw_file = Path::new(&self.data_dir)
            .join("sessions")
            .join(format!("{}.bin", summary.id));
        let raw_file_path = raw_file.to_string_lossy().to_string();
        let start_time = summary.start_time.to_rfc3339();
        let duration_secs = summary.duration_secs as i64;
        let avg_power = summary.avg_power.map(|v| v as i32);
        let max_power = summary.max_power.map(|v| v as i32);
        let np = summary.normalized_power.map(|v| v as i32);
        let avg_hr = summary.avg_hr.map(|v| v as i32);
        let max_hr = summary.max_hr.map(|v| v as i32);
        // INSERT first — a row without a file is visible in history;
        // a file without a row is invisible (data loss on crash).
        let ftp = summary.ftp.map(|v| v as i32);
        sqlx::query(
            "INSERT OR IGNORE INTO sessions (id, start_time, duration_secs, ftp, avg_power, max_power, \
             normalized_power, tss, intensity_factor, avg_hr, max_hr, avg_cadence, avg_speed, \
             work_kj, variability_index, distance_km, \
             raw_file_path, title, activity_type, rpe, notes) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&summary.id)
        .bind(&start_time)
        .bind(duration_secs)
        .bind(ftp)
        .bind(avg_power)
        .bind(max_power)
        .bind(np)
        .bind(summary.tss)
        .bind(summary.intensity_factor)
        .bind(avg_hr)
        .bind(max_hr)
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
        // Write raw data file after DB row exists
        tokio::fs::create_dir_all(raw_file.parent().unwrap())
            .await
            .map_err(|e| AppError::Database(sqlx::Error::Io(e)))?;
        tokio::fs::write(&raw_file, raw_data)
            .await
            .map_err(|e| AppError::Database(sqlx::Error::Io(e)))?;
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

    pub async fn get_user_config(&self) -> Result<SessionConfig, AppError> {
        let row = sqlx::query_as::<_, ConfigRow>(
            "SELECT ftp, weight_kg, hr_zone_1, hr_zone_2, hr_zone_3, hr_zone_4, hr_zone_5, \
             units, power_zone_1, power_zone_2, power_zone_3, power_zone_4, power_zone_5, \
             power_zone_6, date_of_birth, sex, resting_hr, max_hr \
             FROM user_config WHERE id = 1",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(SessionConfig {
            ftp: row.ftp as u16,
            weight_kg: row.weight_kg as f32,
            hr_zones: [
                row.hr_zone_1 as u8,
                row.hr_zone_2 as u8,
                row.hr_zone_3 as u8,
                row.hr_zone_4 as u8,
                row.hr_zone_5 as u8,
            ],
            units: row.units,
            power_zones: [
                row.power_zone_1 as u16,
                row.power_zone_2 as u16,
                row.power_zone_3 as u16,
                row.power_zone_4 as u16,
                row.power_zone_5 as u16,
                row.power_zone_6 as u16,
            ],
            date_of_birth: row.date_of_birth,
            sex: row.sex,
            resting_hr: row.resting_hr.map(|v| v as u8),
            max_hr: row.max_hr.map(|v| v as u8),
        })
    }

    pub async fn save_user_config(&self, config: &SessionConfig) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO user_config (id, ftp, weight_kg, hr_zone_1, hr_zone_2, hr_zone_3, \
             hr_zone_4, hr_zone_5, units, power_zone_1, power_zone_2, power_zone_3, \
             power_zone_4, power_zone_5, power_zone_6, date_of_birth, sex, resting_hr, max_hr) \
             VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
             ftp = excluded.ftp, weight_kg = excluded.weight_kg, \
             hr_zone_1 = excluded.hr_zone_1, hr_zone_2 = excluded.hr_zone_2, \
             hr_zone_3 = excluded.hr_zone_3, hr_zone_4 = excluded.hr_zone_4, \
             hr_zone_5 = excluded.hr_zone_5, units = excluded.units, \
             power_zone_1 = excluded.power_zone_1, power_zone_2 = excluded.power_zone_2, \
             power_zone_3 = excluded.power_zone_3, power_zone_4 = excluded.power_zone_4, \
             power_zone_5 = excluded.power_zone_5, power_zone_6 = excluded.power_zone_6, \
             date_of_birth = excluded.date_of_birth, sex = excluded.sex, \
             resting_hr = excluded.resting_hr, max_hr = excluded.max_hr",
        )
        .bind(config.ftp as i32)
        .bind(config.weight_kg as f64)
        .bind(config.hr_zones[0] as i32)
        .bind(config.hr_zones[1] as i32)
        .bind(config.hr_zones[2] as i32)
        .bind(config.hr_zones[3] as i32)
        .bind(config.hr_zones[4] as i32)
        .bind(&config.units)
        .bind(config.power_zones[0] as i32)
        .bind(config.power_zones[1] as i32)
        .bind(config.power_zones[2] as i32)
        .bind(config.power_zones[3] as i32)
        .bind(config.power_zones[4] as i32)
        .bind(config.power_zones[5] as i32)
        .bind(&config.date_of_birth)
        .bind(&config.sex)
        .bind(config.resting_hr.map(|v| v as i32))
        .bind(config.max_hr.map(|v| v as i32))
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }

    #[cfg(test)]
    pub async fn upsert_known_device(&self, device: &DeviceInfo) -> Result<(), AppError> {
        let device_type = device.device_type.as_str();
        let transport = device.transport.as_str();
        let last_seen = device
            .last_seen
            .clone()
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
        sqlx::query(
            "INSERT INTO known_devices (id, name, device_type, transport, rssi, battery_level, \
             last_seen, manufacturer, model_number, serial_number, device_group) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               name = COALESCE(excluded.name, known_devices.name), \
               rssi = COALESCE(excluded.rssi, known_devices.rssi), \
               battery_level = COALESCE(excluded.battery_level, known_devices.battery_level), \
               last_seen = excluded.last_seen, \
               manufacturer = COALESCE(excluded.manufacturer, known_devices.manufacturer), \
               model_number = COALESCE(excluded.model_number, known_devices.model_number), \
               serial_number = COALESCE(excluded.serial_number, known_devices.serial_number), \
               device_group = COALESCE(excluded.device_group, known_devices.device_group)",
        )
        .bind(&device.id)
        .bind(&device.name)
        .bind(&device_type)
        .bind(&transport)
        .bind(device.rssi.map(|v| v as i32))
        .bind(device.battery_level.map(|v| v as i32))
        .bind(&last_seen)
        .bind(&device.manufacturer)
        .bind(&device.model_number)
        .bind(&device.serial_number)
        .bind(&device.device_group)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }

    pub async fn upsert_known_devices_batch(&self, devices: &[DeviceInfo]) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await.map_err(AppError::Database)?;
        for device in devices {
            let device_type = device.device_type.as_str();
            let transport = device.transport.as_str();
            let last_seen = device
                .last_seen
                .clone()
                .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
            sqlx::query(
                "INSERT INTO known_devices (id, name, device_type, transport, rssi, battery_level, \
                 last_seen, manufacturer, model_number, serial_number, device_group) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(id) DO UPDATE SET \
                   name = COALESCE(excluded.name, known_devices.name), \
                   rssi = COALESCE(excluded.rssi, known_devices.rssi), \
                   battery_level = COALESCE(excluded.battery_level, known_devices.battery_level), \
                   last_seen = excluded.last_seen, \
                   manufacturer = COALESCE(excluded.manufacturer, known_devices.manufacturer), \
                   model_number = COALESCE(excluded.model_number, known_devices.model_number), \
                   serial_number = COALESCE(excluded.serial_number, known_devices.serial_number), \
                   device_group = COALESCE(excluded.device_group, known_devices.device_group)",
            )
            .bind(&device.id)
            .bind(&device.name)
            .bind(&device_type)
            .bind(&transport)
            .bind(device.rssi.map(|v| v as i32))
            .bind(device.battery_level.map(|v| v as i32))
            .bind(&last_seen)
            .bind(&device.manufacturer)
            .bind(&device.model_number)
            .bind(&device.serial_number)
            .bind(&device.device_group)
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;
        }
        tx.commit().await.map_err(AppError::Database)?;
        Ok(())
    }

    pub async fn clear_device_group(&self, device_id: &str) -> Result<(), AppError> {
        sqlx::query("UPDATE known_devices SET device_group = NULL WHERE id = ?")
            .bind(device_id)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
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

    pub fn data_dir(&self) -> &str {
        &self.data_dir
    }

    /// Write an autosave checkpoint for a running session.
    /// Format: 4-byte JSON-length (LE) + JSON summary + bincode sensor_log.
    /// Uses atomic write (write tmp → rename) to avoid corruption.
    pub async fn write_autosave(
        &self,
        session_id: &str,
        summary: &SessionSummary,
        sensor_log: &[SensorReading],
    ) -> Result<(), AppError> {
        let sessions_dir = Path::new(&self.data_dir).join("sessions");
        tokio::fs::create_dir_all(&sessions_dir)
            .await
            .map_err(|e| AppError::Serialization(format!("Failed to create sessions dir: {}", e)))?;

        let json_bytes = serde_json::to_vec(summary)
            .map_err(|e| AppError::Serialization(e.to_string()))?;
        let sensor_bytes = bincode::serialize(sensor_log)
            .map_err(|e| AppError::Serialization(e.to_string()))?;

        let json_len = (json_bytes.len() as u32).to_le_bytes();
        let mut data = Vec::with_capacity(4 + json_bytes.len() + sensor_bytes.len());
        data.extend_from_slice(&json_len);
        data.extend_from_slice(&json_bytes);
        data.extend_from_slice(&sensor_bytes);

        let tmp_path = sessions_dir.join(format!(".autosave_{}.tmp", session_id));
        let final_path = sessions_dir.join(format!(".autosave_{}.bin", session_id));

        tokio::fs::write(&tmp_path, &data)
            .await
            .map_err(|e| AppError::Serialization(format!("Failed to write autosave tmp: {}", e)))?;
        tokio::fs::rename(&tmp_path, &final_path)
            .await
            .map_err(|e| AppError::Serialization(format!("Failed to rename autosave: {}", e)))?;

        Ok(())
    }

    /// Remove the autosave file for a session (e.g. after successful save).
    pub fn remove_autosave(&self, session_id: &str) {
        let path = Path::new(&self.data_dir)
            .join("sessions")
            .join(format!(".autosave_{}.bin", session_id));
        let _ = std::fs::remove_file(path);
    }

    /// Scan for autosave files, recover each into the DB, and delete the autosave.
    /// Returns the count of recovered sessions.
    pub async fn recover_autosaved_sessions(&self) -> Result<usize, AppError> {
        let sessions_dir = Path::new(&self.data_dir).join("sessions");
        if !sessions_dir.exists() {
            return Ok(0);
        }

        let mut count = 0;
        let entries = std::fs::read_dir(&sessions_dir)
            .map_err(|e| AppError::Serialization(format!("Failed to read sessions dir: {}", e)))?;

        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.starts_with(".autosave_") || !name_str.ends_with(".bin") {
                continue;
            }

            let data = match std::fs::read(entry.path()) {
                Ok(d) => d,
                Err(e) => {
                    warn!("Failed to read autosave {}: {}", name_str, e);
                    continue;
                }
            };

            if data.len() < 4 {
                warn!("Autosave {} too short, skipping", name_str);
                let _ = std::fs::remove_file(entry.path());
                continue;
            }

            let json_len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
            if data.len() < 4 + json_len {
                warn!("Autosave {} truncated, skipping", name_str);
                let _ = std::fs::remove_file(entry.path());
                continue;
            }

            let summary: SessionSummary = match serde_json::from_slice(&data[4..4 + json_len]) {
                Ok(s) => s,
                Err(e) => {
                    warn!("Autosave {} bad JSON: {}", name_str, e);
                    let _ = std::fs::remove_file(entry.path());
                    continue;
                }
            };

            let sensor_bytes = &data[4 + json_len..];

            if validate_session_id(&summary.id).is_err() {
                warn!("Autosave {} has invalid session ID, skipping", name_str);
                let _ = std::fs::remove_file(entry.path());
                continue;
            }

            match self.save_session(&summary, sensor_bytes).await {
                Ok(()) => {
                    info!("Recovered autosaved session {}", summary.id);
                    let _ = std::fs::remove_file(entry.path());
                    count += 1;
                }
                Err(e) => {
                    warn!("Failed to recover autosave {}: {}", summary.id, e);
                }
            }
        }

        Ok(count)
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

    pub async fn delete_session(&self, session_id: &str) -> Result<(), AppError> {
        // Delete file first, then DB rows. A row without a file is visible in
        // history (gracefully handleable). A file without a row is an orphan
        // that wastes disk space silently forever.
        let path = Path::new(&self.data_dir)
            .join("sessions")
            .join(format!("{}.bin", session_id));
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| AppError::Session(format!("Failed to delete session file: {}", e)))?;
        }
        sqlx::query("DELETE FROM session_power_curves WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    pub async fn save_zone_config(
        &self,
        session_id: &str,
        zone_config: &str,
    ) -> Result<(), AppError> {
        sqlx::query("UPDATE sessions SET zone_config = ? WHERE id = ?")
            .bind(zone_config)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
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

    pub async fn save_power_curve(
        &self,
        session_id: &str,
        curve: &[PowerCurvePoint],
    ) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await.map_err(AppError::Database)?;
        for point in curve {
            sqlx::query(
                "INSERT OR REPLACE INTO session_power_curves (session_id, duration_secs, watts) \
                 VALUES (?, ?, ?)",
            )
            .bind(session_id)
            .bind(point.duration_secs as i32)
            .bind(point.watts as i32)
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;
        }
        tx.commit().await.map_err(AppError::Database)?;
        Ok(())
    }

    pub async fn get_best_power_curve(
        &self,
        after_date: Option<&str>,
    ) -> Result<Vec<PowerCurvePoint>, AppError> {
        let rows: Vec<(i32, i32)> = if let Some(date) = after_date {
            sqlx::query_as(
                "SELECT pc.duration_secs, MAX(pc.watts) as watts \
                 FROM session_power_curves pc \
                 JOIN sessions s ON s.id = pc.session_id \
                 WHERE s.start_time >= ? \
                 GROUP BY pc.duration_secs \
                 ORDER BY pc.duration_secs",
            )
            .bind(date)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?
        } else {
            sqlx::query_as(
                "SELECT duration_secs, MAX(watts) as watts \
                 FROM session_power_curves \
                 GROUP BY duration_secs \
                 ORDER BY duration_secs",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?
        };
        Ok(rows
            .into_iter()
            .map(|(d, w)| PowerCurvePoint {
                duration_secs: d as u32,
                watts: w as u16,
            })
            .collect())
    }

    pub async fn has_power_curve(&self, session_id: &str) -> Result<bool, AppError> {
        let row: Option<(i32,)> =
            sqlx::query_as("SELECT 1 FROM session_power_curves WHERE session_id = ? LIMIT 1")
                .bind(session_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(AppError::Database)?;
        Ok(row.is_some())
    }

    pub async fn list_known_devices(&self) -> Result<Vec<DeviceInfo>, AppError> {
        let rows = sqlx::query_as::<_, KnownDeviceRow>(
            "SELECT id, name, device_type, transport, rssi, battery_level, last_seen, \
             manufacturer, model_number, serial_number, device_group \
             FROM known_devices ORDER BY last_seen DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(rows.into_iter().map(|r| r.into()).collect())
    }
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

#[derive(sqlx::FromRow)]
struct ConfigRow {
    ftp: i32,
    weight_kg: f64,
    hr_zone_1: i32,
    hr_zone_2: i32,
    hr_zone_3: i32,
    hr_zone_4: i32,
    hr_zone_5: i32,
    units: String,
    power_zone_1: i32,
    power_zone_2: i32,
    power_zone_3: i32,
    power_zone_4: i32,
    power_zone_5: i32,
    power_zone_6: i32,
    date_of_birth: Option<String>,
    sex: Option<String>,
    resting_hr: Option<i32>,
    max_hr: Option<i32>,
}

#[derive(sqlx::FromRow)]
struct KnownDeviceRow {
    id: String,
    name: Option<String>,
    device_type: String,
    transport: String,
    rssi: Option<i32>,
    battery_level: Option<i32>,
    last_seen: String,
    manufacturer: Option<String>,
    model_number: Option<String>,
    serial_number: Option<String>,
    device_group: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::types::{ConnectionStatus, DeviceType, Transport};

    async fn test_storage() -> (Storage, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let storage = Storage::new(&tmp.path().to_string_lossy())
            .await
            .unwrap();
        (storage, tmp)
    }

    fn make_summary(id: &str) -> SessionSummary {
        SessionSummary {
            id: id.to_string(),
            start_time: chrono::Utc::now(),
            duration_secs: 3600,
            ftp: Some(200),
            avg_power: Some(180),
            max_power: Some(300),
            normalized_power: Some(190),
            tss: Some(75.0),
            intensity_factor: Some(0.95),
            avg_hr: Some(145),
            max_hr: Some(170),
            avg_cadence: Some(90.0),
            avg_speed: Some(30.0),
            work_kj: Some(648.0),
            variability_index: Some(1.05),
            distance_km: None,
            title: None,
            activity_type: None,
            rpe: None,
            notes: None,
        }
    }

    fn make_device(id: &str, name: Option<&str>, last_seen: &str) -> DeviceInfo {
        DeviceInfo {
            id: id.to_string(),
            name: name.map(|s| s.to_string()),
            device_type: DeviceType::Power,
            status: ConnectionStatus::Disconnected,
            transport: Transport::Ble,
            rssi: Some(-60),
            battery_level: Some(80),
            last_seen: Some(last_seen.to_string()),
            manufacturer: None,
            model_number: None,
            serial_number: None,
            device_group: None,
            in_range: true,
        }
    }

    #[tokio::test]
    async fn init_creates_tables() {
        let (_storage, _tmp) = test_storage().await;
        // Success means migrations ran without error
    }

    #[tokio::test]
    async fn bad_start_time_returns_error() {
        let (storage, _tmp) = test_storage().await;
        // Insert a row with an unparseable start_time directly via SQL
        sqlx::query(
            "INSERT INTO sessions (id, start_time, duration_secs) VALUES (?, ?, ?)",
        )
        .bind("bad-time-1")
        .bind("not-a-date")
        .bind(60)
        .execute(&storage.pool)
        .await
        .unwrap();

        let result = storage.get_session("bad-time-1").await;
        assert!(result.is_err(), "bad start_time should propagate as error");

        let result = storage.list_sessions().await;
        assert!(result.is_err(), "bad start_time in list should propagate as error");
    }

    #[tokio::test]
    async fn list_sessions_empty() {
        let (storage, _tmp) = test_storage().await;
        let sessions = storage.list_sessions().await.unwrap();
        assert!(sessions.is_empty());
    }

    #[tokio::test]
    async fn save_and_list_session() {
        let (storage, _tmp) = test_storage().await;
        let summary = make_summary("sess-1");
        storage.save_session(&summary, b"raw-data").await.unwrap();

        let sessions = storage.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "sess-1");
        assert_eq!(sessions[0].avg_power, Some(180));
        assert_eq!(sessions[0].max_power, Some(300));
        assert_eq!(sessions[0].duration_secs, 3600);
    }

    #[tokio::test]
    async fn save_session_all_none_fields() {
        let (storage, _tmp) = test_storage().await;
        let summary = SessionSummary {
            id: "sess-none".to_string(),
            start_time: chrono::Utc::now(),
            duration_secs: 60,
            ftp: None,
            avg_power: None,
            max_power: None,
            normalized_power: None,
            tss: None,
            intensity_factor: None,
            avg_hr: None,
            max_hr: None,
            avg_cadence: None,
            avg_speed: None,
            work_kj: None,
            variability_index: None,
            distance_km: None,
            title: None,
            activity_type: None,
            rpe: None,
            notes: None,
        };
        storage.save_session(&summary, b"").await.unwrap();

        let sessions = storage.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].avg_power, None);
        assert_eq!(sessions[0].avg_hr, None);
        assert_eq!(sessions[0].tss, None);
    }

    #[tokio::test]
    async fn save_session_duplicate_id_is_ignored() {
        let (storage, _tmp) = test_storage().await;
        let summary = make_summary("dup-1");
        storage.save_session(&summary, b"first").await.unwrap();

        // Second save with same ID should succeed (INSERT OR IGNORE)
        storage.save_session(&summary, b"second").await.unwrap();

        // Only one row should exist
        let sessions = storage.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 1);
    }

    #[tokio::test]
    async fn get_default_config() {
        let (storage, _tmp) = test_storage().await;
        let config = storage.get_user_config().await.unwrap();
        assert_eq!(config.ftp, 200);
        assert_eq!(config.weight_kg, 75.0);
        assert_eq!(config.hr_zones, [120, 140, 160, 175, 190]);
        assert_eq!(config.units, "metric");
        assert_eq!(config.power_zones, [55, 75, 90, 105, 120, 150]);
    }

    #[tokio::test]
    async fn save_and_get_config() {
        let (storage, _tmp) = test_storage().await;
        let config = SessionConfig {
            ftp: 250,
            weight_kg: 80.0,
            hr_zones: [130, 150, 165, 180, 195],
            units: "imperial".to_string(),
            power_zones: [60, 80, 95, 110, 125, 155],
            date_of_birth: Some("1990-01-15".to_string()),
            sex: Some("male".to_string()),
            resting_hr: Some(55),
            max_hr: Some(195),
        };
        storage.save_user_config(&config).await.unwrap();

        let loaded = storage.get_user_config().await.unwrap();
        assert_eq!(loaded.ftp, 250);
        assert_eq!(loaded.weight_kg, 80.0);
        assert_eq!(loaded.units, "imperial");
        assert_eq!(loaded.date_of_birth, Some("1990-01-15".to_string()));
        assert_eq!(loaded.resting_hr, Some(55));
    }

    #[tokio::test]
    async fn save_config_upsert_overwrites() {
        let (storage, _tmp) = test_storage().await;
        let mut config = SessionConfig::default();
        config.ftp = 300;
        storage.save_user_config(&config).await.unwrap();

        config.ftp = 350;
        storage.save_user_config(&config).await.unwrap();

        let loaded = storage.get_user_config().await.unwrap();
        assert_eq!(loaded.ftp, 350);
    }

    #[tokio::test]
    async fn upsert_and_list_devices() {
        let (storage, _tmp) = test_storage().await;
        let device = make_device("ble-1234", Some("Kickr"), "2024-01-01T00:00:00Z");
        storage.upsert_known_device(&device).await.unwrap();

        let devices = storage.list_known_devices().await.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "ble-1234");
        assert_eq!(devices[0].name, Some("Kickr".to_string()));
    }

    #[tokio::test]
    async fn upsert_device_coalesce_preserves_name() {
        let (storage, _tmp) = test_storage().await;
        let d1 = make_device("ble-1234", Some("Kickr"), "2024-01-01T00:00:00Z");
        storage.upsert_known_device(&d1).await.unwrap();

        // Second upsert with None name — COALESCE should keep "Kickr"
        let d2 = make_device("ble-1234", None, "2024-01-02T00:00:00Z");
        storage.upsert_known_device(&d2).await.unwrap();

        let devices = storage.list_known_devices().await.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, Some("Kickr".to_string()));
    }

    #[tokio::test]
    async fn autosave_write_and_recover() {
        let (storage, _tmp) = test_storage().await;
        let sid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
        let summary = make_summary(sid);
        let sensor_log: Vec<SensorReading> = vec![SensorReading::Power {
            watts: 200,
            timestamp: None,
            epoch_ms: 1000,
            device_id: "test".to_string(),
            pedal_balance: None,
        }];

        storage
            .write_autosave(sid, &summary, &sensor_log)
            .await
            .unwrap();

        // Verify autosave file exists
        let autosave_path = std::path::Path::new(storage.data_dir())
            .join("sessions")
            .join(format!(".autosave_{}.bin", sid));
        assert!(autosave_path.exists());

        // Recover
        let count = storage.recover_autosaved_sessions().await.unwrap();
        assert_eq!(count, 1);

        // Verify session is in DB
        let sessions = storage.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, sid);

        // Verify autosave file is gone
        assert!(!autosave_path.exists());
    }

    #[tokio::test]
    async fn autosave_recovery_rejects_path_traversal_id() {
        let (storage, _tmp) = test_storage().await;
        // Craft a valid autosave file on disk but with a malicious session ID in the JSON
        let bad_id = "../../etc/passwd";
        let summary = make_summary(bad_id);
        let json_bytes = serde_json::to_vec(&summary).unwrap();
        let json_len = (json_bytes.len() as u32).to_le_bytes();
        let mut data = Vec::new();
        data.extend_from_slice(&json_len);
        data.extend_from_slice(&json_bytes);

        let sessions_dir = std::path::Path::new(storage.data_dir()).join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::write(sessions_dir.join(".autosave_crafted.bin"), &data).unwrap();

        let count = storage.recover_autosaved_sessions().await.unwrap();
        assert_eq!(count, 0, "should reject autosave with path-traversal ID");

        // Verify no session saved to DB
        let sessions = storage.list_sessions().await.unwrap();
        assert!(sessions.is_empty());
    }

    #[tokio::test]
    async fn remove_autosave_cleans_up() {
        let (storage, _tmp) = test_storage().await;
        let summary = make_summary("cleanup-1");
        let sensor_log: Vec<SensorReading> = vec![];

        storage
            .write_autosave("cleanup-1", &summary, &sensor_log)
            .await
            .unwrap();

        let autosave_path = std::path::Path::new(storage.data_dir())
            .join("sessions")
            .join(".autosave_cleanup-1.bin");
        assert!(autosave_path.exists());

        storage.remove_autosave("cleanup-1");
        assert!(!autosave_path.exists());
    }

    #[tokio::test]
    async fn upsert_device_metadata_round_trip() {
        let (storage, _tmp) = test_storage().await;
        let mut device = make_device("ble-meta", Some("Kickr"), "2024-01-01T00:00:00Z");
        device.manufacturer = Some("Wahoo Fitness".to_string());
        device.model_number = Some("KICKR5".to_string());
        device.serial_number = Some("12345".to_string());
        device.device_group = Some("group-abc".to_string());
        storage.upsert_known_device(&device).await.unwrap();

        let devices = storage.list_known_devices().await.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].manufacturer, Some("Wahoo Fitness".to_string()));
        assert_eq!(devices[0].model_number, Some("KICKR5".to_string()));
        assert_eq!(devices[0].serial_number, Some("12345".to_string()));
        assert_eq!(devices[0].device_group, Some("group-abc".to_string()));
    }

    #[tokio::test]
    async fn upsert_device_metadata_coalesce_preserves() {
        let (storage, _tmp) = test_storage().await;
        let mut d1 = make_device("ble-meta2", Some("Kickr"), "2024-01-01T00:00:00Z");
        d1.manufacturer = Some("Wahoo Fitness".to_string());
        d1.serial_number = Some("99999".to_string());
        storage.upsert_known_device(&d1).await.unwrap();

        // Second upsert with None metadata — COALESCE should keep originals
        let d2 = make_device("ble-meta2", None, "2024-01-02T00:00:00Z");
        storage.upsert_known_device(&d2).await.unwrap();

        let devices = storage.list_known_devices().await.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].manufacturer, Some("Wahoo Fitness".to_string()));
        assert_eq!(devices[0].serial_number, Some("99999".to_string()));
    }

    #[tokio::test]
    async fn clear_device_group_removes_group() {
        let (storage, _tmp) = test_storage().await;
        let mut device = make_device("ble-grp", Some("Kickr"), "2024-01-01T00:00:00Z");
        device.device_group = Some("group-xyz".to_string());
        storage.upsert_known_device(&device).await.unwrap();

        storage.clear_device_group("ble-grp").await.unwrap();

        let devices = storage.list_known_devices().await.unwrap();
        assert_eq!(devices[0].device_group, None);
    }

    #[tokio::test]
    async fn update_session_metadata_round_trip() {
        let (storage, _tmp) = test_storage().await;
        let summary = make_summary("meta-1");
        storage.save_session(&summary, b"raw").await.unwrap();

        storage
            .update_session_metadata("meta-1", Some("Morning Ride".into()), Some("endurance".into()), Some(6), Some("Felt good".into()))
            .await
            .unwrap();

        let loaded = storage.get_session("meta-1").await.unwrap();
        assert_eq!(loaded.title, Some("Morning Ride".to_string()));
        assert_eq!(loaded.activity_type, Some("endurance".to_string()));
        assert_eq!(loaded.rpe, Some(6));
        assert_eq!(loaded.notes, Some("Felt good".to_string()));
    }

    #[tokio::test]
    async fn update_session_metadata_coalesce() {
        let (storage, _tmp) = test_storage().await;
        let summary = make_summary("meta-2");
        storage.save_session(&summary, b"raw").await.unwrap();

        // First update: set title only
        storage
            .update_session_metadata("meta-2", Some("Evening Ride".into()), None, None, None)
            .await
            .unwrap();

        // Second update: set rpe only — title should be preserved
        storage
            .update_session_metadata("meta-2", None, None, Some(8), None)
            .await
            .unwrap();

        let loaded = storage.get_session("meta-2").await.unwrap();
        assert_eq!(loaded.title, Some("Evening Ride".to_string()));
        assert_eq!(loaded.rpe, Some(8));
    }

    #[tokio::test]
    async fn delete_session_removes_row() {
        let (storage, _tmp) = test_storage().await;
        let summary = make_summary("del-1");
        storage.save_session(&summary, b"raw").await.unwrap();

        assert_eq!(storage.list_sessions().await.unwrap().len(), 1);

        storage.delete_session("del-1").await.unwrap();
        assert!(storage.list_sessions().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn update_metadata_nonexistent_session_returns_error() {
        let (storage, _tmp) = test_storage().await;
        let result = storage
            .update_session_metadata("no-such-id", Some("Title".into()), None, None, None)
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Session not found"), "expected 'Session not found', got: {}", err);
    }

    #[tokio::test]
    async fn load_sensor_data_round_trip() {
        let (storage, _tmp) = test_storage().await;

        let readings = vec![
            SensorReading::Power {
                watts: 250,
                timestamp: None,
                epoch_ms: 1000,
                device_id: "pm-1".to_string(),
                pedal_balance: Some(52),
            },
            SensorReading::HeartRate {
                bpm: 155,
                timestamp: None,
                epoch_ms: 1250,
                device_id: "hr-1".to_string(),
            },
            SensorReading::Cadence {
                rpm: 90.5,
                timestamp: None,
                epoch_ms: 1500,
                device_id: "cad-1".to_string(),
            },
            SensorReading::Speed {
                kmh: 32.1,
                timestamp: None,
                epoch_ms: 1750,
                device_id: "spd-1".to_string(),
            },
            SensorReading::Power {
                watts: 0,
                timestamp: None,
                epoch_ms: 2000,
                device_id: "pm-1".to_string(),
                pedal_balance: None,
            },
        ];

        let raw = bincode::serialize(&readings).unwrap();
        let summary = make_summary("rt-1");
        storage.save_session(&summary, &raw).await.unwrap();

        let loaded = storage.load_sensor_data("rt-1").unwrap();
        assert_eq!(loaded.len(), 5);

        // Verify Power with pedal_balance
        match &loaded[0] {
            SensorReading::Power { watts, epoch_ms, device_id, pedal_balance, .. } => {
                assert_eq!(*watts, 250);
                assert_eq!(*epoch_ms, 1000);
                assert_eq!(device_id, "pm-1");
                assert_eq!(*pedal_balance, Some(52));
            }
            other => panic!("expected Power, got {:?}", other),
        }

        // Verify HeartRate
        match &loaded[1] {
            SensorReading::HeartRate { bpm, epoch_ms, device_id, .. } => {
                assert_eq!(*bpm, 155);
                assert_eq!(*epoch_ms, 1250);
                assert_eq!(device_id, "hr-1");
            }
            other => panic!("expected HeartRate, got {:?}", other),
        }

        // Verify Cadence
        match &loaded[2] {
            SensorReading::Cadence { rpm, epoch_ms, device_id, .. } => {
                assert!((rpm - 90.5).abs() < 0.01);
                assert_eq!(*epoch_ms, 1500);
                assert_eq!(device_id, "cad-1");
            }
            other => panic!("expected Cadence, got {:?}", other),
        }

        // Verify Speed
        match &loaded[3] {
            SensorReading::Speed { kmh, epoch_ms, device_id, .. } => {
                assert!((kmh - 32.1).abs() < 0.01);
                assert_eq!(*epoch_ms, 1750);
                assert_eq!(device_id, "spd-1");
            }
            other => panic!("expected Speed, got {:?}", other),
        }

        // Verify Power with pedal_balance=None
        match &loaded[4] {
            SensorReading::Power { watts, pedal_balance, .. } => {
                assert_eq!(*watts, 0);
                assert_eq!(*pedal_balance, None);
            }
            other => panic!("expected Power, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn load_sensor_data_empty_round_trip() {
        let (storage, _tmp) = test_storage().await;
        let readings: Vec<SensorReading> = vec![];
        let raw = bincode::serialize(&readings).unwrap();
        let summary = make_summary("rt-empty");
        storage.save_session(&summary, &raw).await.unwrap();

        let loaded = storage.load_sensor_data("rt-empty").unwrap();
        assert!(loaded.is_empty());
    }

    // --- Power curve storage tests ---

    #[tokio::test]
    async fn save_and_get_power_curve() {
        let (storage, _tmp) = test_storage().await;
        let summary = make_summary("pc-1");
        storage.save_session(&summary, b"raw").await.unwrap();

        let curve = vec![
            PowerCurvePoint { duration_secs: 1, watts: 400 },
            PowerCurvePoint { duration_secs: 5, watts: 350 },
            PowerCurvePoint { duration_secs: 60, watts: 280 },
        ];
        storage.save_power_curve("pc-1", &curve).await.unwrap();

        let best = storage.get_best_power_curve(None).await.unwrap();
        assert_eq!(best.len(), 3);
        assert_eq!(best[0].duration_secs, 1);
        assert_eq!(best[0].watts, 400);
        assert_eq!(best[2].duration_secs, 60);
        assert_eq!(best[2].watts, 280);
    }

    #[tokio::test]
    async fn best_power_curve_takes_max_across_sessions() {
        let (storage, _tmp) = test_storage().await;

        let s1 = make_summary("pc-max-1");
        storage.save_session(&s1, b"raw").await.unwrap();
        storage.save_power_curve("pc-max-1", &[
            PowerCurvePoint { duration_secs: 1, watts: 400 },
            PowerCurvePoint { duration_secs: 60, watts: 250 },
        ]).await.unwrap();

        let s2 = make_summary("pc-max-2");
        storage.save_session(&s2, b"raw").await.unwrap();
        storage.save_power_curve("pc-max-2", &[
            PowerCurvePoint { duration_secs: 1, watts: 350 },
            PowerCurvePoint { duration_secs: 60, watts: 300 },
        ]).await.unwrap();

        let best = storage.get_best_power_curve(None).await.unwrap();
        // 1s: max(400, 350) = 400
        let p1 = best.iter().find(|p| p.duration_secs == 1).unwrap();
        assert_eq!(p1.watts, 400);
        // 60s: max(250, 300) = 300
        let p60 = best.iter().find(|p| p.duration_secs == 60).unwrap();
        assert_eq!(p60.watts, 300);
    }

    #[tokio::test]
    async fn has_power_curve_detects_presence() {
        let (storage, _tmp) = test_storage().await;
        let summary = make_summary("pc-has-1");
        storage.save_session(&summary, b"raw").await.unwrap();

        assert!(!storage.has_power_curve("pc-has-1").await.unwrap());

        storage.save_power_curve("pc-has-1", &[
            PowerCurvePoint { duration_secs: 1, watts: 300 },
        ]).await.unwrap();

        assert!(storage.has_power_curve("pc-has-1").await.unwrap());
    }

    #[tokio::test]
    async fn delete_session_removes_power_curves() {
        let (storage, _tmp) = test_storage().await;
        let summary = make_summary("pc-del-1");
        storage.save_session(&summary, b"raw").await.unwrap();
        storage.save_power_curve("pc-del-1", &[
            PowerCurvePoint { duration_secs: 1, watts: 400 },
            PowerCurvePoint { duration_secs: 5, watts: 350 },
        ]).await.unwrap();

        assert!(storage.has_power_curve("pc-del-1").await.unwrap());

        storage.delete_session("pc-del-1").await.unwrap();

        assert!(!storage.has_power_curve("pc-del-1").await.unwrap());
        let best = storage.get_best_power_curve(None).await.unwrap();
        assert!(best.is_empty());
    }

    #[tokio::test]
    async fn upsert_batch_coalesce_preserves() {
        let (storage, _tmp) = test_storage().await;
        // Insert device with name+manufacturer via single upsert
        let mut d1 = make_device("ble-batch", Some("Kickr"), "2024-01-01T00:00:00Z");
        d1.manufacturer = Some("Wahoo Fitness".to_string());
        storage.upsert_known_device(&d1).await.unwrap();

        // Batch-upsert with None name — COALESCE should preserve originals
        let d2 = make_device("ble-batch", None, "2024-01-02T00:00:00Z");
        let d3 = make_device("ble-new", Some("HRM"), "2024-01-02T00:00:00Z");
        storage
            .upsert_known_devices_batch(&[d2, d3])
            .await
            .unwrap();

        let devices = storage.list_known_devices().await.unwrap();
        assert_eq!(devices.len(), 2);

        let batch_dev = devices.iter().find(|d| d.id == "ble-batch").unwrap();
        assert_eq!(batch_dev.name, Some("Kickr".to_string()));
        assert_eq!(batch_dev.manufacturer, Some("Wahoo Fitness".to_string()));
        // last_seen should be updated
        assert_eq!(batch_dev.last_seen, Some("2024-01-02T00:00:00Z".to_string()));

        let new_dev = devices.iter().find(|d| d.id == "ble-new").unwrap();
        assert_eq!(new_dev.name, Some("HRM".to_string()));
    }

    #[tokio::test]
    async fn list_devices_ordered_by_last_seen() {
        let (storage, _tmp) = test_storage().await;
        let d1 = make_device("d1", Some("Oldest"), "2024-01-01T00:00:00Z");
        let d2 = make_device("d2", Some("Middle"), "2024-06-01T00:00:00Z");
        let d3 = make_device("d3", Some("Newest"), "2024-12-01T00:00:00Z");
        storage.upsert_known_device(&d1).await.unwrap();
        storage.upsert_known_device(&d2).await.unwrap();
        storage.upsert_known_device(&d3).await.unwrap();

        let devices = storage.list_known_devices().await.unwrap();
        assert_eq!(devices.len(), 3);
        // ORDER BY last_seen DESC
        assert_eq!(devices[0].id, "d3");
        assert_eq!(devices[1].id, "d2");
        assert_eq!(devices[2].id, "d1");
    }
}

impl From<KnownDeviceRow> for DeviceInfo {
    fn from(row: KnownDeviceRow) -> Self {
        let device_type = match row.device_type.as_str() {
            "HeartRate" => DeviceType::HeartRate,
            "Power" => DeviceType::Power,
            "CadenceSpeed" => DeviceType::CadenceSpeed,
            "FitnessTrainer" => DeviceType::FitnessTrainer,
            other => {
                warn!("Unknown device_type '{}' for device '{}', defaulting to HeartRate", other, row.id);
                DeviceType::HeartRate
            }
        };
        let transport = match row.transport.as_str() {
            "AntPlus" => Transport::AntPlus,
            _ => Transport::Ble,
        };
        Self {
            id: row.id,
            name: row.name,
            device_type,
            status: ConnectionStatus::Disconnected,
            transport,
            rssi: row.rssi.map(|v| v as i16),
            battery_level: row.battery_level.map(|v| v as u8),
            last_seen: Some(row.last_seen),
            manufacturer: row.manufacturer,
            model_number: row.model_number,
            serial_number: row.serial_number,
            device_group: row.device_group,
            in_range: true,
        }
    }
}
