use log::{info, warn};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{Emitter, State};
use tokio::sync::broadcast;

use crate::device::manager::DeviceManager;
use crate::device::primary::{PrimaryKey, SharedPrimaryState};
use crate::device::types::{DeviceDetails, DeviceInfo, SensorReading};
use crate::error::AppError;
use crate::prerequisites;
use crate::session::analysis::{self, PowerCurvePoint, SessionAnalysis};
use crate::session::fit_export;
use crate::session::manager::SessionManager;
use crate::session::storage::Storage;
use crate::session::types::{SessionConfig, SessionSummary};
use crate::session::analysis::{compute_hr_power_regression, TimeseriesPoint};
use crate::session::zone_control::controller::ZoneController;
use crate::session::zone_control::lifecycle as zone_lifecycle;
use crate::session::zone_control::types::{
    StopReason, ZoneControlStatus, ZoneMode, ZoneTarget, MAX_HR_BOUND,
};

/// Validate that a session ID from the frontend is a safe UUID string.
/// Prevents path traversal via crafted IDs like "../../etc/passwd".
pub(crate) fn validate_session_id(id: &str) -> Result<(), AppError> {
    if !id.is_empty() && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        Ok(())
    } else {
        Err(AppError::Session("Invalid session ID".into()))
    }
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

/// Validate a user config before saving (design ADR-9): FTP range, zone
/// ordering, and per-value bounds the zone controller relies on.
pub(crate) fn validate_user_config(config: &SessionConfig) -> Result<(), AppError> {
    if !(50..=1000).contains(&config.ftp) {
        return Err(AppError::Session(format!(
            "FTP must be between 50 and 1000 W, got {}",
            config.ftp
        )));
    }
    validate_zones_ascending(&config.hr_zones, "HR zones")?;
    validate_zones_ascending(&config.power_zones, "Power zones")?;
    if let Some(&hr) = config.hr_zones.iter().find(|&&z| z as u16 > MAX_HR_BOUND) {
        return Err(AppError::Session(format!(
            "HR zone boundaries must be at most {} bpm, got {}",
            MAX_HR_BOUND, hr
        )));
    }
    if let Some(&pct) = config.power_zones.iter().find(|&&z| z == 0 || z > 1000) {
        return Err(AppError::Session(format!(
            "Power zone percentages must be between 1 and 1000, got {}",
            pct
        )));
    }
    Ok(())
}

pub struct AppState {
    pub device_manager: Arc<tokio::sync::Mutex<DeviceManager>>,
    pub session_manager: Arc<SessionManager>,
    pub storage: Arc<Storage>,
    pub sensor_tx: broadcast::Sender<SensorReading>,
    pub primary_devices: SharedPrimaryState,
    pub zone_controller: Arc<ZoneController>,
    #[cfg(not(feature = "production"))]
    pub simulator: Arc<tokio::sync::Mutex<crate::simulator::Simulator>>,
}

#[tauri::command]
pub async fn scan_devices(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<Vec<DeviceInfo>, AppError> {
    info!("Scanning for devices");
    let devices = DeviceManager::scan_shared(&state.device_manager).await?;
    let _ = app.emit("device_list_updated", &devices);
    Ok(devices)
}

#[tauri::command]
pub async fn connect_device(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    device_id: String,
) -> Result<DeviceInfo, AppError> {
    info!("Connecting device: {}", device_id);
    let tx = state.sensor_tx.clone();
    let result = DeviceManager::connect_shared(&state.device_manager, &device_id, tx).await;

    // Emit on success and failure: a failed connect can change the device's status.
    let all = state.device_manager.lock().await.list_current().await;
    let _ = app.emit("device_list_updated", &all);

    result
}

#[tauri::command]
pub async fn disconnect_device(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    device_id: String,
) -> Result<(), AppError> {
    info!("Disconnecting device: {}", device_id);
    DeviceManager::disconnect_shared(&state.device_manager, &device_id).await?;

    let all = state.device_manager.lock().await.list_current().await;
    let _ = app.emit("device_list_updated", &all);

    Ok(())
}

#[tauri::command]
pub async fn start_session(state: State<'_, AppState>) -> Result<String, AppError> {
    info!("Starting session");
    let config = state.storage.get_user_config().await?;
    let id = state.session_manager.start_session(config).await?;
    Ok(id)
}

#[tauri::command]
pub async fn stop_session(state: State<'_, AppState>) -> Result<Option<SessionSummary>, AppError> {
    zone_lifecycle::on_session_stop(&state.zone_controller).await;
    let result = state.session_manager.stop_session_with_log().await;

    if let Some((ref summary, ref sensor_log)) = result {
        info!(
            "Session stopped: id={}, duration={}s",
            summary.id, summary.duration_secs
        );
        // Never drops data: on failure the full log stays on disk as an autosave
        // and the error tells the user it will be recovered on next launch.
        state.storage.finalize_session(summary, sensor_log).await?;

        // Save power curve in background
        let storage = state.storage.clone();
        let session_id = summary.id.clone();
        let readings = sensor_log.clone();
        tokio::spawn(async move {
            let curve = analysis::compute_power_curve_from_readings(&readings);
            if !curve.is_empty() {
                if let Err(e) = storage.save_power_curve(&session_id, &curve).await {
                    log::warn!("Failed to save power curve: {}", e);
                }
            }
        });
    } else {
        info!("Stop session: no active session");
    }

    Ok(result.map(|(summary, _)| summary))
}

#[tauri::command]
pub async fn pause_session(state: State<'_, AppState>) -> Result<(), AppError> {
    zone_lifecycle::on_session_pause(&state.zone_controller).await;
    state.session_manager.pause_session().await;
    Ok(())
}

#[tauri::command]
pub async fn resume_session(state: State<'_, AppState>) -> Result<(), AppError> {
    zone_lifecycle::on_session_resume(&state.zone_controller).await;
    state.session_manager.resume_session().await;
    Ok(())
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
    validate_user_config(&config)?;
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
    DeviceManager::device_details_shared(&state.device_manager, &device_id).await
}

/// `device_type` accepts the legacy DeviceType names and the per-metric names
/// "Cadence" / "Speed" (see device::primary::PrimaryKey).
#[tauri::command]
pub async fn set_primary_device(
    state: State<'_, AppState>,
    device_type: PrimaryKey,
    device_id: String,
) -> Result<(), AppError> {
    info!("Set primary device: {:?} = {}", device_type, device_id);
    let mut primaries = state
        .primary_devices
        .write()
        .map_err(|e| AppError::Session(format!("Lock poisoned: {e}")))?;
    primaries.set_primary(device_type, &device_id);
    Ok(())
}

#[tauri::command]
pub async fn get_primary_devices(
    state: State<'_, AppState>,
) -> Result<HashMap<String, String>, AppError> {
    let primaries = state
        .primary_devices
        .read()
        .map_err(|e| AppError::Session(format!("Lock poisoned: {e}")))?;
    Ok(primaries.legacy_map())
}

#[tauri::command]
pub async fn set_trainer_power(state: State<'_, AppState>, watts: i16) -> Result<(), AppError> {
    info!("Set trainer power: {}W", watts);
    state.zone_controller.ensure_manual_control_allowed()?;
    let mut dm = state.device_manager.lock().await;
    let trainer_id = dm
        .connected_trainer_id()
        .ok_or_else(|| AppError::Session("No trainer connected".into()))?;
    dm.set_target_power(&trainer_id, watts).await
}

#[tauri::command]
pub async fn set_trainer_resistance(state: State<'_, AppState>, level: u8) -> Result<(), AppError> {
    info!("Set trainer resistance: level {}", level);
    state.zone_controller.ensure_manual_control_allowed()?;
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
    info!("Set trainer simulation: grade={}, crr={}, cw={}", grade, crr, cw);
    state.zone_controller.ensure_manual_control_allowed()?;
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
    info!("Deleting session: {}", session_id);
    state.storage.delete_session(&session_id).await
}

#[tauri::command]
pub async fn export_session_fit(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<String, AppError> {
    validate_session_id(&session_id)?;
    info!("Exporting session to FIT: {}", session_id);
    let summary = state.storage.get_session(&session_id).await?;
    let readings = state.storage.load_sensor_data(&session_id)?;
    let fit_data = fit_export::export_fit(&summary, &readings)?;

    let fit_path = std::path::Path::new(state.storage.data_dir())
        .join("sessions")
        .join(format!("{}.fit", session_id));
    tokio::fs::write(&fit_path, &fit_data)
        .await
        .map_err(|e| AppError::Serialization(format!("Failed to write FIT file: {}", e)))?;

    Ok(fit_path.to_string_lossy().to_string())
}

/// Load recent sessions with both HR and power data, compute regression,
/// and estimate the power needed to produce `target_hr`.
async fn estimate_power_from_history(
    storage: &Arc<Storage>,
    target_hr: u8,
) -> Result<Option<u16>, AppError> {
    let sessions = storage.list_sessions().await?;
    // Take last 5 sessions (most recent first from list_sessions)
    let recent: Vec<_> = sessions.into_iter().take(5).collect();

    if recent.is_empty() {
        return Ok(None);
    }

    // Build combined timeseries from recent sessions
    let storage_clone = storage.clone();
    let combined = tokio::task::spawn_blocking(move || {
        let mut all_points: Vec<TimeseriesPoint> = Vec::new();
        for session in &recent {
            if let Ok(readings) = storage_clone.load_sensor_data(&session.id) {
                let ts = crate::session::analysis::build_timeseries_from_readings(&readings);
                all_points.extend(ts);
            }
        }
        all_points
    })
    .await
    .map_err(|e| AppError::Session(format!("Failed to load history: {}", e)))?;

    let model = match compute_hr_power_regression(&combined) {
        Some(m) => m,
        None => return Ok(None),
    };

    // Estimate: power = (target_hr - intercept) / slope
    if model.slope.abs() < 1e-6 {
        return Ok(None);
    }
    let estimated = ((target_hr as f64 - model.intercept) / model.slope).round();
    if estimated < 50.0 || estimated > 500.0 {
        return Ok(None);
    }

    Ok(Some(estimated as u16))
}

#[tauri::command]
pub async fn start_zone_control(
    state: State<'_, AppState>,
    target: ZoneTarget,
) -> Result<(), AppError> {
    info!(
        "Start zone control: {:?} zone {} ({}-{})",
        target.mode, target.zone, target.lower_bound, target.upper_bound
    );
    target.validate()?;
    let config = state.storage.get_user_config().await?;
    let ftp = Some(config.ftp);
    let max_hr = config.max_hr;

    // For HR mode, try to estimate initial power from historical data
    let initial_power_estimate = if target.mode == ZoneMode::HeartRate {
        // validate() caps HR bounds at 250, so the midpoint fits in u8
        let target_hr = target.midpoint().min(u8::MAX as u16) as u8;
        match estimate_power_from_history(&state.storage, target_hr).await {
            Ok(v) => v,
            Err(e) => {
                warn!("Failed to estimate initial power from history: {}", e);
                None
            }
        }
    } else {
        None
    };

    let power_zones = Some(config.power_zones);
    let tx = state.sensor_tx.clone();
    state
        .zone_controller
        .start_with_config(target, tx, ftp, max_hr, initial_power_estimate, power_zones)
        .await
}

#[tauri::command]
pub async fn estimate_initial_power(
    state: State<'_, AppState>,
    target_hr: u8,
) -> Result<Option<u16>, AppError> {
    estimate_power_from_history(&state.storage, target_hr).await
}

#[tauri::command]
pub async fn stop_zone_control(
    state: State<'_, AppState>,
) -> Result<Option<StopReason>, AppError> {
    let reason = state.zone_controller.stop(StopReason::UserStopped).await;
    info!("Stop zone control: {:?}", reason);
    Ok(reason)
}

#[tauri::command]
pub async fn pause_zone_control(state: State<'_, AppState>) -> Result<(), AppError> {
    state.zone_controller.pause().await;
    Ok(())
}

#[tauri::command]
pub async fn resume_zone_control(state: State<'_, AppState>) -> Result<(), AppError> {
    state.zone_controller.resume().await;
    Ok(())
}

#[tauri::command]
pub async fn get_zone_control_status(
    state: State<'_, AppState>,
) -> Result<ZoneControlStatus, AppError> {
    Ok(state.zone_controller.status().await)
}

#[tauri::command]
pub async fn save_zone_ride_config(
    state: State<'_, AppState>,
    session_id: String,
    zone_config: String,
) -> Result<(), AppError> {
    validate_session_id(&session_id)?;
    state.storage.save_zone_config(&session_id, &zone_config).await
}

#[tauri::command]
pub async fn get_zone_ride_config(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Option<String>, AppError> {
    validate_session_id(&session_id)?;
    state.storage.get_zone_config(&session_id).await
}

#[tauri::command]
pub async fn get_best_power_curve(
    state: State<'_, AppState>,
    period: String,
) -> Result<Vec<PowerCurvePoint>, AppError> {
    let after_date = match period.as_str() {
        "30d" => {
            let dt = chrono::Utc::now() - chrono::Duration::days(30);
            Some(dt.to_rfc3339())
        }
        "90d" => {
            let dt = chrono::Utc::now() - chrono::Duration::days(90);
            Some(dt.to_rfc3339())
        }
        _ => None,
    };
    state
        .storage
        .get_best_power_curve(after_date.as_deref())
        .await
}

#[tauri::command]
pub async fn backfill_power_curves(state: State<'_, AppState>) -> Result<u32, AppError> {
    let sessions = state.storage.list_sessions().await?;
    let mut filled = 0u32;
    for session in &sessions {
        if state.storage.has_power_curve(&session.id).await? {
            continue;
        }
        let storage = state.storage.clone();
        let sid = session.id.clone();
        let curve = tokio::task::spawn_blocking(move || {
            let readings = storage.load_sensor_data(&sid)?;
            Ok::<_, AppError>(analysis::compute_power_curve_from_readings(&readings))
        })
        .await
        .map_err(|e| AppError::Session(format!("Backfill failed: {}", e)))??;

        if curve.is_empty() {
            continue;
        }
        state.storage.save_power_curve(&session.id, &curve).await?;
        filled += 1;
    }
    Ok(filled)
}

#[tauri::command]
pub async fn check_prerequisites() -> Result<prerequisites::PrereqStatus, AppError> {
    tokio::task::spawn_blocking(prerequisites::check)
        .await
        .map_err(|e| AppError::Session(format!("Prereq check failed: {}", e)))
}

#[tauri::command]
pub async fn fix_prerequisites() -> Result<prerequisites::FixResult, AppError> {
    // One pkexec prompt; the udev rule is embedded and piped on stdin, so no
    // file is staged anywhere (design ADR-4).
    tokio::task::spawn_blocking(prerequisites::fix)
        .await
        .map_err(|e| AppError::Session(format!("Prereq fix failed: {}", e)))
}

#[cfg(not(feature = "production"))]
#[tauri::command]
pub async fn sim_start(
    state: State<'_, AppState>,
    profile: crate::simulator::SimProfile,
) -> Result<(), AppError> {
    info!("Starting simulator: {:?}", profile);
    let tx = state.sensor_tx.clone();
    let mut sim = state.simulator.lock().await;
    sim.start(profile, tx);
    Ok(())
}

#[cfg(not(feature = "production"))]
#[tauri::command]
pub async fn sim_stop(state: State<'_, AppState>) -> Result<(), AppError> {
    info!("Stopping simulator");
    let mut sim = state.simulator.lock().await;
    sim.stop();
    Ok(())
}

#[cfg(not(feature = "production"))]
#[tauri::command]
pub async fn sim_status(
    state: State<'_, AppState>,
) -> Result<crate::simulator::SimStatusResponse, AppError> {
    let sim = state.simulator.lock().await;
    Ok(sim.status())
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

    // --- validate_user_config ---

    fn config_with_ftp(ftp: u16) -> SessionConfig {
        SessionConfig { ftp, ..SessionConfig::default() }
    }

    #[test]
    fn ftp_below_50_rejected() {
        let msg = validate_user_config(&config_with_ftp(49)).unwrap_err().to_string();
        assert!(msg.contains("FTP must be between 50 and 1000 W, got 49"), "{msg}");
        assert!(validate_user_config(&config_with_ftp(50)).is_ok());
    }

    #[test]
    fn ftp_above_1000_rejected() {
        assert!(validate_user_config(&config_with_ftp(1001)).is_err());
        assert!(validate_user_config(&config_with_ftp(1000)).is_ok());
    }

    #[test]
    fn non_increasing_hr_zones_rejected() {
        let mut c = SessionConfig::default();
        c.hr_zones = [120, 140, 140, 175, 190];
        assert!(validate_user_config(&c).is_err());
    }

    #[test]
    fn hr_zone_above_250_rejected() {
        let mut c = SessionConfig::default();
        c.hr_zones = [120, 140, 160, 175, 251];
        let msg = validate_user_config(&c).unwrap_err().to_string();
        assert!(msg.contains("at most 250 bpm, got 251"), "{msg}");
    }

    #[test]
    fn zero_power_zone_percentage_rejected() {
        let mut c = SessionConfig::default();
        c.power_zones = [0, 75, 90, 105, 120, 150];
        assert!(validate_user_config(&c).is_err());
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
}
