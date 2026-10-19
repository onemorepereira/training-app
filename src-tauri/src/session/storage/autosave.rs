use log::{error, info, warn};
use std::path::PathBuf;

use super::fsio::{remove_durable, write_durable};
use super::{SaveOutcome, Storage};
use crate::commands::validate_session_id;
use crate::device::types::SensorReading;
use crate::error::AppError;
use crate::session::types::SessionSummary;

/// Serialize an autosave: 4-byte JSON length (LE) + JSON summary + bincode log.
fn encode_autosave(
    summary: &SessionSummary,
    sensor_log: &[SensorReading],
) -> Result<Vec<u8>, AppError> {
    let json_bytes =
        serde_json::to_vec(summary).map_err(|e| AppError::Serialization(e.to_string()))?;
    let sensor_bytes =
        bincode::serialize(sensor_log).map_err(|e| AppError::Serialization(e.to_string()))?;
    let mut data = Vec::with_capacity(4 + json_bytes.len() + sensor_bytes.len());
    data.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    data.extend_from_slice(&json_bytes);
    data.extend_from_slice(&sensor_bytes);
    Ok(data)
}

impl Storage {
    pub(crate) fn autosave_path(&self, session_id: &str) -> PathBuf {
        self.sessions_dir()
            .join(format!(".autosave_{}.bin", session_id))
    }

    /// Write an autosave checkpoint for a running session (durable, atomic).
    pub async fn write_autosave(
        &self,
        session_id: &str,
        summary: &SessionSummary,
        sensor_log: &[SensorReading],
    ) -> Result<(), AppError> {
        let data = encode_autosave(summary, sensor_log)?;
        write_durable(&self.autosave_path(session_id), &data).await
    }

    /// Remove the autosave file for a session. A missing file is not an error.
    pub async fn remove_autosave(&self, session_id: &str) -> Result<(), AppError> {
        remove_durable(&self.autosave_path(session_id)).await.map(|_| ())
    }

    /// Persist a stopped session without ever losing its data:
    /// 1. durable full autosave (so a failure below still leaves everything on disk),
    /// 2. raw file + row (`save_session`),
    /// 3. remove the autosave.
    ///
    /// On a save failure the autosave is kept and recovered on next launch.
    pub async fn finalize_session(
        &self,
        summary: &SessionSummary,
        sensor_log: &[SensorReading],
    ) -> Result<(), AppError> {
        let autosave_ok = match self.write_autosave(&summary.id, summary, sensor_log).await {
            Ok(()) => true,
            Err(e) => {
                warn!("Final autosave for session {} failed: {}", summary.id, e);
                false
            }
        };
        let raw = bincode::serialize(sensor_log)
            .map_err(|e| AppError::Serialization(e.to_string()));
        let saved = match raw {
            Ok(raw) => self.save_session(summary, &raw).await,
            Err(e) => Err(e),
        };
        if let Err(e) = saved {
            error!("Saving session {} failed: {}", summary.id, e);
            let kept = if autosave_ok {
                "It was kept on disk and will be recovered the next time the app starts."
            } else {
                "The last autosave (up to 30 seconds old) will be recovered the next time the app starts."
            };
            return Err(AppError::Session(format!(
                "Ride could not be saved: {}. {}",
                e, kept
            )));
        }
        if let Err(e) = self.remove_autosave(&summary.id).await {
            warn!(
                "Session {} saved but its autosave could not be removed (cleaned up at next start): {}",
                summary.id, e
            );
        }
        Ok(())
    }

    /// Scan for autosave files, persist each via `save_session` (which never
    /// overwrites a finished ride), and delete the autosave. Also removes stray
    /// `*.tmp` files left by an interrupted durable write.
    /// Returns the count of sessions newly inserted.
    pub async fn recover_autosaved_sessions(&self) -> Result<usize, AppError> {
        let sessions_dir = self.sessions_dir();
        if !sessions_dir.exists() {
            return Ok(0);
        }

        let mut count = 0;
        let entries = std::fs::read_dir(&sessions_dir)
            .map_err(|e| AppError::Serialization(format!("Failed to read sessions dir: {}", e)))?;

        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.ends_with(".tmp") {
                info!("Removing interrupted write {}", name_str);
                if let Err(e) = remove_durable(&entry.path()).await {
                    warn!("Failed to remove {}: {}", name_str, e);
                }
                continue;
            }
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
                let _ = remove_durable(&entry.path()).await;
                continue;
            }

            let json_len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
            if data.len() < 4 + json_len {
                warn!("Autosave {} truncated, skipping", name_str);
                let _ = remove_durable(&entry.path()).await;
                continue;
            }

            let summary: SessionSummary = match serde_json::from_slice(&data[4..4 + json_len]) {
                Ok(s) => s,
                Err(e) => {
                    warn!("Autosave {} bad JSON: {}", name_str, e);
                    let _ = remove_durable(&entry.path()).await;
                    continue;
                }
            };

            let sensor_bytes = &data[4 + json_len..];

            if validate_session_id(&summary.id).is_err() {
                warn!("Autosave {} has invalid session ID, skipping", name_str);
                let _ = remove_durable(&entry.path()).await;
                continue;
            }

            match self.save_session(&summary, sensor_bytes).await {
                Ok(outcome) => {
                    match outcome {
                        SaveOutcome::Inserted => {
                            info!("Recovered autosaved session {}", summary.id);
                            count += 1;
                        }
                        SaveOutcome::RepairedRaw => {
                            info!("Restored missing raw file for session {}", summary.id)
                        }
                        SaveOutcome::AlreadyExisted => {
                            info!("Discarding stale autosave for saved session {}", summary.id)
                        }
                    }
                    if let Err(e) = remove_durable(&entry.path()).await {
                        error!("Failed to remove autosave {} (retried next start): {}", name_str, e);
                    }
                }
                Err(e) => {
                    warn!("Failed to recover autosave {}: {}", summary.id, e);
                }
            }
        }

        Ok(count)
    }
}
