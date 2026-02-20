use std::collections::HashMap;
use std::sync::Arc;
use tauri::{Manager, State};
use tokio::sync::broadcast;

use crate::device::manager::DeviceManager;
use crate::device::types::{DeviceDetails, DeviceInfo, DeviceType, SensorReading};
use crate::error::AppError;
use crate::prerequisites;
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
        primaries.entry(info.device_type).or_insert_with(|| device_id.clone());
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
        primaries.retain(|_, v| v != &device_id);
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
pub async fn get_user_config(state: State<'_, AppState>) -> Result<SessionConfig, AppError> {
    state.storage.get_user_config().await.map_err(AppError::from)
}

#[tauri::command]
pub async fn save_user_config(
    state: State<'_, AppState>,
    config: SessionConfig,
) -> Result<(), AppError> {
    // Validate HR zones are strictly ascending
    for w in config.hr_zones.windows(2) {
        if w[0] >= w[1] {
            return Err(AppError::Session(
                "HR zones must be strictly ascending".into(),
            ));
        }
    }
    // Validate power zones are strictly ascending
    for w in config.power_zones.windows(2) {
        if w[0] >= w[1] {
            return Err(AppError::Session(
                "Power zones must be strictly ascending".into(),
            ));
        }
    }
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
    let result: HashMap<String, String> = primaries
        .iter()
        .map(|(k, v)| (format!("{:?}", k), v.clone()))
        .collect();
    Ok(result)
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
}
