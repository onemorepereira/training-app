use std::collections::HashMap;
use std::sync::Arc;
use tauri::{Manager, State};
use tokio::sync::broadcast;

use crate::device::manager::DeviceManager;
use crate::device::types::{DeviceDetails, DeviceInfo, DeviceType, SensorReading};
use crate::error::AppError;
use crate::prerequisites;
use crate::session::analysis::{self, SessionAnalysis};
use crate::session::fit_export;
use crate::session::manager::SessionManager;
use crate::session::storage::Storage;
use crate::session::types::{LiveMetrics, SessionConfig, SessionSummary};

/// Validate that a session ID from the frontend is a safe UUID string.
/// Prevents path traversal via crafted IDs like "../../etc/passwd".
fn validate_session_id(id: &str) -> Result<(), AppError> {
    if !id.is_empty() && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        Ok(())
    } else {
        Err(AppError::Session("Invalid session ID".into()))
    }
}

/// Set device as primary for its type if no primary exists yet.
fn auto_set_primary(
    primaries: &mut HashMap<DeviceType, String>,
    device_type: DeviceType,
    device_id: &str,
) {
    primaries
        .entry(device_type)
        .or_insert_with(|| device_id.to_owned());
}

/// Remove all primary entries that reference the given device.
fn remove_primary(primaries: &mut HashMap<DeviceType, String>, device_id: &str) {
    primaries.retain(|_, v| v != device_id);
}

/// Validate that zone boundaries are strictly ascending.
fn validate_zones_ascending<T: PartialOrd>(zones: &[T], label: &str) -> Result<(), AppError> {
    for w in zones.windows(2) {
        if w[0] >= w[1] {
            return Err(AppError::Session(format!(
                "{} must be strictly ascending",
                label
            )));
        }
    }
    Ok(())
}

/// Format primary devices map for the frontend (DeviceType debug keys → string keys).
fn format_primaries(primaries: &HashMap<DeviceType, String>) -> HashMap<String, String> {
    primaries
        .iter()
        .map(|(k, v)| (format!("{:?}", k), v.clone()))
        .collect()
}

pub struct AppState {
    pub device_manager: Arc<tokio::sync::Mutex<DeviceManager>>,
    pub session_manager: Arc<SessionManager>,
    pub storage: Arc<Storage>,
    pub sensor_tx: broadcast::Sender<SensorReading>,
    pub primary_devices: Arc<tokio::sync::Mutex<HashMap<DeviceType, String>>>,
}

#[tauri::command]
pub async fn scan_devices(state: State<'_, AppState>) -> Result<Vec<DeviceInfo>, AppError> {
    let mut dm = state.device_manager.lock().await;
    dm.scan_all().await
}

#[tauri::command]
pub async fn connect_device(
    state: State<'_, AppState>,
    device_id: String,
) -> Result<DeviceInfo, AppError> {
    let tx = state.sensor_tx.clone();
    let mut dm = state.device_manager.lock().await;
    let info = dm.connect(&device_id, tx).await?;

    // Auto-set as primary if no primary exists for this device type
    {
        let mut primaries = state.primary_devices.lock().await;
        auto_set_primary(&mut primaries, info.device_type, &device_id);
    }

    Ok(info)
}

#[tauri::command]
pub async fn disconnect_device(
    state: State<'_, AppState>,
    device_id: String,
) -> Result<(), AppError> {
    {
        let mut primaries = state.primary_devices.lock().await;
        remove_primary(&mut primaries, &device_id);
    }
    let mut dm = state.device_manager.lock().await;
    dm.clear_reconnect_target(&device_id);
    dm.disconnect(&device_id).await
}

#[tauri::command]
pub async fn start_session(state: State<'_, AppState>) -> Result<String, AppError> {
    let config = state.storage.get_user_config().await?;
    let id = state.session_manager.start_session(config).await?;
    Ok(id)
}

#[tauri::command]
pub async fn stop_session(state: State<'_, AppState>) -> Result<Option<SessionSummary>, AppError> {
    let result = state.session_manager.stop_session_with_log().await;

    if let Some((ref summary, ref sensor_log)) = result {
        let raw_data = bincode::serialize(sensor_log)
            .map_err(|e| AppError::Serialization(e.to_string()))?;
        state.storage.save_session(summary, &raw_data).await?;
        state.storage.remove_autosave(&summary.id);
    }

    Ok(result.map(|(summary, _)| summary))
}

#[tauri::command]
pub async fn pause_session(state: State<'_, AppState>) -> Result<(), AppError> {
    state.session_manager.pause_session().await;
    Ok(())
}

#[tauri::command]
pub async fn resume_session(state: State<'_, AppState>) -> Result<(), AppError> {
    state.session_manager.resume_session().await;
    Ok(())
}

#[tauri::command]
pub async fn get_live_metrics(state: State<'_, AppState>) -> Result<Option<LiveMetrics>, AppError> {
    Ok(state.session_manager.get_live_metrics().await)
}

#[tauri::command]
pub async fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionSummary>, AppError> {
    state.storage.list_sessions().await.map_err(AppError::from)
}

#[tauri::command]
pub async fn get_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<SessionSummary, AppError> {
    validate_session_id(&session_id)?;
    state.storage.get_session(&session_id).await
}

#[tauri::command]
pub async fn get_session_analysis(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<SessionAnalysis, AppError> {
    validate_session_id(&session_id)?;
    let session = state.storage.get_session(&session_id).await?;
    let config = state.storage.get_user_config().await?;
    let storage = state.storage.clone();
    let sid = session_id.clone();
    tokio::task::spawn_blocking(move || {
        let readings = storage.load_sensor_data(&sid)?;
        Ok::<_, AppError>(analysis::compute_analysis(&readings, &session, &config))
    })
    .await
    .map_err(|e| AppError::Session(format!("Analysis failed: {}", e)))?
}

#[tauri::command]
pub async fn get_user_config(state: State<'_, AppState>) -> Result<SessionConfig, AppError> {
    state.storage.get_user_config().await.map_err(AppError::from)
}

#[tauri::command]
pub async fn save_user_config(
    state: State<'_, AppState>,
    config: SessionConfig,
) -> Result<(), AppError> {
    validate_zones_ascending(&config.hr_zones, "HR zones")?;
    validate_zones_ascending(&config.power_zones, "Power zones")?;
    state
        .storage
        .save_user_config(&config)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn get_known_devices(state: State<'_, AppState>) -> Result<Vec<DeviceInfo>, AppError> {
    let dm = state.device_manager.lock().await;
    Ok(dm.list_current().await)
}

#[tauri::command]
pub async fn get_device_details(
    state: State<'_, AppState>,
    device_id: String,
) -> Result<DeviceDetails, AppError> {
    let dm = state.device_manager.lock().await;
    dm.get_device_details(&device_id).await
}

#[tauri::command]
pub async fn set_primary_device(
    state: State<'_, AppState>,
    device_type: DeviceType,
    device_id: String,
) -> Result<(), AppError> {
    let mut primaries = state.primary_devices.lock().await;
    primaries.insert(device_type, device_id);
    Ok(())
}

#[tauri::command]
pub async fn get_primary_devices(
    state: State<'_, AppState>,
) -> Result<HashMap<String, String>, AppError> {
    let primaries = state.primary_devices.lock().await;
    Ok(format_primaries(&primaries))
}

#[tauri::command]
pub async fn set_trainer_power(state: State<'_, AppState>, watts: i16) -> Result<(), AppError> {
    let mut dm = state.device_manager.lock().await;
    let trainer_id = dm
        .connected_trainer_id()
        .ok_or_else(|| AppError::Session("No trainer connected".into()))?;
    dm.set_target_power(&trainer_id, watts).await
}

#[tauri::command]
pub async fn set_trainer_resistance(state: State<'_, AppState>, level: u8) -> Result<(), AppError> {
    let mut dm = state.device_manager.lock().await;
    let trainer_id = dm
        .connected_trainer_id()
        .ok_or_else(|| AppError::Session("No trainer connected".into()))?;
    dm.set_resistance(&trainer_id, level).await
}

#[tauri::command]
pub async fn set_trainer_simulation(
    state: State<'_, AppState>,
    grade: f32,
    crr: f32,
    cw: f32,
) -> Result<(), AppError> {
    let mut dm = state.device_manager.lock().await;
    let trainer_id = dm
        .connected_trainer_id()
        .ok_or_else(|| AppError::Session("No trainer connected".into()))?;
    dm.set_simulation(&trainer_id, grade, crr, cw).await
}

#[tauri::command]
pub async fn start_trainer(state: State<'_, AppState>) -> Result<(), AppError> {
    let mut dm = state.device_manager.lock().await;
    let trainer_id = dm
        .connected_trainer_id()
        .ok_or_else(|| AppError::Session("No trainer connected".into()))?;
    dm.start_trainer(&trainer_id).await
}

#[tauri::command]
pub async fn stop_trainer(state: State<'_, AppState>) -> Result<(), AppError> {
    let mut dm = state.device_manager.lock().await;
    let trainer_id = dm
        .connected_trainer_id()
        .ok_or_else(|| AppError::Session("No trainer connected".into()))?;
    dm.stop_trainer(&trainer_id).await
}

#[tauri::command]
pub async fn unlink_devices(
    state: State<'_, AppState>,
    device_id: String,
) -> Result<(), AppError> {
    state.storage.clear_device_group(&device_id).await
}

#[tauri::command]
pub async fn update_session_metadata(
    state: State<'_, AppState>,
    session_id: String,
    title: Option<String>,
    activity_type: Option<String>,
    rpe: Option<u8>,
    notes: Option<String>,
) -> Result<(), AppError> {
    validate_session_id(&session_id)?;
    state
        .storage
        .update_session_metadata(&session_id, title, activity_type, rpe, notes)
        .await
}

#[tauri::command]
pub async fn delete_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), AppError> {
    validate_session_id(&session_id)?;
    state.storage.delete_session(&session_id).await
}

#[tauri::command]
pub async fn export_session_fit(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<String, AppError> {
    validate_session_id(&session_id)?;
    let summary = state.storage.get_session(&session_id).await?;
    let readings = state.storage.load_sensor_data(&session_id)?;
    let fit_data = fit_export::export_fit(&summary, &readings)?;

    let fit_path = std::path::Path::new(state.storage.data_dir())
        .join("sessions")
        .join(format!("{}.fit", session_id));
    std::fs::write(&fit_path, &fit_data)
        .map_err(|e| AppError::Serialization(format!("Failed to write FIT file: {}", e)))?;

    Ok(fit_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn check_prerequisites() -> Result<prerequisites::PrereqStatus, AppError> {
    tokio::task::spawn_blocking(prerequisites::check)
        .await
        .map_err(|e| AppError::Session(format!("Prereq check failed: {}", e)))
}

#[tauri::command]
pub async fn fix_prerequisites(
    app: tauri::AppHandle,
) -> Result<prerequisites::FixResult, AppError> {
    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|e| AppError::Session(format!("Cannot locate resource dir: {}", e)))?;
    let bundle_path = resource_dir.join("resources/99-ant-usb.rules");

    // Copy to /tmp so pkexec (running as root) can read it — AppImage FUSE
    // mounts are only accessible to the launching user, not root.
    let tmp_path = std::path::PathBuf::from("/tmp/99-ant-usb.rules");
    std::fs::copy(&bundle_path, &tmp_path).map_err(|e| {
        AppError::Session(format!("Failed to copy udev rules to /tmp: {}", e))
    })?;
    let source = tmp_path.to_string_lossy().to_string();

    tokio::task::spawn_blocking(move || {
        let result = prerequisites::fix(&source);
        let _ = std::fs::remove_file(&tmp_path);
        result
    })
    .await
    .map_err(|e| AppError::Session(format!("Prereq fix failed: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- validate_session_id ---

    #[test]
    fn valid_uuid_session_id() {
        assert!(validate_session_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
    }

    #[test]
    fn valid_hex_only_session_id() {
        assert!(validate_session_id("abcdef0123456789").is_ok());
    }

    #[test]
    fn rejects_path_traversal() {
        assert!(validate_session_id("../../etc/passwd").is_err());
    }

    #[test]
    fn rejects_dot_dot_slash() {
        assert!(validate_session_id("..").is_err());
    }

    #[test]
    fn rejects_empty_string() {
        assert!(validate_session_id("").is_err());
    }

    #[test]
    fn rejects_spaces() {
        assert!(validate_session_id("abc def").is_err());
    }

    #[test]
    fn rejects_null_bytes() {
        assert!(validate_session_id("abc\0def").is_err());
    }

    // --- auto_set_primary ---

    #[test]
    fn auto_set_primary_first_device_becomes_primary() {
        let mut primaries = HashMap::new();
        auto_set_primary(&mut primaries, DeviceType::Power, "dev-1");
        assert_eq!(primaries[&DeviceType::Power], "dev-1");
    }

    #[test]
    fn auto_set_primary_does_not_overwrite_existing() {
        let mut primaries = HashMap::new();
        auto_set_primary(&mut primaries, DeviceType::Power, "dev-1");
        auto_set_primary(&mut primaries, DeviceType::Power, "dev-2");
        assert_eq!(primaries[&DeviceType::Power], "dev-1");
    }

    #[test]
    fn auto_set_primary_different_types_independent() {
        let mut primaries = HashMap::new();
        auto_set_primary(&mut primaries, DeviceType::Power, "pm-1");
        auto_set_primary(&mut primaries, DeviceType::HeartRate, "hr-1");
        assert_eq!(primaries[&DeviceType::Power], "pm-1");
        assert_eq!(primaries[&DeviceType::HeartRate], "hr-1");
    }

    // --- remove_primary ---

    #[test]
    fn remove_primary_clears_matching_entry() {
        let mut primaries = HashMap::from([
            (DeviceType::Power, "dev-1".to_owned()),
            (DeviceType::HeartRate, "hr-1".to_owned()),
        ]);
        remove_primary(&mut primaries, "dev-1");
        assert!(!primaries.contains_key(&DeviceType::Power));
        assert_eq!(primaries[&DeviceType::HeartRate], "hr-1");
    }

    #[test]
    fn remove_primary_noop_for_unknown_device() {
        let mut primaries = HashMap::from([(DeviceType::Power, "dev-1".to_owned())]);
        remove_primary(&mut primaries, "nonexistent");
        assert_eq!(primaries.len(), 1);
    }

    #[test]
    fn remove_primary_clears_all_types_for_same_device() {
        // Edge case: same device_id registered under two types
        let mut primaries = HashMap::from([
            (DeviceType::Power, "multi-dev".to_owned()),
            (DeviceType::CadenceSpeed, "multi-dev".to_owned()),
        ]);
        remove_primary(&mut primaries, "multi-dev");
        assert!(primaries.is_empty());
    }

    // --- validate_zones_ascending ---

    #[test]
    fn ascending_hr_zones_valid() {
        assert!(validate_zones_ascending(&[100u8, 120, 140, 160, 180], "HR zones").is_ok());
    }

    #[test]
    fn equal_adjacent_zones_rejected() {
        assert!(validate_zones_ascending(&[100u8, 120, 120, 160, 180], "HR zones").is_err());
    }

    #[test]
    fn descending_zones_rejected() {
        assert!(validate_zones_ascending(&[100u16, 200, 150, 300, 400, 500], "Power zones").is_err());
    }

    #[test]
    fn single_zone_boundary_valid() {
        assert!(validate_zones_ascending(&[100u8], "HR zones").is_ok());
    }

    #[test]
    fn two_equal_zones_rejected() {
        assert!(validate_zones_ascending(&[150u16, 150], "Power zones").is_err());
    }

    #[test]
    fn error_message_includes_label() {
        let err = validate_zones_ascending(&[5u8, 3], "HR zones").unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("HR zones"), "expected label in error: {msg}");
    }

    // --- format_primaries ---

    #[test]
    fn format_primaries_empty_map() {
        let primaries = HashMap::new();
        assert!(format_primaries(&primaries).is_empty());
    }

    #[test]
    fn format_primaries_uses_debug_key_format() {
        let primaries = HashMap::from([
            (DeviceType::HeartRate, "hr-1".to_owned()),
            (DeviceType::FitnessTrainer, "trainer-1".to_owned()),
        ]);
        let result = format_primaries(&primaries);
        assert_eq!(result["HeartRate"], "hr-1");
        assert_eq!(result["FitnessTrainer"], "trainer-1");
    }
}
