//! Transport I/O seam for `DeviceManager`.
//!
//! `DeviceManager` keeps only bookkeeping and never holds its own lock across the
//! futures defined here (design ADR-1). `RealIo` owns the BLE and ANT+ transports,
//! which are internally synchronised:
//! - BLE: `BleManager` uses inner `tokio::sync::Mutex` maps.
//! - ANT+: `AntManager` lives in `AntSlot` behind a `std::sync::Mutex` that is only
//!   ever locked on `spawn_blocking` threads (ADR-1a). Per-device link handles
//!   (`last_seen`, `closed`) live in `AntLinks`, outside the slot, so the watchdog
//!   probe never waits on a scan.

use log::{info, warn};
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::{broadcast, Notify, OnceCell};
use tokio::task::JoinHandle;

use super::ant::listener::atomic_elapsed;
use super::ant::manager::AntManager;
use super::ble::BleManager;
use super::ftms::TrainerController;
use super::listener::listen_to_device;
use super::types::*;
use crate::config;
use crate::error::{AntError, AppError, BleError};

/// Shared primary-source state handed to listeners.
pub use super::primary::SharedPrimaryState as PrimariesHandle;

/// Trainer control backend for a connected trainer.
pub enum TrainerBackend {
    Ftms(TrainerController),
    Fec {
        link: super::fec::FecLink,
        /// Wind coefficient last acknowledged by the trainer (page 0x32 is resent only on change).
        last_cw_raw: Option<u8>,
    },
    /// Test double: counts commands, completes immediately.
    #[cfg(test)]
    Fake(Arc<std::sync::atomic::AtomicUsize>),
}

/// Liveness of a connected device as seen by the transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    /// Data/connection is healthy.
    Alive,
    /// ANT+: no page for longer than the disconnect timeout, channel still searching.
    Stale,
    /// BLE disconnected, or ANT+ channel closed.
    Lost,
}

/// Per-ANT+-device handles shared between the listener, AntManager and the watchdog.
#[derive(Clone)]
pub struct AntLink {
    pub last_seen: Arc<AtomicI64>,
    pub closed: Arc<AtomicBool>,
}

/// Map of ANT+ link handles. Locked only for get/insert/remove, never across I/O.
pub type AntLinks = Arc<StdMutex<HashMap<String, AntLink>>>;

/// Everything a listener needs to publish readings.
#[derive(Clone)]
pub struct ListenerCtx {
    pub tx: broadcast::Sender<SensorReading>,
    pub primaries: PrimariesHandle,
}

/// Result of a successful transport connect.
pub struct Connected {
    pub info: DeviceInfo,
    /// Async listener task (BLE). ANT+ listeners are owned by AntManager.
    pub listener: Option<JoinHandle<()>>,
    pub trainer: Option<TrainerBackend>,
}

/// Cooperative cancellation for an in-flight connect (no extra crate needed).
#[derive(Clone, Default)]
pub struct CancelToken(Arc<CancelInner>);

#[derive(Default)]
struct CancelInner {
    flag: Arc<AtomicBool>,
    notify: Notify,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.flag.store(true, Ordering::SeqCst);
        self.0.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.flag.load(Ordering::SeqCst)
    }

    /// Flag for blocking code that polls (ANT+ loops check it every ≤ 50 ms).
    pub fn flag(&self) -> Arc<AtomicBool> {
        self.0.flag.clone()
    }

    /// Resolves once `cancel()` has been called.
    pub async fn cancelled(&self) {
        loop {
            let notified = self.0.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }

    pub fn same_as(&self, other: &CancelToken) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Transport operations used by `DeviceManager`. Implementations must be internally
/// synchronised; callers never hold the DeviceManager lock while awaiting them.
pub trait DeviceIo: Send + Sync + 'static {
    /// Scan all transports (BLE and ANT+ concurrently).
    fn scan(&self) -> impl Future<Output = Vec<DeviceInfo>> + Send;
    /// Connect and start the device's listener.
    fn connect(
        &self,
        device_id: &str,
        ctx: ListenerCtx,
        cancel: CancelToken,
    ) -> impl Future<Output = Result<Connected, AppError>> + Send;
    /// Tear down the transport side of a device (channel/peripheral). Idempotent.
    fn disconnect(&self, device_id: &str) -> impl Future<Output = Result<(), AppError>> + Send;
    /// Report link state for the given ids.
    fn probe(&self, ids: Vec<String>) -> impl Future<Output = Vec<(String, LinkState)>> + Send;
    /// ANT+: wait (async, polling the shared last-seen handle, never the AntSlot)
    /// for a page newer than the moment of the call. False on timeout or cancel.
    fn await_data(
        &self,
        device_id: &str,
        timeout: Duration,
        cancel: &CancelToken,
    ) -> impl Future<Output = bool> + Send;
    /// BLE GATT details for a connected device.
    fn ble_details(&self, device_id: &str) -> impl Future<Output = Result<DeviceDetails, AppError>> + Send;
    /// Snapshot of ANT+ common-page metadata.
    fn ant_metadata(&self) -> HashMap<String, AntDeviceMetadata>;
}

/// ANT+ manager plus lazy-init bookkeeping, locked only on blocking threads.
#[derive(Default)]
struct AntSlot {
    mgr: Option<AntManager>,
    /// True if AntManager was ever successfully initialised (for panic recovery logs).
    was_available: bool,
    /// True after a failed USB probe; prevents repeated enumeration until a user scan.
    probe_failed: bool,
}

impl AntSlot {
    fn ensure(&mut self, metadata: &MetadataStore, links: &AntLinks) {
        if self.mgr.is_some() || self.probe_failed {
            return;
        }
        if self.was_available {
            warn!("ANT+ manager was lost (panic?), attempting re-initialization");
        }
        match AntManager::try_new(metadata.clone(), links.clone()) {
            Some(m) => {
                self.mgr = Some(m);
                self.was_available = true;
            }
            None => self.probe_failed = true,
        }
    }
}

type MetadataStore = Arc<StdMutex<HashMap<String, AntDeviceMetadata>>>;

/// Production transports.
pub struct RealIo {
    ble: OnceCell<BleManager>,
    ant: Arc<StdMutex<AntSlot>>,
    links: AntLinks,
    metadata: MetadataStore,
}

impl Default for RealIo {
    fn default() -> Self {
        Self::new()
    }
}

impl RealIo {
    pub fn new() -> Self {
        Self {
            ble: OnceCell::new(),
            ant: Arc::new(StdMutex::new(AntSlot::default())),
            links: Arc::new(StdMutex::new(HashMap::new())),
            metadata: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    /// Test hooks: hold the AntSlot lock / register a link without hardware.
    #[cfg(test)]
    pub(crate) fn hold_ant_slot_for(&self, d: Duration) -> std::sync::mpsc::Receiver<()> {
        let slot = self.ant.clone();
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _g = slot.lock().unwrap();
            let _ = locked_tx.send(());
            std::thread::sleep(d);
        });
        locked_rx
    }

    #[cfg(test)]
    pub(crate) fn insert_link(&self, id: &str, link: AntLink) {
        self.links.lock().unwrap().insert(id.to_string(), link);
    }

    async fn ble(&self) -> Result<&BleManager, AppError> {
        self.ble
            .get_or_try_init(BleManager::new)
            .await
            .map_err(|e| BleError::Btleplug(format!("BLE init failed: {}", e)).into())
    }

    /// Run blocking work with the AntManager on a blocking thread.
    /// A panic inside `f` poisons the slot; the next call drops the (possibly
    /// inconsistent) manager and re-initialises it.
    async fn with_ant<F, R>(&self, reset_probe: bool, f: F) -> Result<R, AppError>
    where
        F: FnOnce(&mut AntManager) -> R + Send + 'static,
        R: Send + 'static,
    {
        let slot = self.ant.clone();
        let metadata = self.metadata.clone();
        let links = self.links.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = match slot.lock() {
                Ok(g) => g,
                Err(poisoned) => {
                    log::error!("[ant+] Slot poisoned by an earlier panic; resetting ANT+ manager");
                    let mut g = poisoned.into_inner();
                    g.mgr = None;
                    slot.clear_poison();
                    g
                }
            };
            if reset_probe {
                guard.probe_failed = false;
            }
            guard.ensure(&metadata, &links);
            match guard.mgr.as_mut() {
                Some(mgr) => Ok(f(mgr)),
                None => Err(AppError::from(AntError::NoUsbStick)),
            }
        })
        .await
        .map_err(|e| {
            log::error!("[ant+] Blocking task panicked: {}", e);
            AppError::from(AntError::TaskPanicked(format!("{}", e)))
        })?
    }

    async fn connect_ble(
        &self,
        device_id: &str,
        ctx: ListenerCtx,
    ) -> Result<Connected, AppError> {
        let ble = self.ble().await?;
        let mut info = ble.connect_device(device_id).await?;

        // Read DIS metadata to populate manufacturer/model/serial
        if let Ok(details) = ble.get_device_details(device_id).await {
            info.manufacturer = details.manufacturer;
            info.model_number = details.model_number;
            info.serial_number = details.serial_number;
        }

        let peripheral = {
            let connected = ble.get_connected();
            let lock = connected.lock().await;
            lock.get(device_id).cloned()
        };
        let Some(peripheral) = peripheral else {
            warn!("[{}] Peripheral not found in connected map after connect", device_id);
            return Ok(Connected { info, listener: None, trainer: None });
        };

        let trainer = if info.device_type == DeviceType::FitnessTrainer {
            match TrainerController::new(peripheral.clone()) {
                Ok(c) => {
                    info!("[{}] FTMS trainer controller created", device_id);
                    Some(TrainerBackend::Ftms(c))
                }
                Err(e) => {
                    warn!("[{}] FTMS controller unavailable: {}", device_id, e);
                    None
                }
            }
        } else {
            None
        };

        let device_type = info.device_type;
        let did = device_id.to_string();
        let primaries = Some(ctx.primaries.clone());
        let tx = ctx.tx.clone();
        let listener = tokio::spawn(async move {
            listen_to_device(peripheral, device_type, tx, did, primaries).await;
        });

        Ok(Connected { info, listener: Some(listener), trainer })
    }

    async fn connect_ant(
        &self,
        device_id: &str,
        ctx: ListenerCtx,
        cancel: CancelToken,
    ) -> Result<Connected, AppError> {
        let id = device_id.to_string();
        let cancel_flag = cancel.flag();
        let (info, fec) = self
            .with_ant(false, move |ant| -> Result<_, AppError> {
                if !ant.is_discovered(&id) {
                    let _ = ant.scan();
                }
                if cancel_flag.load(Ordering::SeqCst) {
                    return Err(AppError::Cancelled);
                }
                let info = ant.connect(&id, ctx.tx, Some(ctx.primaries), &cancel_flag)?;
                let fec = if info.device_type == DeviceType::FitnessTrainer {
                    ant.get_fec_channel(&id)
                } else {
                    None
                };
                Ok((info, fec))
            })
            .await??;
        let trainer = fec.map(|link| {
            info!("[{}] FE-C trainer controller created", device_id);
            TrainerBackend::Fec { link, last_cw_raw: None }
        });
        Ok(Connected { info, listener: None, trainer })
    }
}

impl DeviceIo for RealIo {
    async fn scan(&self) -> Vec<DeviceInfo> {
        let mut found = Vec::new();
        let ble = match self.ble().await {
            Ok(b) => Some(b),
            Err(e) => {
                warn!("[ble] Not available: {}", e);
                None
            }
        };
        if let Some(ble) = ble {
            if let Err(e) = ble.start_scan().await {
                warn!("[ble] Scan start failed: {}", e);
            }
        }

        // ANT+ probe+scan runs on a blocking thread while BLE scans.
        // User-initiated scan always retries the ANT+ probe.
        let ant_scan = self.with_ant(true, |ant| ant.scan());

        let ble_part = async {
            tokio::time::sleep(Duration::from_secs(config::BLE_SCAN_DURATION_SECS)).await;
            let mut out = Vec::new();
            if let Some(ble) = ble {
                let _ = ble.stop_scan().await;
                match ble.get_discovered_devices().await {
                    Ok(devices) => out = devices,
                    Err(e) => warn!("[ble] Discovery failed: {}", e),
                }
            }
            out
        };

        let (ble_devices, ant_result) = tokio::join!(ble_part, ant_scan);
        found.extend(ble_devices);
        match ant_result {
            Ok(Ok(devices)) => found.extend(devices),
            Ok(Err(e)) => warn!("[ant+] Scan failed: {}", e),
            Err(e) => log::debug!("[ant+] Not available for scan: {}", e),
        }
        found
    }

    async fn connect(
        &self,
        device_id: &str,
        ctx: ListenerCtx,
        cancel: CancelToken,
    ) -> Result<Connected, AppError> {
        if device_id.starts_with("ant:") {
            self.connect_ant(device_id, ctx, cancel).await
        } else {
            let attempt = self.connect_ble(device_id, ctx);
            tokio::select! {
                r = attempt => r,
                _ = cancel.cancelled() => {
                    // Undo any partial connection before reporting cancellation.
                    if let Ok(ble) = self.ble().await {
                        if let Err(e) = ble.disconnect_device(device_id).await {
                            warn!("[{}] Disconnect after cancelled connect failed: {}", device_id, e);
                        }
                    }
                    Err(AppError::Cancelled)
                }
            }
        }
    }

    async fn disconnect(&self, device_id: &str) -> Result<(), AppError> {
        if device_id.starts_with("ant:") {
            let id = device_id.to_string();
            match self.with_ant(false, move |ant| ant.disconnect(&id)).await {
                Ok(r) => r,
                // No stick (or lost): nothing to tear down on the transport side.
                Err(AppError::AntPlus(AntError::NoUsbStick)) => Ok(()),
                Err(e) => Err(e),
            }
        } else {
            match self.ble.get() {
                Some(ble) => ble.disconnect_device(device_id).await,
                None => Err(BleError::NotInitialized.into()),
            }
        }
    }

    async fn probe(&self, ids: Vec<String>) -> Vec<(String, LinkState)> {
        let mut out = Vec::with_capacity(ids.len());
        let (ant_ids, ble_ids): (Vec<String>, Vec<String>) =
            ids.into_iter().partition(|id| id.starts_with("ant:"));

        if !ble_ids.is_empty() {
            if let Some(ble) = self.ble.get() {
                let connected_arc = ble.get_connected();
                let peripherals: Vec<(String, Option<btleplug::platform::Peripheral>)> = {
                    let connected = connected_arc.lock().await;
                    ble_ids.iter().map(|id| (id.clone(), connected.get(id).cloned())).collect()
                };
                for (id, p) in peripherals {
                    use btleplug::api::Peripheral as _;
                    let alive = match p {
                        Some(p) => p.is_connected().await.unwrap_or(false),
                        None => false,
                    };
                    if !alive {
                        // Drop it from the BLE connected map so a reconnect starts fresh.
                        connected_arc.lock().await.remove(&id);
                    }
                    out.push((id, if alive { LinkState::Alive } else { LinkState::Lost }));
                }
            } else {
                out.extend(ble_ids.into_iter().map(|id| (id, LinkState::Lost)));
            }
        }

        if !ant_ids.is_empty() {
            let links = self.links.lock().unwrap_or_else(|e| e.into_inner());
            for id in ant_ids {
                out.push((id.clone(), ant_link_state(links.get(&id))));
            }
        }
        out
    }

    async fn await_data(&self, device_id: &str, timeout: Duration, cancel: &CancelToken) -> bool {
        let link = self.links.lock().unwrap_or_else(|e| e.into_inner()).get(device_id).cloned();
        let Some(link) = link else {
            return false;
        };
        let start = link.last_seen.load(Ordering::SeqCst);
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            if link.last_seen.load(Ordering::SeqCst) != start {
                return true;
            }
            if link.closed.load(Ordering::SeqCst) {
                return false;
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
                _ = cancel.cancelled() => return false,
            }
        }
        false
    }

    async fn ble_details(&self, device_id: &str) -> Result<DeviceDetails, AppError> {
        match self.ble.get() {
            Some(ble) => ble.get_device_details(device_id).await,
            None => Err(BleError::NotInitialized.into()),
        }
    }

    fn ant_metadata(&self) -> HashMap<String, AntDeviceMetadata> {
        self.metadata.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Classify an ANT+ link from its shared handles.
pub fn ant_link_state(link: Option<&AntLink>) -> LinkState {
    let Some(link) = link else {
        return LinkState::Lost;
    };
    if link.closed.load(Ordering::SeqCst) {
        return LinkState::Lost;
    }
    match atomic_elapsed(&link.last_seen) {
        Some(elapsed) if elapsed > Duration::from_secs(config::DEVICE_DISCONNECT_TIMEOUT_SECS) => {
            LinkState::Stale
        }
        // No page yet (0) → just connected, give it time
        _ => LinkState::Alive,
    }
}

#[cfg(test)]
pub mod fake {
    //! In-memory transport for lifecycle/concurrency tests.
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::AtomicUsize;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum IoEvent {
        Scan,
        Connect(String),
        ConnectCancelled(String),
        Disconnect(String),
    }

    #[derive(Default)]
    pub struct FakeIo {
        pub log: StdMutex<Vec<IoEvent>>,
        pub scan_delay: StdMutex<Duration>,
        pub scan_result: StdMutex<Vec<DeviceInfo>>,
        pub connect_delay: StdMutex<HashMap<String, Duration>>,
        pub connect_fail: StdMutex<HashMap<String, String>>,
        /// Ids whose connect ignores cancellation (completes successfully anyway).
        pub ignore_cancel: StdMutex<HashSet<String>>,
        pub links: StdMutex<HashMap<String, LinkState>>,
        /// Number of listener tasks currently alive.
        pub listeners_alive: Arc<AtomicUsize>,
        /// await_data: delay after which data "arrives" for an id (absent = never).
        pub data_after: StdMutex<HashMap<String, Duration>>,
        pub trainer_commands: Arc<AtomicUsize>,
    }

    struct AliveGuard(Arc<AtomicUsize>);
    impl Drop for AliveGuard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl FakeIo {
        pub fn events(&self) -> Vec<IoEvent> {
            self.log.lock().unwrap().clone()
        }
        pub fn set_link(&self, id: &str, state: LinkState) {
            self.links.lock().unwrap().insert(id.to_string(), state);
        }
        pub fn set_connect_delay(&self, id: &str, d: Duration) {
            self.connect_delay.lock().unwrap().insert(id.to_string(), d);
        }
        pub fn set_connect_fail(&self, id: &str, msg: &str) {
            self.connect_fail.lock().unwrap().insert(id.to_string(), msg.to_string());
        }
        fn push(&self, e: IoEvent) {
            self.log.lock().unwrap().push(e);
        }
    }

    pub fn device_type_for(id: &str) -> DeviceType {
        if id.contains("trainer") || id.starts_with("ant:17:") {
            DeviceType::FitnessTrainer
        } else if id.contains("hr") || id.starts_with("ant:120:") {
            DeviceType::HeartRate
        } else if id.contains("pm") {
            DeviceType::Power
        } else {
            DeviceType::CadenceSpeed
        }
    }

    pub fn info(id: &str, status: ConnectionStatus) -> DeviceInfo {
        DeviceInfo {
            id: id.to_string(),
            name: Some(format!("Fake {}", id)),
            device_type: device_type_for(id),
            status,
            transport: if id.starts_with("ant:") { Transport::AntPlus } else { Transport::Ble },
            rssi: None,
            battery_level: None,
            last_seen: None,
            manufacturer: None,
            model_number: None,
            serial_number: None,
            device_group: None,
            in_range: true,
        }
    }

    impl DeviceIo for FakeIo {
        async fn scan(&self) -> Vec<DeviceInfo> {
            self.push(IoEvent::Scan);
            let d = *self.scan_delay.lock().unwrap();
            tokio::time::sleep(d).await;
            self.scan_result.lock().unwrap().clone()
        }

        async fn connect(
            &self,
            device_id: &str,
            _ctx: ListenerCtx,
            cancel: CancelToken,
        ) -> Result<Connected, AppError> {
            self.push(IoEvent::Connect(device_id.to_string()));
            let d = self.connect_delay.lock().unwrap().get(device_id).copied().unwrap_or_default();
            if self.ignore_cancel.lock().unwrap().contains(device_id) {
                tokio::time::sleep(d).await;
            } else {
                tokio::select! {
                _ = tokio::time::sleep(d) => {}
                _ = cancel.cancelled() => {
                    self.push(IoEvent::ConnectCancelled(device_id.to_string()));
                    return Err(AppError::Cancelled);
                }
                }
            }
            if let Some(msg) = self.connect_fail.lock().unwrap().get(device_id).cloned() {
                return Err(AppError::DeviceNotFound(msg));
            }
            self.links.lock().unwrap().insert(device_id.to_string(), LinkState::Alive);
            self.listeners_alive.fetch_add(1, Ordering::SeqCst);
            let guard = AliveGuard(self.listeners_alive.clone());
            let listener = tokio::spawn(async move {
                let _guard = guard;
                std::future::pending::<()>().await;
            });
            let info = info(device_id, ConnectionStatus::Connected);
            let trainer = (info.device_type == DeviceType::FitnessTrainer)
                .then(|| TrainerBackend::Fake(self.trainer_commands.clone()));
            Ok(Connected { info, listener: Some(listener), trainer })
        }

        async fn disconnect(&self, device_id: &str) -> Result<(), AppError> {
            self.push(IoEvent::Disconnect(device_id.to_string()));
            self.links.lock().unwrap().remove(device_id);
            Ok(())
        }

        async fn probe(&self, ids: Vec<String>) -> Vec<(String, LinkState)> {
            let links = self.links.lock().unwrap();
            ids.into_iter()
                .map(|id| {
                    let s = links.get(&id).copied().unwrap_or(LinkState::Lost);
                    (id, s)
                })
                .collect()
        }

        async fn await_data(&self, device_id: &str, timeout: Duration, cancel: &CancelToken) -> bool {
            let after = self.data_after.lock().unwrap().get(device_id).copied();
            let wait = after.unwrap_or(timeout).min(timeout);
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = cancel.cancelled() => return false,
            }
            match after {
                Some(a) if a <= timeout => {
                    self.links.lock().unwrap().insert(device_id.to_string(), LinkState::Alive);
                    true
                }
                _ => false,
            }
        }

        async fn ble_details(&self, device_id: &str) -> Result<DeviceDetails, AppError> {
            Err(AppError::DeviceNotFound(device_id.to_string()))
        }

        fn ant_metadata(&self) -> HashMap<String, AntDeviceMetadata> {
            HashMap::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(age_secs: Option<u64>, closed: bool) -> AntLink {
        let ts = Arc::new(AtomicI64::new(0));
        if let Some(age) = age_secs {
            super::super::ant::listener::atomic_set_age(&ts, Duration::from_secs(age));
        }
        AntLink { last_seen: ts, closed: Arc::new(AtomicBool::new(closed)) }
    }

    #[test]
    fn ant_link_missing_is_lost() {
        assert_eq!(ant_link_state(None), LinkState::Lost);
    }

    #[test]
    fn ant_link_closed_is_lost_even_if_fresh() {
        assert_eq!(ant_link_state(Some(&link(Some(0), true))), LinkState::Lost);
    }

    #[test]
    fn ant_link_no_page_yet_is_alive() {
        assert_eq!(ant_link_state(Some(&link(None, false))), LinkState::Alive);
    }

    #[test]
    fn ant_link_older_than_timeout_is_stale() {
        let age = config::DEVICE_DISCONNECT_TIMEOUT_SECS + 1;
        assert_eq!(ant_link_state(Some(&link(Some(age), false))), LinkState::Stale);
    }

    #[tokio::test]
    async fn cancel_token_wakes_waiter() {
        let t = CancelToken::new();
        let t2 = t.clone();
        let h = tokio::spawn(async move { t2.cancelled().await });
        tokio::task::yield_now().await;
        t.cancel();
        tokio::time::timeout(Duration::from_millis(50), h).await.unwrap().unwrap();
        assert!(t.is_cancelled());
    }

    #[tokio::test]
    async fn ant_probe_returns_while_ant_slot_locked() {
        let io = RealIo::new();
        io.insert_link("ant:120:1", link(Some(0), false));
        // A "scan" holds the AntSlot for 300 ms on another thread.
        let locked = io.hold_ant_slot_for(Duration::from_millis(300));
        locked.recv().unwrap();
        let states = tokio::time::timeout(
            Duration::from_millis(50),
            io.probe(vec!["ant:120:1".to_string()]),
        )
        .await
        .expect("probe must not wait for the AntSlot lock");
        assert_eq!(states, vec![("ant:120:1".to_string(), LinkState::Alive)]);
    }
}
