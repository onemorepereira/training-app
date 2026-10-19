//! Unified device manager.
//!
//! Locking (design ADR-1): the `DeviceManager` mutex guards bookkeeping only. Every
//! transport operation (scan, connect, disconnect, probe) runs through the
//! associated `*_shared` functions as *prepare (lock) → I/O (no lock) → commit (lock)*.
//! A per-device gate serialises operations on one device; the DM lock is never held
//! while awaiting a gate or transport I/O. It is never held while awaiting
//! `spawn_blocking` either, except for trainer commands, whose blocking task never
//! touches `AntSlot` or the DM lock.

use log::{info, warn};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::{broadcast, Mutex, Semaphore};
use tokio::time::Instant;
use tokio::task::JoinHandle;

use super::dedup::compute_device_groups;
use super::fec::FecController;
use super::io::*;
use super::primary::{DeviceCaps, Metric, PrimaryState};
use super::reconnect::ReconnectManager;
use super::types::*;
use crate::config;
use crate::error::{AntError, AppError};
use crate::session::storage::Storage;

/// Maximum concurrently running auto-reconnect attempts.
const RECONNECT_CONCURRENCY: usize = 2;

/// An ANT+ device silent this long in background search is given up (ADR-2c).
const ANT_GIVE_UP: Duration = Duration::from_secs(600);

/// How long a manual connect to a Reconnecting ANT+ device waits for data (ADR-2b).
const ANT_REJOIN_WAIT: Duration = Duration::from_secs(5);

/// Shared handle used by commands and background tasks.
pub type SharedDeviceManager<Io = RealIo> = Arc<Mutex<DeviceManager<Io>>>;

/// Lifecycle phase of a device the user has connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Delivering data.
    Connected,
    /// ANT+: no data, channel still open in background search (ADR-2).
    Stale { since: Instant, ticks: u32 },
    /// Link lost; transport teardown not yet performed. The gate holder runs it (NB1).
    LostPendingTeardown { give_up: bool },
    /// Transport torn down; waiting for auto-reconnect.
    TornDown,
}

struct DeviceRecord {
    info: DeviceInfo,
    phase: Phase,
}

impl DeviceRecord {
    fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
        self.info.status = match phase {
            Phase::Connected => ConnectionStatus::Connected,
            _ => ConnectionStatus::Reconnecting,
        };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectOrigin {
    Manual,
    AutoReconnect,
}

/// Outcome of `connect_shared` that is not an error.
#[derive(Debug, Clone)]
pub enum ConnectOutcome {
    Connected(DeviceInfo),
    /// Already connected: no-op success.
    AlreadyConnected(DeviceInfo),
    /// ANT+ device still in background search after a manual connect (not an error).
    StillReconnecting(DeviceInfo),
    /// Auto-reconnect skipped (device busy or no longer a target).
    Skipped,
}

/// What a watchdog tick observed; `lib.rs` turns it into frontend events.
#[derive(Debug, Default)]
pub struct WatchdogReport {
    /// `device_disconnected` payloads.
    pub disconnected: Vec<String>,
    /// `device_reconnected` payloads.
    pub reconnected: Vec<String>,
    /// `device_reconnecting` payloads (info, attempt).
    pub reconnecting: Vec<(DeviceInfo, u32)>,
    /// A device's displayed state is final and the list should be re-sent. Not set
    /// while a teardown is still pending (the list would show a stale status).
    pub list_dirty: bool,
}

impl WatchdogReport {
    pub fn list_changed(&self) -> bool {
        self.list_dirty
    }

    fn absorb(&mut self, other: WatchdogReport) {
        self.disconnected.extend(other.disconnected);
        self.reconnected.extend(other.reconnected);
        self.reconnecting.extend(other.reconnecting);
        self.list_dirty |= other.list_dirty;
    }
}

/// Frontend event derived from a watchdog report (lib.rs emits these 1:1).
#[derive(Debug, Clone)]
pub enum DeviceEvent {
    /// `device_disconnected`
    Disconnected(String),
    /// `device_reconnected`
    Reconnected(String),
    /// `device_list_updated`
    ListUpdated(Vec<DeviceInfo>),
    /// `device_reconnecting`
    Reconnecting { device_id: String, device_type: String, attempt: u32 },
}

/// Unified device manager wrapping BLE and ANT+ transports.
pub struct DeviceManager<Io: DeviceIo = RealIo> {
    io: Arc<Io>,
    storage: Option<Arc<Storage>>,
    /// Devices the user connected (Connected or Reconnecting).
    records: HashMap<String, DeviceRecord>,
    listener_handles: HashMap<String, JoinHandle<()>>,
    trainer_backends: HashMap<String, TrainerBackend>,
    reconnect: ReconnectManager,
    /// Per-device operation gates (never awaited while holding the DM lock).
    gates: HashMap<String, Arc<Mutex<()>>>,
    /// Cancel tokens of in-flight connects, owned by the gate holder (NB2).
    in_flight: HashMap<String, CancelToken>,
    /// Bumped by every disconnect; a connect that started before it gives up (so a
    /// connect queued on the gate behind a disconnect never reconnects the device).
    disconnect_epoch: HashMap<String, u64>,
    /// Devices with a spawned reconnect attempt not yet finished.
    attempts_spawned: HashSet<String>,
    reconnect_permits: Arc<Semaphore>,
    /// ANT+ give-up delay (ADR-2c); a field so tests can shorten it.
    ant_give_up: Duration,
    /// Results of spawned background tasks, drained by the next watchdog tick.
    pending_report: Arc<StdMutex<WatchdogReport>>,
    /// Shared primary-device map; listeners check this before sending readings.
    primary_devices: PrimariesHandle,
}

impl DeviceManager<RealIo> {
    pub fn new() -> Self {
        Self::with_io(RealIo::new())
    }
}

impl<Io: DeviceIo> DeviceManager<Io> {
    pub fn with_io(io: Io) -> Self {
        Self {
            io: Arc::new(io),
            storage: None,
            records: HashMap::new(),
            listener_handles: HashMap::new(),
            trainer_backends: HashMap::new(),
            reconnect: ReconnectManager::new(),
            gates: HashMap::new(),
            in_flight: HashMap::new(),
            disconnect_epoch: HashMap::new(),
            attempts_spawned: HashSet::new(),
            reconnect_permits: Arc::new(Semaphore::new(RECONNECT_CONCURRENCY)),
            ant_give_up: ANT_GIVE_UP,
            pending_report: Arc::new(StdMutex::new(WatchdogReport::default())),
            primary_devices: PrimaryState::shared(),
        }
    }

    pub fn set_storage(&mut self, storage: Arc<Storage>) {
        self.storage = Some(storage);
    }

    /// Returns a clone of the shared primary-device Arc for external consumers.
    pub fn primaries_handle(&self) -> PrimariesHandle {
        self.primary_devices.clone()
    }

    #[cfg(test)]
    pub fn io(&self) -> Arc<Io> {
        self.io.clone()
    }

    fn primary_state(&self) -> std::sync::RwLockWriteGuard<'_, PrimaryState> {
        self.primary_devices.write().unwrap_or_else(|e| e.into_inner())
    }

    /// Device is (again) delivering: real capabilities; the trainer primary is
    /// claimed at connect (other metrics are claimed lazily on first reading).
    fn on_connected(&self, device_type: DeviceType, device_id: &str) {
        let mut p = self.primary_state();
        p.register(device_id, DeviceCaps::for_device(device_id, device_type));
        if device_type == DeviceType::FitnessTrainer {
            p.claim_if_free(Metric::Trainer, device_id);
        }
    }

    /// Stale or lost: free primary slots so a backup can take over.
    fn release_claims(&self, device_id: &str) {
        self.primary_state().release_claims(device_id);
    }

    /// Disconnected: capabilities first, then claims (ADR-3a).
    fn unregister_device(&self, device_id: &str) {
        self.primary_state().unregister(device_id);
    }

    fn gate(&mut self, device_id: &str) -> Arc<Mutex<()>> {
        self.gates
            .entry(device_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Live view of a device: records, plus in-flight connects shown as Connecting.
    fn live_overlay(&self, devices: &mut HashMap<String, DeviceInfo>) {
        for id in self.in_flight.keys() {
            if !self.records.contains_key(id) {
                if let Some(info) = devices.get_mut(id) {
                    info.status = ConnectionStatus::Connecting;
                }
            }
        }
        for (id, rec) in &self.records {
            let mut info = rec.info.clone();
            info.in_range = true;
            devices.insert(id.clone(), info);
        }
    }

    /// Return known devices from storage, overlaid with current connection state.
    pub async fn list_current(&self) -> Vec<DeviceInfo> {
        let mut devices: HashMap<String, DeviceInfo> = HashMap::new();
        if let Some(ref storage) = self.storage {
            if let Ok(known) = storage.list_known_devices().await {
                for d in known {
                    devices.insert(d.id.clone(), d);
                }
            }
        }
        self.live_overlay(&mut devices);
        self.annotate_ant_metadata(&mut devices);
        apply_groups(&mut devices);
        devices.into_values().collect()
    }

    /// Merge scan results with known devices and live state.
    /// Live connection status always wins over a scan result for the same id;
    /// the scan only contributes `in_range` (and fresh fields for idle devices).
    async fn merge_scan(&self, found: Vec<DeviceInfo>) -> Vec<DeviceInfo> {
        let mut discovered: HashMap<String, DeviceInfo> = HashMap::new();
        if let Some(ref storage) = self.storage {
            if let Ok(known) = storage.list_known_devices().await {
                for d in known {
                    discovered.insert(d.id.clone(), d);
                }
            }
        }
        let scan_found: HashSet<String> = found.iter().map(|d| d.id.clone()).collect();
        for d in found {
            discovered.insert(d.id.clone(), d);
        }
        for info in discovered.values_mut() {
            info.in_range = scan_found.contains(&info.id);
        }
        self.live_overlay(&mut discovered);
        self.annotate_ant_metadata(&mut discovered);
        apply_groups(&mut discovered);

        let result: Vec<DeviceInfo> = discovered.into_values().collect();
        if let Some(ref storage) = self.storage {
            if let Err(e) = storage.upsert_known_devices_batch(&result).await {
                warn!("Failed to batch-persist devices: {}", e);
            }
        }
        result
    }

    /// Scan all transports without holding the DM lock during the scan.
    pub async fn scan_shared(dm: &SharedDeviceManager<Io>) -> Result<Vec<DeviceInfo>, AppError> {
        let io = dm.lock().await.io.clone();
        let found = io.scan().await;
        let g = dm.lock().await;
        Ok(g.merge_scan(found).await)
    }

    /// Manual connect (command path). Already-connected devices are a no-op success.
    pub async fn connect_shared(
        dm: &SharedDeviceManager<Io>,
        device_id: &str,
        tx: broadcast::Sender<SensorReading>,
    ) -> Result<DeviceInfo, AppError> {
        match Self::connect_with_origin(dm, device_id, tx, ConnectOrigin::Manual).await? {
            ConnectOutcome::Connected(i)
            | ConnectOutcome::AlreadyConnected(i)
            | ConnectOutcome::StillReconnecting(i) => Ok(i),
            ConnectOutcome::Skipped => Err(AppError::Cancelled),
        }
    }

    pub async fn connect_with_origin(
        dm: &SharedDeviceManager<Io>,
        device_id: &str,
        tx: broadcast::Sender<SensorReading>,
        origin: ConnectOrigin,
    ) -> Result<ConnectOutcome, AppError> {
        // Phase 1 (lock): fast no-op path, manual connect clears auto-reconnect.
        let (gate, io, primaries, epoch) = {
            let mut g = dm.lock().await;
            if let Some(rec) = g.records.get(device_id) {
                if rec.phase == Phase::Connected {
                    return Ok(ConnectOutcome::AlreadyConnected(rec.info.clone()));
                }
            }
            if origin == ConnectOrigin::Manual {
                g.reconnect.remove(device_id);
            }
            let epoch = g.disconnect_epoch.get(device_id).copied().unwrap_or(0);
            (g.gate(device_id), g.io.clone(), g.primary_devices.clone(), epoch)
        };

        // Phase 2: own this device. Auto attempts never queue behind a busy device.
        let _gate = match origin {
            ConnectOrigin::Manual => gate.lock_owned().await,
            ConnectOrigin::AutoReconnect => match gate.try_lock_owned() {
                Ok(g) => g,
                Err(_) => return Ok(ConnectOutcome::Skipped),
            },
        };

        // NB1: the gate holder performs any teardown still pending for a Lost record.
        Self::run_pending_teardown(dm, device_id).await;

        // Phase 3 (lock): re-check, then register the in-flight token (NB2: post-gate).
        let (token, stale) = {
            let mut g = dm.lock().await;
            if g.disconnect_epoch.get(device_id).copied().unwrap_or(0) != epoch {
                // A disconnect was requested after this connect started.
                return Err(AppError::Cancelled);
            }
            if let Some(rec) = g.records.get(device_id) {
                if rec.phase == Phase::Connected {
                    return Ok(ConnectOutcome::AlreadyConnected(rec.info.clone()));
                }
            }
            if origin == ConnectOrigin::AutoReconnect {
                if !g.reconnect.contains(device_id) {
                    return Ok(ConnectOutcome::Skipped);
                }
                g.reconnect.record_attempt(device_id);
            }
            let stale = g
                .records
                .get(device_id)
                .is_some_and(|r| matches!(r.phase, Phase::Stale { .. }));
            let token = CancelToken::new();
            g.in_flight.insert(device_id.to_string(), token.clone());
            // NB2: registered by the gate holder, before any listener can run.
            if !g.primary_state().is_registered(device_id) {
                g.primary_state().register(device_id, DeviceCaps::pending());
            }
            (token, stale)
        };

        // ADR-2b: the channel is still open and searching. Never open a second one;
        // wait briefly for the device to show up on it.
        if stale {
            let got_data = io.await_data(device_id, ANT_REJOIN_WAIT, &token).await;
            let mut g = dm.lock().await;
            if g.in_flight.get(device_id).is_some_and(|t| t.same_as(&token)) {
                g.in_flight.remove(device_id);
            }
            if token.is_cancelled() {
                return Err(AppError::Cancelled);
            }
            let Some(rec) = g.records.get_mut(device_id) else {
                return Err(AppError::Cancelled);
            };
            if got_data && matches!(rec.phase, Phase::Stale { .. }) {
                rec.set_phase(Phase::Connected);
                let info = rec.info.clone();
                g.on_connected(info.device_type, device_id);
                {
                    let mut r = g.pending_report.lock().unwrap_or_else(|e| e.into_inner());
                    r.reconnected.push(device_id.to_string());
                    r.list_dirty = true;
                }
                return Ok(ConnectOutcome::Connected(info));
            }
            return Ok(ConnectOutcome::StillReconnecting(rec.info.clone()));
        }

        // Phase 4: transport I/O without the DM lock.
        let ctx = ListenerCtx { tx, primaries };
        let result = io.connect(device_id, ctx, token.clone()).await;

        // Phase 5 (lock): commit, unless cancelled.
        let mut g = dm.lock().await;
        let mine = g.in_flight.get(device_id).is_some_and(|t| t.same_as(&token));
        if mine {
            g.in_flight.remove(device_id);
        }
        // Only the owning attempt may drop the registration it made (NB2).
        let undo_registration = |g: &DeviceManager<Io>| {
            if mine && !g.records.contains_key(device_id) {
                g.unregister_device(device_id);
            }
        };
        if token.is_cancelled() {
            undo_registration(&g);
            drop(g);
            // NB3: undo a connect that completed after the cancel, before releasing the gate.
            if let Ok(c) = result {
                if let Some(h) = c.listener {
                    h.abort();
                }
                if let Err(e) = io.disconnect(device_id).await {
                    warn!("[{}] Disconnect after cancelled connect failed: {}", device_id, e);
                }
            }
            return Err(AppError::Cancelled);
        }

        match result {
            Err(e) => {
                if origin == ConnectOrigin::Manual {
                    // A failed manual connect of a Reconnecting device leaves it
                    // Reconnecting and keeps auto-reconnect going.
                    let torn_down = g
                        .records
                        .get(device_id)
                        .filter(|r| r.phase == Phase::TornDown)
                        .map(|r| r.info.clone());
                    if let Some(info) = torn_down {
                        g.reconnect.register(info);
                    }
                }
                undo_registration(&g);
                Err(e)
            }
            Ok(c) => {
                let info = g.commit_connect(device_id, c);
                if origin == ConnectOrigin::AutoReconnect {
                    g.reconnect.remove(device_id);
                }
                Ok(ConnectOutcome::Connected(info))
            }
        }
    }

    fn commit_connect(&mut self, device_id: &str, c: Connected) -> DeviceInfo {
        let mut info = c.info;
        info.status = ConnectionStatus::Connected;
        if let Some(old) = self.listener_handles.remove(device_id) {
            old.abort();
        }
        if let Some(h) = c.listener {
            self.listener_handles.insert(device_id.to_string(), h);
        }
        match c.trainer {
            Some(t) => {
                self.trainer_backends.insert(device_id.to_string(), t);
            }
            None => {
                self.trainer_backends.remove(device_id);
            }
        }
        info!(
            "[{}] Connected via {:?}: {:?} ({})",
            device_id,
            info.transport,
            info.device_type,
            info.name.as_deref().unwrap_or("unnamed")
        );
        self.records.insert(
            device_id.to_string(),
            DeviceRecord { info: info.clone(), phase: Phase::Connected },
        );
        self.on_connected(info.device_type, device_id);
        info
    }

    /// Disconnect a device: cancels an in-flight connect, stops auto-reconnect,
    /// releases primaries, then tears down the transport outside the DM lock.
    pub async fn disconnect_shared(dm: &SharedDeviceManager<Io>, device_id: &str) -> Result<(), AppError> {
        let gate = {
            let mut g = dm.lock().await;
            if let Some(t) = g.in_flight.get(device_id) {
                t.cancel();
            }
            *g.disconnect_epoch.entry(device_id.to_string()).or_insert(0) += 1;
            g.reconnect.remove(device_id);
            g.unregister_device(device_id);
            g.gate(device_id)
        };
        let _gate = gate.lock_owned().await;
        let io = {
            let mut g = dm.lock().await;
            g.records.remove(device_id);
            // A watchdog Lost transition may have re-registered it while we waited.
            g.reconnect.remove(device_id);
            g.unregister_device(device_id);
            if let Some(h) = g.listener_handles.remove(device_id) {
                h.abort();
            }
            g.trainer_backends.remove(device_id);
            g.io.clone()
        };
        io.disconnect(device_id).await?;
        info!("[{}] Disconnected", device_id);
        Ok(())
    }

    /// Must be called while holding the device's gate. Performs a teardown that a
    /// watchdog Lost transition left pending; no-op otherwise.
    async fn run_pending_teardown(dm: &SharedDeviceManager<Io>, device_id: &str) {
        let (io, give_up, report) = {
            let mut g = dm.lock().await;
            let Some(rec) = g.records.get_mut(device_id) else {
                return;
            };
            let Phase::LostPendingTeardown { give_up } = rec.phase else {
                return;
            };
            if give_up {
                g.records.remove(device_id);
                g.unregister_device(device_id);
            } else {
                rec.set_phase(Phase::TornDown);
            }
            if let Some(h) = g.listener_handles.remove(device_id) {
                h.abort();
            }
            g.trainer_backends.remove(device_id);
            (g.io.clone(), give_up, g.pending_report.clone())
        };
        if let Err(e) = io.disconnect(device_id).await {
            warn!("[{}] Teardown after link loss failed: {}", device_id, e);
        }
        if give_up {
            // Only now is "Disconnected" the true state; re-send the list next tick.
            report.lock().unwrap_or_else(|e| e.into_inner()).list_dirty = true;
        }
    }

    /// Turn a watchdog report into the frontend events, in emit order. The list is
    /// built after the transitions it reflects, so it never shows a stale status.
    pub async fn report_events(dm: &SharedDeviceManager<Io>, report: WatchdogReport) -> Vec<DeviceEvent> {
        let mut events: Vec<DeviceEvent> = report.disconnected.iter().cloned().map(DeviceEvent::Disconnected).collect();
        events.extend(report.reconnected.iter().cloned().map(DeviceEvent::Reconnected));
        if report.list_changed() {
            events.push(DeviceEvent::ListUpdated(dm.lock().await.list_current().await));
        }
        events.extend(report.reconnecting.into_iter().map(|(info, attempt)| DeviceEvent::Reconnecting {
            device_id: info.id,
            device_type: info.device_type.as_str().to_string(),
            attempt,
        }));
        events
    }

    /// Spawned teardown for a Lost device. Takes the gate; if a connect or
    /// disconnect got there first it already did the work and this is a no-op.
    async fn teardown_task(dm: SharedDeviceManager<Io>, device_id: String) {
        let gate = dm.lock().await.gate(&device_id);
        let _g = gate.lock_owned().await;
        Self::run_pending_teardown(&dm, &device_id).await;
    }

    /// Spawned auto-reconnect attempt (bounded concurrency).
    async fn reconnect_task(
        dm: SharedDeviceManager<Io>,
        info: DeviceInfo,
        tx: broadcast::Sender<SensorReading>,
    ) {
        let (permits, report) = {
            let g = dm.lock().await;
            (g.reconnect_permits.clone(), g.pending_report.clone())
        };
        let outcome = match permits.acquire_owned().await {
            Ok(_permit) => {
                Self::connect_with_origin(&dm, &info.id, tx, ConnectOrigin::AutoReconnect).await
            }
            Err(_) => Ok(ConnectOutcome::Skipped),
        };
        let mut g = dm.lock().await;
        g.attempts_spawned.remove(&info.id);
        let attempt = g.reconnect.attempt_count(&info.id);
        let mut r = report.lock().unwrap_or_else(|e| e.into_inner());
        match outcome {
            Ok(ConnectOutcome::Connected(new_info)) => {
                info!("[{}] Reconnected", new_info.id);
                r.reconnected.push(new_info.id);
                r.list_dirty = true;
            }
            Ok(_) => {}
            Err(e) => {
                log::debug!("[{}] Reconnect attempt {} failed: {}", info.id, attempt, e);
                r.reconnecting.push((info, attempt));
            }
        }
    }

    /// One watchdog pass: probe links, apply transitions, spawn teardowns and due
    /// reconnects. Never awaits transport work while holding the DM lock, and never
    /// waits for a reconnect attempt to finish.
    pub async fn watchdog_tick(
        dm: &SharedDeviceManager<Io>,
        tx: &broadcast::Sender<SensorReading>,
    ) -> WatchdogReport {
        let (io, ids) = {
            let g = dm.lock().await;
            let ids: Vec<String> = g
                .records
                .iter()
                .filter(|(_, r)| matches!(r.phase, Phase::Connected | Phase::Stale { .. }))
                .map(|(id, _)| id.clone())
                .collect();
            (g.io.clone(), ids)
        };
        let states = io.probe(ids).await;

        let mut report = WatchdogReport::default();
        let (teardowns, due) = {
            let mut g = dm.lock().await;
            let now = Instant::now();
            let give_up_after = g.ant_give_up;
            let mut teardowns = Vec::new();
            for (id, state) in states {
                let Some(rec) = g.records.get_mut(&id) else { continue };
                match (rec.phase, state) {
                    (Phase::Connected, LinkState::Alive) => {}
                    (Phase::Connected, LinkState::Stale) => {
                        // ADR-2: keep the channel searching; only the status changes.
                        warn!("[{}] Connection watchdog: no ANT+ data, searching", id);
                        rec.set_phase(Phase::Stale { since: now, ticks: 0 });
                        g.release_claims(&id);
                        report.disconnected.push(id);
                        report.list_dirty = true;
                    }
                    (Phase::Stale { .. }, LinkState::Alive) => {
                        info!("[{}] ANT+ data resumed on the same channel", id);
                        rec.set_phase(Phase::Connected);
                        let device_type = rec.info.device_type;
                        g.on_connected(device_type, &id);
                        report.reconnected.push(id);
                        report.list_dirty = true;
                    }
                    (Phase::Stale { since, ticks }, LinkState::Stale) => {
                        if now.duration_since(since) >= give_up_after {
                            warn!("[{}] ANT+ device silent for {:?}; giving up", id, give_up_after);
                            rec.set_phase(Phase::LostPendingTeardown { give_up: true });
                            g.trainer_backends.remove(&id);
                            report.disconnected.push(id.clone());
                            teardowns.push(id);
                        } else {
                            rec.set_phase(Phase::Stale { since, ticks: ticks + 1 });
                            report.reconnecting.push((rec.info.clone(), ticks + 1));
                        }
                    }
                    (Phase::Connected | Phase::Stale { .. }, LinkState::Lost) => {
                        warn!("[{}] Connection watchdog: {:?} link lost", id, rec.info.device_type);
                        rec.set_phase(Phase::LostPendingTeardown { give_up: false });
                        let info = rec.info.clone();
                        g.release_claims(&id);
                        g.trainer_backends.remove(&id);
                        g.reconnect.register(info);
                        if !report.disconnected.contains(&id) {
                            report.disconnected.push(id.clone());
                        }
                        report.list_dirty = true;
                        teardowns.push(id);
                    }
                    _ => {}
                }
            }
            let due: Vec<DeviceInfo> = g
                .reconnect
                .peek_due()
                .into_iter()
                .filter(|i| !g.attempts_spawned.contains(&i.id))
                .collect();
            for i in &due {
                g.attempts_spawned.insert(i.id.clone());
            }
            let pending = std::mem::take(&mut *g.pending_report.lock().unwrap_or_else(|e| e.into_inner()));
            report.absorb(pending);
            (teardowns, due)
        };

        for id in teardowns {
            tokio::spawn(Self::teardown_task(dm.clone(), id));
        }
        for info in due {
            tokio::spawn(Self::reconnect_task(dm.clone(), info, tx.clone()));
        }
        report
    }

    /// Device details without holding the DM lock across BLE GATT reads.
    pub async fn device_details_shared(
        dm: &SharedDeviceManager<Io>,
        device_id: &str,
    ) -> Result<DeviceDetails, AppError> {
        let io = {
            let g = dm.lock().await;
            if device_id.starts_with("ant:") {
                return g.ant_device_details(device_id);
            }
            g.io.clone()
        };
        io.ble_details(device_id).await
    }

    fn ant_device_details(&self, device_id: &str) -> Result<DeviceDetails, AppError> {
        let info = &self
            .records
            .get(device_id)
            .ok_or_else(|| AppError::DeviceNotFound(device_id.to_string()))?
            .info;
        let meta = self.io.ant_metadata().remove(device_id);
        let (manufacturer, model_number, serial_number, hw_revision, sw_revision, battery_level) =
            if let Some(m) = meta {
                (
                    m.manufacturer_id.map(ant_manufacturer_name),
                    m.model_number.map(|n| n.to_string()),
                    m.serial_number.map(|n| n.to_string()),
                    m.hw_revision.map(|r| r.to_string()),
                    m.sw_revision.clone(),
                    m.battery_level.or(info.battery_level),
                )
            } else {
                (None, None, None, None, None, info.battery_level)
            };
        Ok(DeviceDetails {
            id: info.id.clone(),
            name: info.name.clone(),
            device_type: info.device_type,
            transport: Transport::AntPlus,
            rssi: info.rssi,
            battery_level,
            manufacturer,
            model_number,
            serial_number,
            firmware_revision: sw_revision,
            hardware_revision: hw_revision,
            software_revision: None,
            services: vec![],
        })
    }

    // Trainer control. FE-C sends run in spawn_blocking; their task touches only the
    // AntUsb handle (never AntSlot or the DM lock).

    pub async fn set_target_power(&mut self, device_id: &str, watts: i16) -> Result<(), AppError> {
        let watts = config::clamp_trainer_watts(watts as i32);
        let result = match self.trainer_backends.get_mut(device_id) {
            Some(TrainerBackend::Ftms(controller)) => controller.set_target_power(watts).await,
            Some(TrainerBackend::Fec { link, .. }) => {
                let link = link.clone();
                let w = watts.max(0) as u16;
                tokio::task::spawn_blocking(move || {
                    let fec = FecController::new(&link);
                    fec.set_target_power(w)
                })
                .await
                .map_err(|e| AppError::from(AntError::TaskPanicked(format!("FEC: {}", e))))?
            }
            #[cfg(test)]
            Some(TrainerBackend::Fake(count)) => {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
            None => Err(AppError::Session("No trainer connected".into())),
        };
        if let Err(ref e) = result {
            warn!("[{}] set_target_power({}W) failed: {}", device_id, watts, e);
        }
        result
    }

    pub async fn set_resistance(&mut self, device_id: &str, level: u8) -> Result<(), AppError> {
        let result = match self.trainer_backends.get_mut(device_id) {
            Some(TrainerBackend::Ftms(controller)) => controller.set_resistance(level).await,
            Some(TrainerBackend::Fec { link, .. }) => {
                let link = link.clone();
                tokio::task::spawn_blocking(move || {
                    let fec = FecController::new(&link);
                    fec.set_resistance(level)
                })
                .await
                .map_err(|e| AppError::from(AntError::TaskPanicked(format!("FEC: {}", e))))?
            }
            #[cfg(test)]
            Some(TrainerBackend::Fake(count)) => {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
            None => Err(AppError::Session("No trainer connected".into())),
        };
        if let Err(ref e) = result {
            warn!("[{}] set_resistance({}) failed: {}", device_id, level, e);
        }
        result
    }

    pub async fn set_simulation(
        &mut self,
        device_id: &str,
        grade: f32,
        crr: f32,
        cw: f32,
    ) -> Result<(), AppError> {
        let result = match self.trainer_backends.get_mut(device_id) {
            Some(TrainerBackend::Ftms(controller)) => controller.set_simulation(grade, crr, cw).await,
            Some(TrainerBackend::Fec { link, last_cw_raw }) => {
                let link = link.clone();
                let last = *last_cw_raw;
                let sent = tokio::task::spawn_blocking(move || {
                    let fec = FecController::new(&link);
                    fec.set_simulation(grade, crr, cw, last)
                })
                .await
                .map_err(|e| AppError::from(AntError::TaskPanicked(format!("FEC: {}", e))))?;
                sent.map(|raw| *last_cw_raw = Some(raw))
            }
            #[cfg(test)]
            Some(TrainerBackend::Fake(count)) => {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
            None => Err(AppError::Session("No trainer connected".into())),
        };
        if let Err(ref e) = result {
            warn!("[{}] set_simulation(grade={}, crr={}, cw={}) failed: {}", device_id, grade, crr, cw, e);
        }
        result
    }

    pub async fn start_trainer(&mut self, device_id: &str) -> Result<(), AppError> {
        match self.trainer_backends.get_mut(device_id) {
            Some(TrainerBackend::Ftms(controller)) => controller.start().await,
            Some(TrainerBackend::Fec { .. }) => {
                Err(AntError::NotSupported("start/stop for ANT+ trainers".into()).into())
            }
            #[cfg(test)]
            Some(TrainerBackend::Fake(_)) => Ok(()),
            None => Err(AppError::Session("No trainer connected".into())),
        }
    }

    pub async fn stop_trainer(&mut self, device_id: &str) -> Result<(), AppError> {
        match self.trainer_backends.get_mut(device_id) {
            Some(TrainerBackend::Ftms(controller)) => controller.stop().await,
            Some(TrainerBackend::Fec { .. }) => {
                Err(AntError::NotSupported("start/stop for ANT+ trainers".into()).into())
            }
            #[cfg(test)]
            Some(TrainerBackend::Fake(_)) => Ok(()),
            None => Err(AppError::Session("No trainer connected".into())),
        }
    }

    /// Annotate ANT+ devices with metadata from common data pages.
    fn annotate_ant_metadata(&self, devices: &mut HashMap<String, DeviceInfo>) {
        let meta = self.io.ant_metadata();
        for (id, info) in devices.iter_mut() {
            if !id.starts_with("ant:") {
                continue;
            }
            if let Some(m) = meta.get(id) {
                if info.manufacturer.is_none() {
                    info.manufacturer = m.manufacturer_id.map(ant_manufacturer_name);
                }
                if info.model_number.is_none() {
                    info.model_number = m.model_number.map(|n| n.to_string());
                }
                if info.serial_number.is_none() {
                    info.serial_number = m.serial_number.filter(|&s| s != 0).map(|n| n.to_string());
                }
            }
        }
    }

    /// Get the connected trainer device ID (for command routing).
    /// Only a trainer that is actually Connected is returned, avoiding stale
    /// entries left behind during reconnect.
    pub fn connected_trainer_id(&self) -> Option<String> {
        self.trainer_backends
            .keys()
            .find(|id| self.records.get(*id).is_some_and(|r| r.phase == Phase::Connected))
            .cloned()
    }
}

/// Compute cross-transport device groups in place.
fn apply_groups(devices: &mut HashMap<String, DeviceInfo>) {
    let device_list: Vec<DeviceInfo> = devices.values().cloned().collect();
    for (id, group_id) in compute_device_groups(&device_list) {
        if let Some(info) = devices.get_mut(&id) {
            info.device_group = Some(group_id);
        }
    }
}

/// Look up ANT+ manufacturer name from FIT SDK manufacturer ID registry.
/// Source: FIT Profile.xls 'Types' tab, 'manufacturer' field type.
pub fn ant_manufacturer_name(id: u16) -> String {
    match id {
        1 => "Garmin".into(),
        6 => "SRM".into(),
        7 => "Quarq".into(),
        8 => "iBike".into(),
        9 => "Saris".into(),
        15 => "Dynastream".into(),
        16 => "Timex".into(),
        17 => "MetriGear".into(),
        19 => "Beurer".into(),
        20 => "Cardiosport".into(),
        23 => "Suunto".into(),
        30 => "LeMond Fitness".into(),
        32 => "Wahoo Fitness".into(),
        40 => "Concept2".into(),
        41 => "Shimano".into(),
        44 => "Brim Brothers".into(),
        45 => "Xplova".into(),
        48 => "Pioneer".into(),
        49 => "Spantec".into(),
        50 => "Metalogics".into(),
        51 => "4iiii".into(),
        56 => "Star Trac".into(),
        60 => "Rotor".into(),
        61 => "Geonaute".into(),
        63 => "Specialized".into(),
        65 => "Physical Enterprises".into(),
        66 => "North Pole Engineering".into(),
        67 => "Bkool".into(),
        68 => "CatEye".into(),
        69 => "Stages Cycling".into(),
        70 => "Sigmasport".into(),
        71 => "TomTom".into(),
        72 => "Peripedal".into(),
        73 => "Wattbike".into(),
        76 => "Moxy".into(),
        77 => "Ciclosport".into(),
        78 => "Powerbahn".into(),
        80 => "Lifebeam".into(),
        81 => "Bontrager".into(),
        83 => "Scosche".into(),
        86 => "Elite".into(),
        89 => "Tacx".into(),
        93 => "Inside Ride".into(),
        95 => "Stryd".into(),
        96 => "ICG".into(),
        99 => "Look".into(),
        100 => "Campagnolo".into(),
        101 => "Body Bike Smart".into(),
        102 => "Praxisworks".into(),
        107 => "Magene".into(),
        108 => "Giant".into(),
        111 => "Technogym".into(),
        112 => "Bryton".into(),
        115 => "iGPSport".into(),
        116 => "ThinkRider".into(),
        118 => "WaterRower".into(),
        121 => "Kinetic".into(),
        122 => "Johnson Health Tech".into(),
        123 => "Polar".into(),
        128 => "iFit".into(),
        129 => "Coros".into(),
        132 => "Cycplus".into(),
        134 => "Sigeyi".into(),
        135 => "Coospo".into(),
        137 => "Bosch".into(),
        140 => "Decathlon".into(),
        143 => "Keiser".into(),
        255 => "Development".into(),
        258 => "Lezyne".into(),
        260 => "Zwift".into(),
        261 => "Watteam".into(),
        263 => "Favero".into(),
        266 => "Precor".into(),
        268 => "SRAM".into(),
        270 => "COBI".into(),
        278 => "Minoura".into(),
        281 => "TrainerRoad".into(),
        282 => "The Sufferfest".into(),
        283 => "FSA".into(),
        285 => "Feedback Sports".into(),
        287 => "VDO".into(),
        288 => "MagneticDays".into(),
        289 => "Hammerhead".into(),
        290 => "Kinetic by Kurt".into(),
        293 => "JetBlack".into(),
        294 => "Coros".into(),
        305 => "Whoop".into(),
        308 => "Monark Exercise".into(),
        311 => "Syncros".into(),
        313 => "Cannondale".into(),
        315 => "RGT Cycling".into(),
        327 => "Magicshine".into(),
        331 => "MyWhoosh".into(),
        _ => format!("Unknown ({})", id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manufacturer_garmin() {
        assert_eq!(ant_manufacturer_name(1), "Garmin");
    }

    #[test]
    fn manufacturer_wahoo() {
        assert_eq!(ant_manufacturer_name(32), "Wahoo Fitness");
    }

    #[test]
    fn manufacturer_tacx() {
        assert_eq!(ant_manufacturer_name(89), "Tacx");
    }

    #[test]
    fn manufacturer_unknown_id() {
        assert_eq!(ant_manufacturer_name(9999), "Unknown (9999)");
    }

    #[test]
    fn manufacturer_shimano() {
        assert_eq!(ant_manufacturer_name(41), "Shimano");
    }

    #[test]
    fn manufacturer_keiser() {
        assert_eq!(ant_manufacturer_name(143), "Keiser");
    }

    #[test]
    fn manufacturer_coospo() {
        assert_eq!(ant_manufacturer_name(135), "Coospo");
    }
}

#[cfg(test)]
mod lifecycle_tests {
    //! Device lifecycle and lock-scope tests against `FakeIo` with a paused clock.
    use super::*;
    use crate::config;
    use crate::device::io::fake::{FakeIo, IoEvent};
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use tokio::time::Instant;

    type Dm = SharedDeviceManager<FakeIo>;

    fn dm() -> Dm {
        Arc::new(Mutex::new(DeviceManager::with_io(FakeIo::default())))
    }

    fn tx() -> broadcast::Sender<SensorReading> {
        broadcast::channel(16).0
    }

    async fn io(dm: &Dm) -> Arc<FakeIo> {
        dm.lock().await.io()
    }

    /// Let spawned tasks (aborts, teardowns, attempts) run to quiescence.
    async fn settle() {
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
    }

    fn connects(events: &[IoEvent], id: &str) -> usize {
        events.iter().filter(|e| **e == IoEvent::Connect(id.to_string())).count()
    }

    async fn status(dm: &Dm, id: &str) -> Option<ConnectionStatus> {
        dm.lock().await.records.get(id).map(|r| r.info.status)
    }

    fn fake_info(id: &str) -> DeviceInfo {
        crate::device::io::fake::info(id, ConnectionStatus::Disconnected)
    }

    async fn lose_and_wait_for_retry(dm: &Dm, id: &str, tx: &broadcast::Sender<SensorReading>) {
        io(dm).await.set_link(id, LinkState::Lost);
        DeviceManager::watchdog_tick(dm, tx).await;
        settle().await;
        tokio::time::advance(Duration::from_millis(config::RECONNECT_INITIAL_BACKOFF_MS + 1)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn connect_twice_spawns_one_listener() {
        let dm = dm();
        DeviceManager::connect_shared(&dm, "ble-hr", tx()).await.unwrap();
        let again = DeviceManager::connect_shared(&dm, "ble-hr", tx()).await.unwrap();
        settle().await;
        let io = io(&dm).await;
        assert_eq!(again.status, ConnectionStatus::Connected);
        assert_eq!(connects(&io.events(), "ble-hr"), 1, "second connect must be a no-op");
        assert_eq!(io.listeners_alive.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_connects_same_device_one_io_connect() {
        let dm = dm();
        io(&dm).await.set_connect_delay("ble-hr", Duration::from_secs(2));
        let (a, b) = tokio::join!(
            DeviceManager::connect_shared(&dm, "ble-hr", tx()),
            DeviceManager::connect_shared(&dm, "ble-hr", tx()),
        );
        a.unwrap();
        b.unwrap();
        settle().await;
        let io = io(&dm).await;
        assert_eq!(connects(&io.events(), "ble-hr"), 1);
        assert_eq!(io.listeners_alive.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn manual_connect_clears_pending_reconnect() {
        let dm = dm();
        dm.lock().await.reconnect.register(fake_info("ble-hr"));
        DeviceManager::connect_shared(&dm, "ble-hr", tx()).await.unwrap();
        assert!(!dm.lock().await.reconnect.contains("ble-hr"));
    }

    #[tokio::test(start_paused = true)]
    async fn commit_aborts_existing_listener_handle() {
        let dm = dm();
        DeviceManager::connect_shared(&dm, "ble-hr", tx()).await.unwrap();
        // A second transport result reaching commit for the same id.
        let io = io(&dm).await;
        let ctx = ListenerCtx { tx: tx(), primaries: dm.lock().await.primaries_handle() };
        let c = io.connect("ble-hr", ctx, CancelToken::new()).await.unwrap();
        dm.lock().await.commit_connect("ble-hr", c);
        settle().await;
        assert_eq!(io.listeners_alive.load(Ordering::SeqCst), 1, "old listener must be aborted");
    }

    #[tokio::test(start_paused = true)]
    async fn trainer_command_during_scan_completes_within_500ms() {
        let dm = dm();
        DeviceManager::connect_shared(&dm, "ble-trainer", tx()).await.unwrap();
        *io(&dm).await.scan_delay.lock().unwrap() = Duration::from_secs(4);
        let scan = tokio::spawn({
            let dm = dm.clone();
            async move { DeviceManager::scan_shared(&dm).await }
        });
        settle().await; // the scan is now sleeping inside io.scan()
        let start = Instant::now();
        {
            let mut g = dm.lock().await;
            let id = g.connected_trainer_id().unwrap();
            g.set_target_power(&id, 200).await.unwrap();
        }
        assert!(start.elapsed() < Duration::from_millis(500), "took {:?}", start.elapsed());
        assert_eq!(io(&dm).await.trainer_commands.load(Ordering::SeqCst), 1);
        scan.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn trainer_command_during_reconnect_completes_within_500ms() {
        let dm = dm();
        let tx = tx();
        DeviceManager::connect_shared(&dm, "ble-trainer", tx.clone()).await.unwrap();
        DeviceManager::connect_shared(&dm, "ble-hr", tx.clone()).await.unwrap();
        let io = io(&dm).await;
        lose_and_wait_for_retry(&dm, "ble-hr", &tx).await;
        io.set_connect_delay("ble-hr", Duration::from_secs(4));
        DeviceManager::watchdog_tick(&dm, &tx).await; // spawns the slow attempt
        settle().await;
        assert_eq!(connects(&io.events(), "ble-hr"), 2, "reconnect attempt in flight");
        let start = Instant::now();
        {
            let mut g = dm.lock().await;
            let id = g.connected_trainer_id().unwrap();
            g.set_target_power(&id, 150).await.unwrap();
        }
        assert!(start.elapsed() < Duration::from_millis(500), "took {:?}", start.elapsed());
    }

    #[tokio::test(start_paused = true)]
    async fn scan_merge_keeps_connecting_status() {
        let dm = dm();
        let io = io(&dm).await;
        io.set_connect_delay("ble-hr", Duration::from_secs(3));
        *io.scan_result.lock().unwrap() = vec![fake_info("ble-hr")];
        let connecting = tokio::spawn({
            let dm = dm.clone();
            async move { DeviceManager::connect_shared(&dm, "ble-hr", tx()).await }
        });
        settle().await;
        let list = DeviceManager::scan_shared(&dm).await.unwrap();
        let hr = list.iter().find(|d| d.id == "ble-hr").unwrap();
        assert_eq!(hr.status, ConnectionStatus::Connecting);
        connecting.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn scan_merge_keeps_reconnecting_status() {
        let dm = dm();
        let tx = tx();
        DeviceManager::connect_shared(&dm, "ble-hr", tx.clone()).await.unwrap();
        let io = io(&dm).await;
        io.set_link("ble-hr", LinkState::Lost);
        DeviceManager::watchdog_tick(&dm, &tx).await;
        *io.scan_result.lock().unwrap() = vec![fake_info("ble-hr")];
        let list = DeviceManager::scan_shared(&dm).await.unwrap();
        let hr = list.iter().find(|d| d.id == "ble-hr").unwrap();
        assert_eq!(hr.status, ConnectionStatus::Reconnecting);
    }

    #[tokio::test(start_paused = true)]
    async fn scan_merge_updates_in_range_only() {
        let dm = dm();
        DeviceManager::connect_shared(&dm, "ble-hr", tx()).await.unwrap();
        // The scan does not see the connected strap, but sees another device.
        *io(&dm).await.scan_result.lock().unwrap() = vec![fake_info("ble-pm")];
        let list = DeviceManager::scan_shared(&dm).await.unwrap();
        let hr = list.iter().find(|d| d.id == "ble-hr").unwrap();
        assert_eq!(hr.status, ConnectionStatus::Connected);
        assert!(hr.in_range, "connected devices are always in range");
        let pm = list.iter().find(|d| d.id == "ble-pm").unwrap();
        assert_eq!(pm.status, ConnectionStatus::Disconnected);
        assert!(pm.in_range);
    }

    #[tokio::test(start_paused = true)]
    async fn disconnect_cancels_in_flight_connect_promptly() {
        let dm = dm();
        io(&dm).await.set_connect_delay("ble-hr", Duration::from_secs(9));
        let connecting = tokio::spawn({
            let dm = dm.clone();
            async move { DeviceManager::connect_shared(&dm, "ble-hr", tx()).await }
        });
        settle().await;
        let start = Instant::now();
        DeviceManager::disconnect_shared(&dm, "ble-hr").await.unwrap();
        assert!(start.elapsed() < Duration::from_millis(100), "took {:?}", start.elapsed());
        assert!(matches!(connecting.await.unwrap(), Err(AppError::Cancelled)));
        assert_eq!(status(&dm, "ble-hr").await, None);
    }

    #[tokio::test(start_paused = true)]
    async fn cancelled_connect_does_not_commit_listener() {
        let dm = dm();
        let io = io(&dm).await;
        io.set_connect_delay("ble-hr", Duration::from_secs(2));
        io.ignore_cancel.lock().unwrap().insert("ble-hr".into());
        let connecting = tokio::spawn({
            let dm = dm.clone();
            async move { DeviceManager::connect_shared(&dm, "ble-hr", tx()).await }
        });
        settle().await;
        DeviceManager::disconnect_shared(&dm, "ble-hr").await.unwrap();
        assert!(matches!(connecting.await.unwrap(), Err(AppError::Cancelled)));
        settle().await;
        let events = io.events();
        let connect_at = events.iter().position(|e| *e == IoEvent::Connect("ble-hr".into())).unwrap();
        assert!(
            events[connect_at..].contains(&IoEvent::Disconnect("ble-hr".into())),
            "a connect that completed after cancel must be undone: {:?}",
            events
        );
        assert_eq!(io.listeners_alive.load(Ordering::SeqCst), 0);
        assert_eq!(status(&dm, "ble-hr").await, None);
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_connects_disconnect_cancels_owner_attempt() {
        let dm = dm();
        io(&dm).await.set_connect_delay("ble-hr", Duration::from_secs(3));
        let a = tokio::spawn({
            let dm = dm.clone();
            async move { DeviceManager::connect_shared(&dm, "ble-hr", tx()).await }
        });
        settle().await;
        let b = tokio::spawn({
            let dm = dm.clone();
            async move { DeviceManager::connect_shared(&dm, "ble-hr", tx()).await }
        });
        settle().await;
        DeviceManager::disconnect_shared(&dm, "ble-hr").await.unwrap();
        assert!(matches!(a.await.unwrap(), Err(AppError::Cancelled)));
        assert!(
            matches!(b.await.unwrap(), Err(AppError::Cancelled)),
            "a connect queued behind the disconnect must not reconnect"
        );
        assert_eq!(connects(&io(&dm).await.events(), "ble-hr"), 1);
        assert_eq!(status(&dm, "ble-hr").await, None);
        assert!(dm.lock().await.in_flight.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn losing_attempt_cannot_remove_winner_token() {
        let dm = dm();
        io(&dm).await.set_connect_delay("ble-hr", Duration::from_secs(3));
        let winner = tokio::spawn({
            let dm = dm.clone();
            async move { DeviceManager::connect_shared(&dm, "ble-hr", tx()).await }
        });
        settle().await;
        let winner_token = dm.lock().await.in_flight.get("ble-hr").cloned().unwrap();
        let loser =
            DeviceManager::connect_with_origin(&dm, "ble-hr", tx(), ConnectOrigin::AutoReconnect).await;
        assert!(matches!(loser, Ok(ConnectOutcome::Skipped)));
        let still = dm.lock().await.in_flight.get("ble-hr").cloned().unwrap();
        assert!(still.same_as(&winner_token));
        winner.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn lost_teardown_runs_under_gate_and_keeps_reconnect_target() {
        let dm = dm();
        let tx = tx();
        DeviceManager::connect_shared(&dm, "ble-hr", tx.clone()).await.unwrap();
        let io = io(&dm).await;
        io.set_link("ble-hr", LinkState::Lost);
        let report = DeviceManager::watchdog_tick(&dm, &tx).await;
        assert_eq!(report.disconnected, vec!["ble-hr".to_string()]);
        settle().await;
        assert!(io.events().contains(&IoEvent::Disconnect("ble-hr".into())));
        assert_eq!(io.listeners_alive.load(Ordering::SeqCst), 0);
        let g = dm.lock().await;
        assert!(g.reconnect.contains("ble-hr"));
        assert_eq!(g.records["ble-hr"].phase, Phase::TornDown);
        assert_eq!(g.records["ble-hr"].info.status, ConnectionStatus::Reconnecting);
    }

    #[tokio::test(start_paused = true)]
    async fn lost_teardown_noop_after_concurrent_manual_disconnect() {
        let dm = dm();
        let tx = tx();
        DeviceManager::connect_shared(&dm, "ble-hr", tx.clone()).await.unwrap();
        let io = io(&dm).await;
        io.set_link("ble-hr", LinkState::Lost);
        // Hold the gate: the manual disconnect queues first, the spawned teardown second.
        let gate = dm.lock().await.gate("ble-hr");
        let held = gate.clone().lock_owned().await;
        let disconnect = tokio::spawn({
            let dm = dm.clone();
            async move { DeviceManager::disconnect_shared(&dm, "ble-hr").await }
        });
        settle().await;
        DeviceManager::watchdog_tick(&dm, &tx).await;
        settle().await;
        drop(held);
        disconnect.await.unwrap().unwrap();
        settle().await;
        let disconnects =
            io.events().iter().filter(|e| **e == IoEvent::Disconnect("ble-hr".into())).count();
        assert_eq!(disconnects, 1, "the late teardown must be a no-op: {:?}", io.events());
        let g = dm.lock().await;
        assert!(!g.records.contains_key("ble-hr"));
        assert!(!g.reconnect.contains("ble-hr"), "manual disconnect stops reconnects");
        assert_eq!(io.listeners_alive.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn connect_wins_gate_before_spawned_teardown_reopens_cleanly() {
        let dm = dm();
        let tx = tx();
        let id = "ant:120:1";
        DeviceManager::connect_shared(&dm, id, tx.clone()).await.unwrap();
        let io = io(&dm).await;
        io.set_link(id, LinkState::Lost);
        // A manual connect queues on the gate before the spawned teardown task runs.
        let gate = dm.lock().await.gate(id);
        let held = gate.clone().lock_owned().await;
        DeviceManager::watchdog_tick(&dm, &tx).await; // marks Lost, spawns teardown (not yet run)
        let (connected, _) = tokio::join!(DeviceManager::connect_shared(&dm, id, tx.clone()), async {
            settle().await; // connect is first in the gate queue, teardown second
            drop(held);
        });
        let info = connected.unwrap();
        settle().await;
        assert_eq!(info.status, ConnectionStatus::Connected);
        assert_eq!(
            io.events(),
            vec![
                IoEvent::Connect(id.into()),
                IoEvent::Disconnect(id.into()),
                IoEvent::Connect(id.into()),
            ],
            "the gate holder tears down before reopening; the late teardown is a no-op"
        );
        assert_eq!(io.listeners_alive.load(Ordering::SeqCst), 1);
        assert_eq!(status(&dm, id).await, Some(ConnectionStatus::Connected));
    }

    #[tokio::test(start_paused = true)]
    async fn slow_reconnect_does_not_delay_next_tick() {
        let dm = dm();
        let tx = tx();
        DeviceManager::connect_shared(&dm, "ble-hr", tx.clone()).await.unwrap();
        DeviceManager::connect_shared(&dm, "ble-pm", tx.clone()).await.unwrap();
        let io = io(&dm).await;
        lose_and_wait_for_retry(&dm, "ble-hr", &tx).await;
        io.set_connect_delay("ble-hr", Duration::from_secs(30));
        DeviceManager::watchdog_tick(&dm, &tx).await; // spawns the 30 s attempt
        settle().await;
        io.set_link("ble-pm", LinkState::Lost);
        let start = Instant::now();
        let report = DeviceManager::watchdog_tick(&dm, &tx).await;
        assert!(start.elapsed() < Duration::from_millis(50));
        assert_eq!(report.disconnected, vec!["ble-pm".to_string()]);
    }

    #[tokio::test(start_paused = true)]
    async fn reconnects_bounded_to_two_concurrent() {
        let dm = dm();
        let tx = tx();
        let ids = ["ble-a", "ble-b", "ble-c"];
        for id in ids {
            DeviceManager::connect_shared(&dm, id, tx.clone()).await.unwrap();
        }
        let io = io(&dm).await;
        for id in ids {
            io.set_link(id, LinkState::Lost);
            io.set_connect_delay(id, Duration::from_secs(10));
        }
        DeviceManager::watchdog_tick(&dm, &tx).await;
        settle().await;
        tokio::time::advance(Duration::from_millis(config::RECONNECT_INITIAL_BACKOFF_MS + 1)).await;
        DeviceManager::watchdog_tick(&dm, &tx).await;
        settle().await;
        assert_eq!(dm.lock().await.in_flight.len(), 2, "the third attempt waits for a permit");
    }

    #[tokio::test(start_paused = true)]
    async fn busy_gate_skips_reconnect_round() {
        let dm = dm();
        let tx = tx();
        DeviceManager::connect_shared(&dm, "ble-hr", tx.clone()).await.unwrap();
        lose_and_wait_for_retry(&dm, "ble-hr", &tx).await;
        let gate = dm.lock().await.gate("ble-hr");
        let held = gate.clone().lock_owned().await;
        DeviceManager::watchdog_tick(&dm, &tx).await;
        settle().await;
        assert_eq!(connects(&io(&dm).await.events(), "ble-hr"), 1, "busy device: no attempt");
        assert_eq!(dm.lock().await.reconnect.attempt_count("ble-hr"), 0, "backoff not consumed");
        drop(held);
    }

    #[tokio::test(start_paused = true)]
    async fn reconnect_success_reported_next_tick() {
        let dm = dm();
        let tx = tx();
        DeviceManager::connect_shared(&dm, "ble-hr", tx.clone()).await.unwrap();
        lose_and_wait_for_retry(&dm, "ble-hr", &tx).await;
        DeviceManager::watchdog_tick(&dm, &tx).await;
        settle().await;
        let report = DeviceManager::watchdog_tick(&dm, &tx).await;
        assert_eq!(report.reconnected, vec!["ble-hr".to_string()]);
        assert_eq!(status(&dm, "ble-hr").await, Some(ConnectionStatus::Connected));
        assert_eq!(io(&dm).await.listeners_alive.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn watchdog_tick_completes_within_250ms_during_scan() {
        use crate::device::io::{AntLink, RealIo};
        use std::sync::atomic::{AtomicBool, AtomicI64};
        let io = RealIo::new();
        let id = "ant:120:7";
        let last_seen = Arc::new(AtomicI64::new(0));
        crate::device::ant::listener::atomic_set_age(&last_seen, Duration::from_secs(1));
        io.insert_link(id, AntLink { last_seen, closed: Arc::new(AtomicBool::new(false)) });
        let locked = io.hold_ant_slot_for(Duration::from_secs(2));
        let mut manager = DeviceManager::with_io(io);
        manager.records.insert(
            id.to_string(),
            DeviceRecord { info: fake_info(id), phase: Phase::Connected },
        );
        let dm = Arc::new(Mutex::new(manager));
        locked.recv().unwrap();
        let report = tokio::time::timeout(Duration::from_millis(250), DeviceManager::watchdog_tick(&dm, &tx()))
            .await
            .expect("watchdog must not wait for a scan holding the AntSlot");
        assert!(report.disconnected.is_empty(), "fresh link stays connected");
    }

}

#[cfg(test)]
mod ant_lifecycle_tests {
    //! ANT+ persistent-channel lifecycle (design ADR-2, 2a-2c) against FakeIo.
    use super::*;
    use crate::device::io::fake::{FakeIo, IoEvent};
    use std::sync::atomic::Ordering;

    type Dm = SharedDeviceManager<FakeIo>;
    const ID: &str = "ant:120:4242";

    fn dm() -> Dm {
        Arc::new(Mutex::new(DeviceManager::with_io(FakeIo::default())))
    }

    fn tx() -> broadcast::Sender<SensorReading> {
        broadcast::channel(16).0
    }

    async fn settle() {
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
    }

    async fn connected(dm: &Dm) -> Arc<FakeIo> {
        DeviceManager::connect_shared(dm, ID, tx()).await.unwrap();
        dm.lock().await.io()
    }

    async fn phase(dm: &Dm) -> Option<Phase> {
        dm.lock().await.records.get(ID).map(|r| r.phase)
    }

    fn count(io: &FakeIo, e: IoEvent) -> usize {
        io.events().iter().filter(|x| **x == e).count()
    }

    #[tokio::test(start_paused = true)]
    async fn watchdog_stale_ant_marked_reconnecting_not_connected() {
        let dm = dm();
        let io = connected(&dm).await;
        io.set_link(ID, LinkState::Stale);
        let report = DeviceManager::watchdog_tick(&dm, &tx()).await;
        assert_eq!(report.disconnected, vec![ID.to_string()]);
        let g = dm.lock().await;
        assert_eq!(g.records[ID].info.status, ConnectionStatus::Reconnecting);
        assert!(g.connected_trainer_id().is_none());
        assert!(!g.reconnect.contains(ID), "a searching channel needs no reconnect");
        assert!(g.primaries_handle().read().unwrap().legacy_map().is_empty(), "claims released");
    }

    #[tokio::test(start_paused = true)]
    async fn watchdog_stale_ant_reports_reconnecting_each_tick() {
        let dm = dm();
        let io = connected(&dm).await;
        io.set_link(ID, LinkState::Stale);
        DeviceManager::watchdog_tick(&dm, &tx()).await;
        let r2 = DeviceManager::watchdog_tick(&dm, &tx()).await;
        let r3 = DeviceManager::watchdog_tick(&dm, &tx()).await;
        assert!(r2.disconnected.is_empty(), "device_disconnected only once");
        assert_eq!(r2.reconnecting.iter().map(|(_, a)| *a).collect::<Vec<_>>(), vec![1]);
        assert_eq!(r3.reconnecting.iter().map(|(_, a)| *a).collect::<Vec<_>>(), vec![2]);
    }

    #[tokio::test(start_paused = true)]
    async fn watchdog_stale_ant_recovers_without_new_channel() {
        let dm = dm();
        let io = connected(&dm).await;
        io.set_link(ID, LinkState::Stale);
        DeviceManager::watchdog_tick(&dm, &tx()).await;
        io.set_link(ID, LinkState::Alive);
        let report = DeviceManager::watchdog_tick(&dm, &tx()).await;
        settle().await;
        assert_eq!(report.reconnected, vec![ID.to_string()]);
        assert_eq!(phase(&dm).await, Some(Phase::Connected));
        assert_eq!(count(&io, IoEvent::Connect(ID.into())), 1, "no second channel");
        assert_eq!(count(&io, IoEvent::Disconnect(ID.into())), 0);
        assert_eq!(io.listeners_alive.load(Ordering::SeqCst), 1, "single listener, no duplicates");
        assert!(dm.lock().await.primaries_handle().read().unwrap().is_registered(ID), "still registered; re-claims lazily");
    }

    #[tokio::test(start_paused = true)]
    async fn watchdog_lost_ant_tears_down_before_reconnect() {
        let dm = dm();
        let tx = tx();
        let io = connected(&dm).await;
        io.set_link(ID, LinkState::Lost); // channel closed by the radio
        DeviceManager::watchdog_tick(&dm, &tx).await;
        settle().await;
        tokio::time::advance(Duration::from_millis(crate::config::RECONNECT_INITIAL_BACKOFF_MS + 1)).await;
        DeviceManager::watchdog_tick(&dm, &tx).await;
        settle().await;
        assert_eq!(
            io.events(),
            vec![IoEvent::Connect(ID.into()), IoEvent::Disconnect(ID.into()), IoEvent::Connect(ID.into())],
        );
        assert_eq!(phase(&dm).await, Some(Phase::Connected));
    }

    #[tokio::test(start_paused = true)]
    async fn lost_teardown_then_manual_connect_opens_single_channel() {
        let dm = dm();
        let tx = tx();
        let io = connected(&dm).await;
        io.set_link(ID, LinkState::Lost);
        DeviceManager::watchdog_tick(&dm, &tx).await;
        settle().await; // spawned teardown has run
        let info = DeviceManager::connect_shared(&dm, ID, tx.clone()).await.unwrap();
        settle().await;
        assert_eq!(info.status, ConnectionStatus::Connected);
        assert_eq!(
            io.events(),
            vec![IoEvent::Connect(ID.into()), IoEvent::Disconnect(ID.into()), IoEvent::Connect(ID.into())],
        );
        assert_eq!(io.listeners_alive.load(Ordering::SeqCst), 1);
        assert!(!dm.lock().await.reconnect.contains(ID), "manual connect cleared the target");
    }

    #[tokio::test(start_paused = true)]
    async fn ant_reconnect_yields_single_listener() {
        let dm = dm();
        let tx = tx();
        let io = connected(&dm).await;
        io.set_link(ID, LinkState::Lost);
        DeviceManager::watchdog_tick(&dm, &tx).await;
        settle().await;
        tokio::time::advance(Duration::from_millis(crate::config::RECONNECT_INITIAL_BACKOFF_MS + 1)).await;
        DeviceManager::watchdog_tick(&dm, &tx).await;
        settle().await;
        assert_eq!(io.listeners_alive.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn manual_connect_reconnecting_ant_reuses_channel_and_connects_on_data() {
        let dm = dm();
        let io = connected(&dm).await;
        io.set_link(ID, LinkState::Stale);
        DeviceManager::watchdog_tick(&dm, &tx()).await;
        io.data_after.lock().unwrap().insert(ID.into(), Duration::from_secs(2));
        let info = DeviceManager::connect_shared(&dm, ID, tx()).await.unwrap();
        assert_eq!(info.status, ConnectionStatus::Connected);
        assert_eq!(count(&io, IoEvent::Connect(ID.into())), 1, "existing channel reused");
        assert_eq!(phase(&dm).await, Some(Phase::Connected));
    }

    #[tokio::test(start_paused = true)]
    async fn manual_connect_reconnecting_ant_without_data_returns_ok_reconnecting() {
        let dm = dm();
        let io = connected(&dm).await;
        io.set_link(ID, LinkState::Stale);
        DeviceManager::watchdog_tick(&dm, &tx()).await;
        let start = Instant::now();
        let info = DeviceManager::connect_shared(&dm, ID, tx()).await.unwrap();
        assert_eq!(info.status, ConnectionStatus::Reconnecting, "never reported Connected without data");
        assert!(start.elapsed() >= ANT_REJOIN_WAIT);
        assert_eq!(count(&io, IoEvent::Connect(ID.into())), 1);
        assert!(matches!(phase(&dm).await, Some(Phase::Stale { .. })));
    }

    #[tokio::test(start_paused = true)]
    async fn ant_stale_ten_minutes_gives_up_to_disconnected() {
        let dm = dm();
        let io = connected(&dm).await;
        io.set_link(ID, LinkState::Stale);
        DeviceManager::watchdog_tick(&dm, &tx()).await;
        tokio::time::advance(ANT_GIVE_UP - Duration::from_secs(1)).await;
        DeviceManager::watchdog_tick(&dm, &tx()).await;
        assert!(matches!(phase(&dm).await, Some(Phase::Stale { .. })), "not yet");
        tokio::time::advance(Duration::from_secs(2)).await;
        let report = DeviceManager::watchdog_tick(&dm, &tx()).await;
        settle().await;
        assert_eq!(report.disconnected, vec![ID.to_string()], "device_disconnected re-emitted");
        assert_eq!(phase(&dm).await, None, "shown as Disconnected (known device only)");
    }

    #[tokio::test(start_paused = true)]
    async fn ant_give_up_frees_channel_and_registers_no_reconnect() {
        let dm = dm();
        let io = connected(&dm).await;
        io.set_link(ID, LinkState::Stale);
        DeviceManager::watchdog_tick(&dm, &tx()).await;
        tokio::time::advance(ANT_GIVE_UP + Duration::from_secs(1)).await;
        DeviceManager::watchdog_tick(&dm, &tx()).await;
        settle().await;
        assert_eq!(count(&io, IoEvent::Disconnect(ID.into())), 1, "channel freed");
        assert_eq!(io.listeners_alive.load(Ordering::SeqCst), 0);
        assert!(!dm.lock().await.reconnect.contains(ID));
    }
}

#[cfg(test)]
mod primary_lifecycle_tests {
    //! Primary-state bookkeeping across connect/disconnect (ADR-3a, NB2).
    use super::*;
    use crate::device::io::fake::FakeIo;

    type Dm = SharedDeviceManager<FakeIo>;

    fn dm() -> Dm {
        Arc::new(Mutex::new(DeviceManager::with_io(FakeIo::default())))
    }

    fn tx() -> broadcast::Sender<SensorReading> {
        broadcast::channel(16).0
    }

    async fn settle() {
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn trainer_primary_claimed_at_connect_commit() {
        let dm = dm();
        DeviceManager::connect_shared(&dm, "ble-trainer", tx()).await.unwrap();
        let map = dm.lock().await.primaries_handle().read().unwrap().legacy_map();
        assert_eq!(map.get("FitnessTrainer").map(String::as_str), Some("ble-trainer"));
    }

    #[tokio::test(start_paused = true)]
    async fn non_trainer_primary_not_claimed_until_first_reading() {
        let dm = dm();
        DeviceManager::connect_shared(&dm, "ble-pm", tx()).await.unwrap();
        let handle = dm.lock().await.primaries_handle();
        assert!(handle.read().unwrap().legacy_map().is_empty());
        assert!(handle.read().unwrap().is_registered("ble-pm"));
    }

    #[tokio::test(start_paused = true)]
    async fn losing_attempt_cannot_remove_winner_caps() {
        let dm = dm();
        dm.lock().await.io().set_connect_delay("ble-pm", Duration::from_secs(3));
        let winner = tokio::spawn({
            let dm = dm.clone();
            async move { DeviceManager::connect_shared(&dm, "ble-pm", tx()).await }
        });
        settle().await;
        // The winner registered (pending) caps post-gate; a losing auto attempt is skipped.
        let loser =
            DeviceManager::connect_with_origin(&dm, "ble-pm", tx(), ConnectOrigin::AutoReconnect).await;
        assert!(matches!(loser, Ok(ConnectOutcome::Skipped)));
        let handle = dm.lock().await.primaries_handle();
        assert!(handle.read().unwrap().is_registered("ble-pm"));
        winner.await.unwrap().unwrap();
        assert!(handle.read().unwrap().is_registered("ble-pm"));
    }

    #[tokio::test(start_paused = true)]
    async fn failed_connect_unregisters_pending_caps() {
        let dm = dm();
        dm.lock().await.io().set_connect_fail("ble-pm", "gone");
        assert!(DeviceManager::connect_shared(&dm, "ble-pm", tx()).await.is_err());
        let handle = dm.lock().await.primaries_handle();
        assert!(!handle.read().unwrap().is_registered("ble-pm"));
    }

    #[tokio::test(start_paused = true)]
    async fn disconnect_unregisters_device_so_late_readings_drop() {
        let dm = dm();
        DeviceManager::connect_shared(&dm, "ble-pm", tx()).await.unwrap();
        let handle = dm.lock().await.primaries_handle();
        let reading = SensorReading::Power {
            watts: 250,
            timestamp: None,
            epoch_ms: 0,
            device_id: "ble-pm".into(),
            pedal_balance: None,
        };
        assert!(crate::device::primary::admit_reading(&handle, &reading, std::time::Instant::now()));
        DeviceManager::disconnect_shared(&dm, "ble-pm").await.unwrap();
        assert!(!crate::device::primary::admit_reading(&handle, &reading, std::time::Instant::now()));
        assert!(handle.read().unwrap().legacy_map().is_empty(), "no re-claim after disconnect");
    }
}

#[cfg(test)]
mod ui_event_tests {
    //! What the frontend is sent: the emitted event sequence, captured from
    //! `report_events` (the same path lib.rs emits), with real storage.
    use super::*;
    use crate::device::io::fake::{info as fake_info, FakeIo, IoEvent};
    use std::sync::atomic::Ordering;

    const ANT: &str = "ant:120:777";

    fn tx() -> broadcast::Sender<SensorReading> {
        broadcast::channel(16).0
    }

    async fn settle() {
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
    }

    /// DM backed by a real (temporary) SQLite store that knows `ids`.
    async fn dm_with_storage(ids: &[&str]) -> (SharedDeviceManager<FakeIo>, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let storage = Arc::new(Storage::new(&tmp.path().to_string_lossy()).await.unwrap());
        for id in ids {
            storage.upsert_known_device(&fake_info(id, ConnectionStatus::Disconnected)).await.unwrap();
        }
        let mut manager = DeviceManager::with_io(FakeIo::default());
        manager.set_storage(storage);
        (Arc::new(Mutex::new(manager)), tmp)
    }

    async fn tick_events(dm: &SharedDeviceManager<FakeIo>) -> Vec<DeviceEvent> {
        let report = DeviceManager::watchdog_tick(dm, &tx()).await;
        DeviceManager::report_events(dm, report).await
    }

    fn listed_status(events: &[DeviceEvent], id: &str) -> Vec<ConnectionStatus> {
        events
            .iter()
            .filter_map(|e| match e {
                DeviceEvent::ListUpdated(list) => list.iter().find(|d| d.id == id).map(|d| d.status),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn give_up_emits_list_only_after_teardown_and_final_status_disconnected() {
        let (dm, _tmp) = dm_with_storage(&[ANT]).await;
        dm.lock().await.ant_give_up = Duration::ZERO;
        DeviceManager::connect_shared(&dm, ANT, tx()).await.unwrap();
        let io = dm.lock().await.io();
        io.set_link(ANT, LinkState::Stale);

        let stale = tick_events(&dm).await;
        assert_eq!(listed_status(&stale, ANT), vec![ConnectionStatus::Reconnecting]);

        let give_up = tick_events(&dm).await;
        assert!(matches!(give_up.as_slice(), [DeviceEvent::Disconnected(id)] if id == ANT),
            "no list while the teardown is pending: {:?}", give_up);

        settle().await; // teardown task completes
        let after = tick_events(&dm).await;
        assert_eq!(listed_status(&after, ANT), vec![ConnectionStatus::Disconnected]);
        assert!(io.events().contains(&IoEvent::Disconnect(ANT.into())));
    }

    #[tokio::test]
    async fn lost_transition_emits_reconnecting_list() {
        let (dm, _tmp) = dm_with_storage(&["ble-hr"]).await;
        DeviceManager::connect_shared(&dm, "ble-hr", tx()).await.unwrap();
        dm.lock().await.io().set_link("ble-hr", LinkState::Lost);
        let events = tick_events(&dm).await;
        assert!(matches!(&events[0], DeviceEvent::Disconnected(id) if id == "ble-hr"));
        assert_eq!(listed_status(&events, "ble-hr"), vec![ConnectionStatus::Reconnecting]);
    }

    #[tokio::test]
    async fn failed_manual_connect_keeps_reconnect_target_and_reconnecting() {
        let (dm, _tmp) = dm_with_storage(&["ble-hr"]).await;
        DeviceManager::connect_shared(&dm, "ble-hr", tx()).await.unwrap();
        let io = dm.lock().await.io();
        io.set_link("ble-hr", LinkState::Lost);
        tick_events(&dm).await;
        settle().await; // torn down, waiting for auto-reconnect
        io.set_connect_fail("ble-hr", "out of range");
        assert!(DeviceManager::connect_shared(&dm, "ble-hr", tx()).await.is_err());
        let g = dm.lock().await;
        assert!(g.reconnect.contains("ble-hr"), "auto-reconnect continues");
        let listed = g.list_current().await;
        let hr = listed.iter().find(|d| d.id == "ble-hr").unwrap();
        assert_eq!(hr.status, ConnectionStatus::Reconnecting);
    }

    #[tokio::test]
    async fn disconnect_stale_ant_closes_channel_and_clears_state() {
        let (dm, _tmp) = dm_with_storage(&[ANT]).await;
        DeviceManager::connect_shared(&dm, ANT, tx()).await.unwrap();
        let io = dm.lock().await.io();
        io.set_link(ANT, LinkState::Stale);
        tick_events(&dm).await; // Reconnecting, channel still searching
        DeviceManager::disconnect_shared(&dm, ANT).await.unwrap();
        settle().await;
        assert!(io.events().contains(&IoEvent::Disconnect(ANT.into())), "searching channel closed");
        assert_eq!(io.listeners_alive.load(Ordering::SeqCst), 0);
        let g = dm.lock().await;
        assert!(!g.records.contains_key(ANT));
        assert!(!g.reconnect.contains(ANT));
        let listed = g.list_current().await;
        assert_eq!(listed.iter().find(|d| d.id == ANT).unwrap().status, ConnectionStatus::Disconnected);
    }

    #[tokio::test]
    async fn disconnect_reconnecting_ble_clears_reconnect_target() {
        let (dm, _tmp) = dm_with_storage(&["ble-hr"]).await;
        DeviceManager::connect_shared(&dm, "ble-hr", tx()).await.unwrap();
        dm.lock().await.io().set_link("ble-hr", LinkState::Lost);
        tick_events(&dm).await;
        settle().await;
        assert!(dm.lock().await.reconnect.contains("ble-hr"));
        DeviceManager::disconnect_shared(&dm, "ble-hr").await.unwrap();
        let g = dm.lock().await;
        assert!(!g.reconnect.contains("ble-hr"));
        assert!(!g.records.contains_key("ble-hr"));
    }
}
