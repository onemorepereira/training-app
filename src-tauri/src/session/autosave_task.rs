//! One tick of the periodic autosave (the loop itself lives in `lib.rs`).

use crate::device::types::SensorReading;
use crate::error::AppError;
use crate::session::manager::{AutosaveTicket, SessionManager};
use crate::session::storage::Storage;
use crate::session::types::SessionSummary;

/// Destination of autosave snapshots (the real one is [`Storage`]); a trait so
/// tests can gate the write and observe its ordering against stop.
pub trait AutosaveWriter {
    async fn write_autosave(
        &self,
        session_id: &str,
        summary: &SessionSummary,
        sensor_log: &[SensorReading],
    ) -> Result<(), AppError>;
}

impl AutosaveWriter for Storage {
    async fn write_autosave(
        &self,
        session_id: &str,
        summary: &SessionSummary,
        sensor_log: &[SensorReading],
    ) -> Result<(), AppError> {
        Storage::write_autosave(self, session_id, summary, sensor_log).await
    }
}

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
    autosave_tick_with(session_mgr, storage, acc).await
}

/// [`autosave_tick`] against any writer. The ticket (and with it the autosave
/// gate) is held until the write has completed.
pub async fn autosave_tick_with<W: AutosaveWriter>(
    session_mgr: &SessionManager,
    writer: &W,
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
    let written = match writer.write_autosave(&session_id, &summary, &acc.log).await {
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

    /// Fake writer that blocks inside the write until released.
    struct GatedWriter {
        entered: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
        events: Arc<std::sync::Mutex<Vec<&'static str>>>,
    }

    impl AutosaveWriter for GatedWriter {
        async fn write_autosave(
            &self,
            _session_id: &str,
            _summary: &SessionSummary,
            _sensor_log: &[SensorReading],
        ) -> Result<(), AppError> {
            if let Some(tx) = self.entered.lock().unwrap().take() {
                let _ = tx.send(());
            }
            let rx = self.release.lock().await.take();
            if let Some(rx) = rx {
                let _ = rx.await;
            }
            self.events.lock().unwrap().push("write-done");
            Ok(())
        }
    }

    #[tokio::test]
    async fn stop_waits_until_autosave_write_completes() {
        let mgr = Arc::new(SessionManager::new());
        mgr.start_session(SessionConfig::default()).await.unwrap();
        mgr.process_reading(power(200, 1000)).await;

        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = Arc::new(GatedWriter {
            entered: std::sync::Mutex::new(Some(entered_tx)),
            release: tokio::sync::Mutex::new(Some(release_rx)),
            events: events.clone(),
        });

        let tick = tokio::spawn({
            let (mgr, writer) = (mgr.clone(), writer.clone());
            async move {
                let mut acc = AutosaveAccumulator::default();
                autosave_tick_with(&mgr, &*writer, &mut acc).await
            }
        });
        entered_rx.await.unwrap(); // tick is now blocked inside the write

        let stop = tokio::spawn({
            let (mgr, events) = (mgr.clone(), events.clone());
            async move {
                let r = mgr.stop_session_with_log().await;
                events.lock().unwrap().push("stop-done");
                r
            }
        });
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        assert!(!stop.is_finished(), "stop must not complete while a write is in flight");

        release_tx.send(()).unwrap();
        assert!(tick.await.unwrap());
        assert!(stop.await.unwrap().is_some());
        assert_eq!(*events.lock().unwrap(), vec!["write-done", "stop-done"]);
    }
}
