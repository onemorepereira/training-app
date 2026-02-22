use log::{info, warn};
use std::path::Path;

use super::Storage;
use crate::commands::validate_session_id;
use crate::device::types::SensorReading;
use crate::error::AppError;
use crate::session::types::SessionSummary;

impl Storage {
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
}
