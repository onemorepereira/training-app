mod autosave;
mod config;
mod devices;
mod power_curves;
mod sessions;

use log::info;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;

use crate::error::AppError;

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
    pub(crate) pool: SqlitePool,
    pub(crate) data_dir: String,
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
        info!("Database opened: {}", db_path.display());
        let migration_sql = include_str!("../../../migrations/001_init.sql");
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
        info!("Database migrations complete");
        Ok(Self {
            pool,
            data_dir: data_dir.to_string(),
        })
    }

    pub fn data_dir(&self) -> &str {
        &self.data_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::types::{ConnectionStatus, DeviceType, SensorReading, Transport};
    use crate::session::analysis::PowerCurvePoint;
    use crate::session::types::{SessionConfig, SessionSummary};

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

    fn make_device(id: &str, name: Option<&str>, last_seen: &str) -> crate::device::types::DeviceInfo {
        crate::device::types::DeviceInfo {
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
