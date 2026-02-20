mod commands;
mod device;
mod error;
mod prerequisites;
mod session;

use commands::AppState;
use device::manager::DeviceManager;
use flexi_logger::{
    Cleanup, Criterion, DeferredNow, Duplicate, FileSpec, Logger, Naming, WriteMode,
};
use log::Record;
use session::manager::SessionManager;
use session::storage::Storage;
use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use tauri::{Emitter, Manager, WindowEvent};
use tokio::sync::broadcast;

/// Stderr: colored, time-only, shortened module path
fn stderr_format(
    w: &mut dyn Write,
    now: &mut DeferredNow,
    record: &Record,
) -> std::io::Result<()> {
    let module = record
        .module_path()
        .unwrap_or("<unknown>")
        .strip_prefix("app_lib::")
        .unwrap_or(record.module_path().unwrap_or("<unknown>"));
    write!(
        w,
        "{} {:<5} [{}] {}",
        now.format("%H:%M:%S%.3f"),
        record.level(),
        module,
        record.args()
    )
}

/// File: no colors, full date+time, shortened module path
fn file_format(
    w: &mut dyn Write,
    now: &mut DeferredNow,
    record: &Record,
) -> std::io::Result<()> {
    let module = record
        .module_path()
        .unwrap_or("<unknown>")
        .strip_prefix("app_lib::")
        .unwrap_or(record.module_path().unwrap_or("<unknown>"));
    write!(
        w,
        "{} {:<5} [{}] {}",
        now.format("%Y-%m-%d %H:%M:%S%.3f"),
        record.level(),
        module,
        record.args()
    )
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Work around WebKitGTK EGL crashes in AppImage bundles on certain
    // GPU/driver combinations (AMD Mesa, some NVIDIA). Must be set before
    // WebKitGTK initializes. See: https://github.com/tauri-apps/tauri/issues/11988
    #[cfg(target_os = "linux")]
    {
        if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to get app data dir")
                .to_string_lossy()
                .to_string();

            let log_dir = std::path::Path::new(&data_dir).join("logs");
            std::fs::create_dir_all(&log_dir).expect("Failed to create log directory");

            #[cfg(feature = "production")]
            let log_spec = "warn, app_lib=info, btleplug=warn, rusb=warn, sqlx=warn";
            #[cfg(not(feature = "production"))]
            let log_spec = "info, btleplug=warn, rusb=warn, sqlx=warn";

            #[cfg(feature = "production")]
            let stderr_dup = Duplicate::Warn;
            #[cfg(not(feature = "production"))]
            let stderr_dup = Duplicate::All;

            let _logger = Logger::try_with_env_or_str(log_spec)
            .expect("Failed to parse log spec")
            .log_to_file(
                FileSpec::default()
                    .directory(&log_dir)
                    .basename("training-app"),
            )
            .rotate(
                Criterion::Size(5_000_000),
                Naming::Timestamps,
                Cleanup::KeepLogFiles(5),
            )
            .write_mode(WriteMode::BufferAndFlush)
            .format_for_files(file_format)
            .duplicate_to_stderr(stderr_dup)
            .format_for_stderr(stderr_format)
            .start()
            .expect("Failed to start logger");

            // Leak the handle — logger must live for the process lifetime.
            // Dropping it deregisters the logger and stops all logging.
            Box::leak(Box::new(_logger));

            log::info!("Logging to {}", log_dir.display());

            let (sensor_tx, _) = broadcast::channel(256);
            let app_handle = app.handle().clone();

            let state = tauri::async_runtime::block_on(async {
                let storage = Storage::new(&data_dir)
                    .await
                    .expect("Failed to initialize storage");

                // Recover any sessions from autosave files (crash recovery)
                match storage.recover_autosaved_sessions().await {
                    Ok(0) => {}
                    Ok(n) => log::info!("Recovered {} autosaved session(s)", n),
                    Err(e) => log::warn!("Autosave recovery failed: {}", e),
                }

                let session_manager = Arc::new(SessionManager::new());
                let primary_devices: Arc<tokio::sync::Mutex<HashMap<crate::device::types::DeviceType, String>>> =
                    Arc::new(tokio::sync::Mutex::new(HashMap::new()));

                // I6: Spawn a single global processor task that handles ALL sensor readings.
                // This replaces the per-device processor tasks that caused duplicate processing.
                let session_mgr_clone = session_manager.clone();
                let primaries_clone = primary_devices.clone();
                let sensor_rx: broadcast::Receiver<crate::device::types::SensorReading> = sensor_tx.subscribe();
                let handle = app_handle.clone();
                tokio::spawn(async move {
                    let mut rx = sensor_rx;
                    loop {
                        match rx.recv().await {
                            Ok(reading) => {
                                // Check if this reading's device is the primary for its type
                                let dominated = {
                                    let primaries = primaries_clone.lock().await;
                                    if let Some(primary_id) = primaries.get(&reading.device_type()) {
                                        !reading.device_id().is_empty() && reading.device_id() != primary_id
                                    } else {
                                        false
                                    }
                                };
                                if dominated {
                                    continue;
                                }
                                session_mgr_clone.process_reading(reading.clone()).await;
                                let _ = handle.emit("sensor_reading", &reading);
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                log::warn!("Dropped {} sensor readings", n);
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                });

                let storage = Arc::new(storage);
                let mut device_manager = DeviceManager::new();
                device_manager.set_storage(storage.clone());

                let device_manager = Arc::new(tokio::sync::Mutex::new(device_manager));

                // Connection watchdog: every 5s, check for silently-disconnected devices
                // and attempt reconnects. DeviceManager.check_connections() handles all
                // internal cleanup (listener handles, trainer backends, connected_devices).
                // This watchdog cleans up primaries, emits frontend events, and drives
                // the auto-reconnect engine.
                {
                    let dm = device_manager.clone();
                    let primaries = primary_devices.clone();
                    let handle = app_handle.clone();
                    let sensor_tx_clone = sensor_tx.clone();
                    tokio::spawn(async move {
                        loop {
                            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

                            let disconnected = {
                                let mut dm = dm.lock().await;
                                dm.check_connections().await
                            };

                            if !disconnected.is_empty() {
                                // Clean up primaries
                                {
                                    let mut p = primaries.lock().await;
                                    let ids: Vec<String> =
                                        disconnected.iter().map(|i| i.id.clone()).collect();
                                    p.retain(|_, v| !ids.contains(v));
                                }

                                // Emit disconnect events to frontend
                                for info in &disconnected {
                                    let _ = handle.emit("device_disconnected", &info.id);
                                }
                            }

                            // Attempt reconnects for devices due for retry
                            let (reconnected, trying) = {
                                let mut dm = dm.lock().await;
                                dm.attempt_reconnects(&sensor_tx_clone).await
                            };

                            for info in &reconnected {
                                let _ = handle.emit("device_reconnected", &info.id);
                                let mut p = primaries.lock().await;
                                p.entry(info.device_type)
                                    .or_insert_with(|| info.id.clone());
                            }

                            for (info, attempt) in &trying {
                                let _ =
                                    handle.emit("device_reconnecting", &serde_json::json!({
                                        "device_id": info.id,
                                        "device_type": format!("{:?}", info.device_type),
                                        "attempt": attempt,
                                    }));
                            }
                        }
                    });
                }

                // Autosave task: every 30s, snapshot the active session to disk
                {
                    let session_mgr = session_manager.clone();
                    let storage_clone = storage.clone();
                    tokio::spawn(async move {
                        loop {
                            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                            if let Some((session_id, summary, sensor_log)) =
                                session_mgr.snapshot_for_autosave().await
                            {
                                if let Err(e) =
                                    storage_clone.write_autosave(&session_id, &summary, &sensor_log)
                                {
                                    log::warn!("Autosave failed: {}", e);
                                }
                            }
                        }
                    });
                }

                AppState {
                    device_manager,
                    session_manager,
                    storage,
                    sensor_tx,
                    primary_devices,
                }
            });

            app.manage(state);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { .. } = event {
                let state = window.state::<AppState>();
                let session_mgr = state.session_manager.clone();
                let storage = state.storage.clone();
                tauri::async_runtime::block_on(async {
                    // Save active session before shutdown
                    if let Some((summary, sensor_log)) = session_mgr.stop_session_with_log().await {
                        let raw_data = bincode::serialize(&sensor_log).unwrap_or_default();
                        if let Err(e) = storage.save_session(&summary, &raw_data).await {
                            log::warn!("Failed to save session on shutdown: {}", e);
                        }
                        storage.remove_autosave(&summary.id);
                    }
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::scan_devices,
            commands::connect_device,
            commands::disconnect_device,
            commands::get_known_devices,
            commands::get_device_details,
            commands::start_session,
            commands::stop_session,
            commands::pause_session,
            commands::resume_session,
            commands::get_live_metrics,
            commands::list_sessions,
            commands::get_user_config,
            commands::save_user_config,
            commands::set_trainer_power,
            commands::set_trainer_resistance,
            commands::set_trainer_simulation,
            commands::start_trainer,
            commands::stop_trainer,
            commands::export_session_fit,
            commands::update_session_metadata,
            commands::delete_session,
            commands::set_primary_device,
            commands::get_primary_devices,
            commands::unlink_devices,
            commands::check_prerequisites,
            commands::fix_prerequisites,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
