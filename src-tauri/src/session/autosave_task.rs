//! One tick of the periodic autosave (the loop itself lives in `lib.rs`).

use crate::device::types::SensorReading;
use crate::session::manager::{AutosaveTicket, SessionManager};
use crate::session::storage::Storage;

/// Full sensor log of the active session, rebuilt from per-tick deltas.
#[derive(Default)]
pub struct AutosaveAccumulator {
    session_id: Option<String>,
    log: Vec<SensorReading>,
}

/// Snapshot the active session and write its autosave while holding the
/// autosave gate, so a concurrent stop waits for this write and no write can
/// start after stop. Returns true if an autosave was written.
pub async fn autosave_tick(
    session_mgr: &SessionManager,
    storage: &Storage,
    acc: &mut AutosaveAccumulator,
) -> bool {
    let Some(AutosaveTicket {
        gate,
        session_id,
        summary,
        delta,
    }) = session_mgr.autosave_ticket().await
    else {
        acc.session_id = None;
        acc.log.clear();
        return false;
    };
    if acc.session_id.as_deref() != Some(session_id.as_str()) {
        acc.log.clear();
        acc.session_id = Some(session_id.clone());
    }
    acc.log.extend(delta);
    let written = match storage.write_autosave(&session_id, &summary, &acc.log).await {
        Ok(()) => true,
        Err(e) => {
            log::warn!("Autosave failed: {}", e);
            false
        }
    };
    drop(gate);
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::types::SessionConfig;
    use std::sync::Arc;
    use std::time::Duration;

    fn power(watts: u16, epoch_ms: u64) -> SensorReading {
        SensorReading::Power {
            watts,
            timestamp: None,
            epoch_ms,
            device_id: "pm".to_string(),
            pedal_balance: None,
        }
    }

    async fn storage() -> (Storage, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let s = Storage::new(&tmp.path().to_string_lossy()).await.unwrap();
        (s, tmp)
    }

    fn autosave_exists(storage: &Storage, id: &str) -> bool {
        std::path::Path::new(storage.data_dir())
            .join("sessions")
            .join(format!(".autosave_{id}.bin"))
            .exists()
    }

    #[tokio::test]
    async fn ticks_accumulate_full_log_across_deltas() {
        let (storage, tmp) = storage().await;
        let mgr = SessionManager::new();
        let id = mgr.start_session(SessionConfig::default()).await.unwrap();
        let mut acc = AutosaveAccumulator::default();
        for i in 0..3 {
            mgr.process_reading(power(200, 1000 * (i + 1))).await;
        }
        assert!(autosave_tick(&mgr, &storage, &mut acc).await);
        for i in 3..5 {
            mgr.process_reading(power(200, 1000 * (i + 1))).await;
        }
        assert!(autosave_tick(&mgr, &storage, &mut acc).await);
        drop(storage);

        // Crash: recover from the autosave alone
        let storage = Storage::new(&tmp.path().to_string_lossy()).await.unwrap();
        assert_eq!(storage.recover_autosaved_sessions().await.unwrap(), 1);
        assert_eq!(storage.load_sensor_data(&id).unwrap().len(), 5);
    }

    #[tokio::test]
    async fn autosave_tick_after_stop_writes_nothing() {
        let (storage, _tmp) = storage().await;
        let mgr = SessionManager::new();
        let id = mgr.start_session(SessionConfig::default()).await.unwrap();
        mgr.process_reading(power(250, 1000)).await;
        let (summary, log) = mgr.stop_session_with_log().await.unwrap();
        storage.finalize_session(&summary, &log).await.unwrap();

        let mut acc = AutosaveAccumulator::default();
        assert!(!autosave_tick(&mgr, &storage, &mut acc).await);
        assert!(!autosave_exists(&storage, &id));
    }

    #[tokio::test]
    async fn stop_waits_for_in_flight_autosave_ticket() {
        let mgr = Arc::new(SessionManager::new());
        mgr.start_session(SessionConfig::default()).await.unwrap();
        let ticket = mgr.autosave_ticket().await.unwrap();

        let mgr2 = mgr.clone();
        let mut stop = tokio::spawn(async move { mgr2.stop_session_with_log().await });
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut stop).await.is_err(),
            "stop must block while an autosave write is in flight"
        );
        drop(ticket);
        let stopped = tokio::time::timeout(Duration::from_secs(2), stop)
            .await
            .expect("stop proceeds once the ticket is dropped")
            .unwrap();
        assert!(stopped.is_some());
        assert!(mgr.autosave_ticket().await.is_none(), "no snapshot after stop");
    }
}
