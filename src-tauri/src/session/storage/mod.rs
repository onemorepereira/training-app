mod autosave;
mod config;
mod devices;
mod fsio;
mod meta;
mod power_curves;
mod schema;
mod sessions;

pub use meta::verify_sqlite_file;
pub use sessions::SaveOutcome;

use log::info;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;

use crate::error::AppError;

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
        let version = schema::migrate(&pool, &db_path).await?;
        info!("Database schema at version {}", version);
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
            zones: None,
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
    async fn reopening_storage_is_idempotent_with_app_meta_and_zones() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().to_string_lossy().to_string();
        {
            let s = Storage::new(&dir).await.unwrap();
            let mut summary = make_summary("idem-1");
            summary.zones = Some(crate::session::types::ZoneSnapshot {
                power_zones: [50, 70, 85, 100, 115, 140],
                hr_zones: [110, 130, 150, 165, 180],
            });
            s.save_session(&summary, b"raw").await.unwrap();
            s.set_meta("metrics_version", "2").await.unwrap();
            s.pool.close().await;
        }
        // Second open re-runs every inline migration on an existing DB
        let s = Storage::new(&dir).await.unwrap();
        assert!(s.get_session("idem-1").await.unwrap().zones.is_some());
        assert_eq!(s.get_meta("metrics_version").await.unwrap().as_deref(), Some("2"));
    }

    #[tokio::test]
    async fn save_session_round_trips_zones() {
        let (storage, _tmp) = test_storage().await;
        let zones = crate::session::types::ZoneSnapshot {
            power_zones: [50, 70, 85, 100, 115, 140],
            hr_zones: [110, 130, 150, 165, 180],
        };
        let mut summary = make_summary("zones-1");
        summary.zones = Some(zones);
        storage.save_session(&summary, b"raw").await.unwrap();

        assert_eq!(storage.get_session("zones-1").await.unwrap().zones, Some(zones));
        let listed = storage.list_sessions().await.unwrap();
        assert_eq!(listed[0].zones, Some(zones));
    }

    #[test]
    fn autosave_summary_json_without_zones_still_deserializes() {
        // JSON written by a build before zone snapshots existed
        let mut v = serde_json::to_value(make_summary("old-json")).unwrap();
        v.as_object_mut().unwrap().remove("zones");
        let s: SessionSummary = serde_json::from_value(v).unwrap();
        assert_eq!(s.zones, None);
    }

    #[tokio::test]
    async fn legacy_row_has_no_zones() {
        let (storage, _tmp) = test_storage().await;
        // Row written by an older build: no zones column value
        sqlx::query("INSERT INTO sessions (id, start_time, duration_secs) VALUES (?, ?, ?)")
            .bind("legacy-1")
            .bind("2024-06-15T10:00:00+00:00")
            .bind(60)
            .execute(&storage.pool)
            .await
            .unwrap();
        assert_eq!(storage.get_session("legacy-1").await.unwrap().zones, None);
    }

    #[tokio::test]
    async fn get_meta_missing_key_returns_none() {
        let (storage, _tmp) = test_storage().await;
        assert_eq!(storage.get_meta("metrics_version").await.unwrap(), None);
    }

    #[tokio::test]
    async fn set_meta_overwrites_value() {
        let (storage, _tmp) = test_storage().await;
        storage.set_meta("metrics_repair_attempts", "1").await.unwrap();
        storage.set_meta("metrics_repair_attempts", "2").await.unwrap();
        assert_eq!(
            storage.get_meta("metrics_repair_attempts").await.unwrap().as_deref(),
            Some("2")
        );
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
            zones: None,
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

        storage.remove_autosave("cleanup-1").await.unwrap();
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
    async fn update_session_metadata_none_clears_field() {
        let (storage, _tmp) = test_storage().await;
        let summary = make_summary("meta-2");
        storage.save_session(&summary, b"raw").await.unwrap();
        storage
            .update_session_metadata("meta-2", Some("Evening Ride".into()), Some("tempo".into()), Some(7), Some("Windy".into()))
            .await
            .unwrap();

        // Full-replace: None clears activity type, RPE and notes
        storage
            .update_session_metadata("meta-2", Some("Evening Ride".into()), None, None, None)
            .await
            .unwrap();

        let loaded = storage.get_session("meta-2").await.unwrap();
        assert_eq!(loaded.title, Some("Evening Ride".to_string()));
        assert_eq!(loaded.activity_type, None);
        assert_eq!(loaded.rpe, None);
        assert_eq!(loaded.notes, None);
    }

    #[tokio::test]
    async fn update_session_metadata_empty_string_stored_as_null() {
        let (storage, _tmp) = test_storage().await;
        let summary = make_summary("meta-3");
        storage.save_session(&summary, b"raw").await.unwrap();
        storage
            .update_session_metadata("meta-3", Some("Ride".into()), Some("tempo".into()), None, Some("Notes".into()))
            .await
            .unwrap();

        storage
            .update_session_metadata("meta-3", Some("   ".into()), Some("".into()), None, Some("".into()))
            .await
            .unwrap();

        let loaded = storage.get_session("meta-3").await.unwrap();
        assert_eq!(loaded.title, None);
        assert_eq!(loaded.activity_type, None);
        assert_eq!(loaded.notes, None);
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
    async fn save_power_curve_replaces_stale_durations() {
        let (storage, _tmp) = test_storage().await;
        storage.save_session(&make_summary("pc-re-1"), b"raw").await.unwrap();
        // Old (fabricated) curve had a 20-min point; the recomputed one does not.
        storage.save_power_curve("pc-re-1", &[
            PowerCurvePoint { duration_secs: 1, watts: 400 },
            PowerCurvePoint { duration_secs: 1200, watts: 300 },
        ]).await.unwrap();
        storage.save_power_curve("pc-re-1", &[
            PowerCurvePoint { duration_secs: 1, watts: 380 },
        ]).await.unwrap();

        let best = storage.get_best_power_curve(None).await.unwrap();
        assert_eq!(best.len(), 1, "stale 1200 s point must be gone: {:?}", best.iter().map(|p| p.duration_secs).collect::<Vec<_>>());
        assert_eq!(best[0].watts, 380);
    }

    #[tokio::test]
    async fn save_power_curve_for_deleted_session_writes_nothing() {
        let (storage, _tmp) = test_storage().await;
        storage.save_session(&make_summary("pc-gone-1"), b"raw").await.unwrap();
        // Background curve save after stop races a delete
        storage.delete_session("pc-gone-1").await.unwrap();
        storage.save_power_curve("pc-gone-1", &[
            PowerCurvePoint { duration_secs: 1, watts: 900 },
        ]).await.unwrap();

        assert!(!storage.has_power_curve("pc-gone-1").await.unwrap());
    }

    #[tokio::test]
    async fn best_power_curve_ignores_orphan_rows() {
        let (storage, _tmp) = test_storage().await;
        storage.save_session(&make_summary("pc-live-1"), b"raw").await.unwrap();
        storage.save_power_curve("pc-live-1", &[
            PowerCurvePoint { duration_secs: 1, watts: 300 },
        ]).await.unwrap();
        // Orphan row left by an older build for a session that no longer exists
        sqlx::query("INSERT INTO session_power_curves (session_id, duration_secs, watts) VALUES ('ghost', 1, 900)")
            .execute(&storage.pool)
            .await
            .unwrap();

        let best = storage.get_best_power_curve(None).await.unwrap();
        assert_eq!(best.len(), 1);
        assert_eq!(best[0].watts, 300, "orphan 900 W must not count");
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
    async fn save_and_get_zone_config() {
        let (storage, _tmp) = test_storage().await;
        let summary = make_summary("zc-1");
        storage.save_session(&summary, b"raw").await.unwrap();

        // No zone config initially
        let config = storage.get_zone_config("zc-1").await.unwrap();
        assert_eq!(config, None);

        // Save zone config
        let zone_json = r#"{"mode":"zone","zone":3}"#;
        storage.save_zone_config("zc-1", zone_json).await.unwrap();

        // Read it back
        let config = storage.get_zone_config("zc-1").await.unwrap();
        assert_eq!(config, Some(zone_json.to_string()));
    }

    #[tokio::test]
    async fn save_zone_config_nonexistent_session_returns_error() {
        let (storage, _tmp) = test_storage().await;
        let result = storage.save_zone_config("no-such-id", r#"{"zone":1}"#).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Session not found"),
            "expected 'Session not found', got: {}",
            err
        );
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

    // --- Ride-data lifecycle: save / autosave recovery / finalize / delete ---

    const RIDE_ID: &str = "3f2b1c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";

    fn power_log(n: u64) -> Vec<SensorReading> {
        (0..n)
            .map(|i| SensorReading::Power {
                watts: 200 + i as u16,
                timestamp: None,
                epoch_ms: 1_000 * (i + 1),
                device_id: "pm".to_string(),
                pedal_balance: None,
            })
            .collect()
    }

    /// Summary whose duration equals the number of readings, so a test can tell
    /// which snapshot a row came from.
    fn summary_for(n: u64) -> SessionSummary {
        let mut s = make_summary(RIDE_ID);
        s.duration_secs = n;
        s.title = None;
        s
    }

    fn raw(n: u64) -> Vec<u8> {
        bincode::serialize(&power_log(n)).unwrap()
    }

    fn sessions_dir(storage: &Storage) -> std::path::PathBuf {
        std::path::Path::new(storage.data_dir()).join("sessions")
    }

    fn files_in(storage: &Storage) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(sessions_dir(storage))
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    async fn restart(tmp: &tempfile::TempDir) -> Storage {
        Storage::new(&tmp.path().to_string_lossy()).await.unwrap()
    }

    #[tokio::test]
    async fn save_session_returns_inserted_then_already_existed() {
        let (storage, _tmp) = test_storage().await;
        let s = summary_for(10);
        assert_eq!(storage.save_session(&s, &raw(10)).await.unwrap(), SaveOutcome::Inserted);
        assert_eq!(
            storage.save_session(&s, &raw(10)).await.unwrap(),
            SaveOutcome::AlreadyExisted
        );
    }

    #[tokio::test]
    async fn save_session_existing_row_keeps_original_raw_file() {
        let (storage, _tmp) = test_storage().await;
        storage.save_session(&summary_for(10), &raw(10)).await.unwrap();
        // An older snapshot of the same ride must not overwrite the final log
        storage.save_session(&summary_for(6), &raw(6)).await.unwrap();
        assert_eq!(storage.load_sensor_data(RIDE_ID).unwrap().len(), 10);
        assert_eq!(storage.get_session(RIDE_ID).await.unwrap().duration_secs, 10);
    }

    #[tokio::test]
    async fn crash_after_final_autosave_before_raw_recovers_full_ride() {
        let (storage, tmp) = test_storage().await;
        storage.write_autosave(RIDE_ID, &summary_for(10), &power_log(10)).await.unwrap();
        drop(storage);

        let storage = restart(&tmp).await;
        assert_eq!(storage.recover_autosaved_sessions().await.unwrap(), 1);
        assert_eq!(storage.get_session(RIDE_ID).await.unwrap().duration_secs, 10);
        assert_eq!(storage.load_sensor_data(RIDE_ID).unwrap().len(), 10);
        assert_eq!(files_in(&storage), vec![format!("{RIDE_ID}.bin")]);
    }

    #[tokio::test]
    async fn crash_after_raw_before_row_inserts_row_and_keeps_raw() {
        let (storage, tmp) = test_storage().await;
        storage.write_autosave(RIDE_ID, &summary_for(10), &power_log(10)).await.unwrap();
        std::fs::write(sessions_dir(&storage).join(format!("{RIDE_ID}.bin")), raw(10)).unwrap();
        drop(storage);

        let storage = restart(&tmp).await;
        assert_eq!(storage.recover_autosaved_sessions().await.unwrap(), 1);
        assert_eq!(storage.get_session(RIDE_ID).await.unwrap().duration_secs, 10);
        assert_eq!(storage.load_sensor_data(RIDE_ID).unwrap().len(), 10);
        assert_eq!(files_in(&storage), vec![format!("{RIDE_ID}.bin")]);
    }

    #[tokio::test]
    async fn crash_after_raw_before_row_with_stale_autosave_keeps_orphan_and_consistent_pair() {
        let (storage, tmp) = test_storage().await;
        // Final autosave failed: only the 30-s-old periodic one (6 readings) exists,
        // while the raw file holds the full 10 readings but no row describes it.
        storage.write_autosave(RIDE_ID, &summary_for(6), &power_log(6)).await.unwrap();
        std::fs::write(sessions_dir(&storage).join(format!("{RIDE_ID}.bin")), raw(10)).unwrap();
        drop(storage);

        let storage = restart(&tmp).await;
        assert_eq!(storage.recover_autosaved_sessions().await.unwrap(), 1);
        // Row and raw describe the same snapshot
        assert_eq!(storage.get_session(RIDE_ID).await.unwrap().duration_secs, 6);
        assert_eq!(storage.load_sensor_data(RIDE_ID).unwrap().len(), 6);
        // The unmatched full log is preserved, not destroyed
        let files = files_in(&storage);
        let orphan: Vec<_> = files.iter().filter(|f| f.contains(".bin.orphan-")).collect();
        assert_eq!(orphan.len(), 1, "files: {files:?}");
        let kept = std::fs::read(sessions_dir(&storage).join(orphan[0])).unwrap();
        assert_eq!(kept, raw(10));
    }

    #[tokio::test]
    async fn crash_after_row_before_autosave_removal_keeps_final_raw() {
        let (storage, tmp) = test_storage().await;
        storage.save_session(&summary_for(10), &raw(10)).await.unwrap();
        storage.write_autosave(RIDE_ID, &summary_for(6), &power_log(6)).await.unwrap();
        drop(storage);

        let storage = restart(&tmp).await;
        assert_eq!(storage.recover_autosaved_sessions().await.unwrap(), 0);
        assert_eq!(storage.get_session(RIDE_ID).await.unwrap().duration_secs, 10);
        assert_eq!(storage.load_sensor_data(RIDE_ID).unwrap().len(), 10);
        assert_eq!(files_in(&storage), vec![format!("{RIDE_ID}.bin")]);
    }

    #[tokio::test]
    async fn legacy_row_without_raw_is_repaired_from_autosave() {
        let (storage, tmp) = test_storage().await;
        // Old code inserted the row first, then crashed before writing the file
        storage.save_session(&summary_for(10), &raw(10)).await.unwrap();
        storage
            .update_session_metadata(RIDE_ID, Some("Hill repeats".into()), None, None, None)
            .await
            .unwrap();
        std::fs::remove_file(sessions_dir(&storage).join(format!("{RIDE_ID}.bin"))).unwrap();
        storage.write_autosave(RIDE_ID, &summary_for(6), &power_log(6)).await.unwrap();
        drop(storage);

        let storage = restart(&tmp).await;
        assert_eq!(storage.recover_autosaved_sessions().await.unwrap(), 0);
        let row = storage.get_session(RIDE_ID).await.unwrap();
        assert_eq!(row.duration_secs, 6, "summary must describe the restored file");
        assert_eq!(row.title.as_deref(), Some("Hill repeats"), "user metadata kept");
        assert_eq!(storage.load_sensor_data(RIDE_ID).unwrap().len(), 6);
    }

    #[tokio::test]
    async fn recovery_removes_stray_tmp_files() {
        let (storage, _tmp) = test_storage().await;
        let dir = sessions_dir(&storage);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{RIDE_ID}.bin.tmp")), b"half").unwrap();
        std::fs::write(dir.join(format!(".autosave_{RIDE_ID}.tmp")), b"half").unwrap();
        storage.recover_autosaved_sessions().await.unwrap();
        assert!(files_in(&storage).is_empty());
    }

    #[tokio::test]
    async fn remove_autosave_missing_file_is_ok() {
        let (storage, _tmp) = test_storage().await;
        storage.remove_autosave(RIDE_ID).await.unwrap();
    }

    #[tokio::test]
    async fn finalize_success_persists_and_removes_autosave() {
        let (storage, _tmp) = test_storage().await;
        storage.finalize_session(&summary_for(10), &power_log(10)).await.unwrap();
        assert_eq!(storage.load_sensor_data(RIDE_ID).unwrap().len(), 10);
        assert_eq!(files_in(&storage), vec![format!("{RIDE_ID}.bin")]);
    }

    #[tokio::test]
    async fn finalize_with_failed_db_save_keeps_full_log_on_disk() {
        let (storage, tmp) = test_storage().await;
        // Older periodic autosave exists from 30 s ago
        storage.write_autosave(RIDE_ID, &summary_for(6), &power_log(6)).await.unwrap();
        storage.pool.close().await; // every DB call now fails

        let err = storage
            .finalize_session(&summary_for(10), &power_log(10))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("could not be saved"), "{err}");
        assert!(err.contains("recovered the next time the app starts"), "{err}");
        drop(storage);

        let storage = restart(&tmp).await;
        assert_eq!(storage.recover_autosaved_sessions().await.unwrap(), 1);
        assert_eq!(storage.get_session(RIDE_ID).await.unwrap().duration_secs, 10);
        assert_eq!(
            storage.load_sensor_data(RIDE_ID).unwrap().len(),
            10,
            "all readings up to stop must survive, not just the last autosave"
        );
    }

    #[tokio::test]
    async fn delete_session_removes_row_curves_and_file() {
        let (storage, _tmp) = test_storage().await;
        storage.save_session(&summary_for(10), &raw(10)).await.unwrap();
        storage
            .save_power_curve(RIDE_ID, &[PowerCurvePoint { duration_secs: 1, watts: 209 }])
            .await
            .unwrap();
        storage.delete_session(RIDE_ID).await.unwrap();
        assert!(storage.list_sessions().await.unwrap().is_empty());
        assert!(!storage.has_power_curve(RIDE_ID).await.unwrap());
        assert!(files_in(&storage).is_empty());
    }

    #[tokio::test]
    async fn delete_session_failed_transaction_leaves_row_curves_and_file() {
        let (storage, _tmp) = test_storage().await;
        storage.save_session(&summary_for(10), &raw(10)).await.unwrap();
        storage
            .save_power_curve(RIDE_ID, &[PowerCurvePoint { duration_secs: 1, watts: 209 }])
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TRIGGER block_delete BEFORE DELETE ON sessions \
             BEGIN SELECT RAISE(ABORT, 'blocked'); END",
        )
        .execute(&storage.pool)
        .await
        .unwrap();

        assert!(storage.delete_session(RIDE_ID).await.is_err());
        assert_eq!(storage.list_sessions().await.unwrap().len(), 1);
        assert!(storage.has_power_curve(RIDE_ID).await.unwrap(), "curve delete rolled back");
        assert_eq!(storage.load_sensor_data(RIDE_ID).unwrap().len(), 10, "raw file kept");
    }

    #[tokio::test]
    async fn delete_session_removes_leftover_autosave_so_restart_cannot_resurrect() {
        let (storage, tmp) = test_storage().await;
        storage.save_session(&summary_for(10), &raw(10)).await.unwrap();
        storage.write_autosave(RIDE_ID, &summary_for(6), &power_log(6)).await.unwrap();
        storage.delete_session(RIDE_ID).await.unwrap();
        assert!(files_in(&storage).is_empty());
        drop(storage);

        let storage = restart(&tmp).await;
        assert_eq!(storage.recover_autosaved_sessions().await.unwrap(), 0);
        assert!(storage.list_sessions().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_session_aborts_when_autosave_cannot_be_removed() {
        let (storage, _tmp) = test_storage().await;
        storage.save_session(&summary_for(10), &raw(10)).await.unwrap();
        // A non-empty directory at the autosave path makes removal fail
        let blocker = sessions_dir(&storage).join(format!(".autosave_{RIDE_ID}.bin"));
        std::fs::create_dir_all(blocker.join("x")).unwrap();

        assert!(storage.delete_session(RIDE_ID).await.is_err());
        assert_eq!(storage.list_sessions().await.unwrap().len(), 1, "nothing deleted");
        assert_eq!(storage.load_sensor_data(RIDE_ID).unwrap().len(), 10);
    }
}
