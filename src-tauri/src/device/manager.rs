use btleplug::api::Peripheral as _;
use log::{info, warn};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicI64;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use super::ant_manager::AntManager;
use super::ant_usb::AntUsb;
use super::ble::BleManager;
use super::dedup::compute_device_groups;
use super::fec::FecController;
use super::ftms::TrainerController;
use super::listener::listen_to_device;
use super::reconnect::ReconnectManager;
use super::types::*;
use crate::error::AppError;
use crate::session::storage::Storage;

enum TrainerBackend {
    Ftms(TrainerController),
    Fec { usb: Arc<AntUsb>, channel: u8 },
}

/// ANT+ staleness threshold: device considered disconnected after 10s without data
const ANT_STALE_SECS: u64 = 10;

/// Unified device manager wrapping BLE and ANT+ transports
pub struct DeviceManager {
    ble: Option<BleManager>,
    ant: Option<AntManager>,
    /// True if AntManager was ever successfully initialized (for panic recovery)
    ant_was_available: bool,
    /// True after a failed ANT+ USB probe; prevents repeated USB enumeration.
    /// Reset on successful ANT+ init or on user-initiated scan.
    ant_probe_failed: bool,
    trainer_backends: HashMap<String, TrainerBackend>,
    /// Tracks currently connected devices so rescanning doesn't lose them
    connected_devices: HashMap<String, DeviceInfo>,
    storage: Option<Arc<Storage>>,
    /// Cached ANT+ metadata store (survives take/put-back of AntManager)
    ant_metadata: Option<Arc<StdMutex<HashMap<String, AntDeviceMetadata>>>>,
    /// Lock-free ANT+ last-seen timestamps (survives take/put-back of AntManager)
    ant_last_seen: Option<Arc<StdMutex<HashMap<String, Arc<AtomicI64>>>>>,
    /// BLE listener task handles (keyed by device_id)
    listener_handles: HashMap<String, JoinHandle<()>>,
    /// Auto-reconnect engine for dropped devices
    reconnect: ReconnectManager,
}

impl DeviceManager {
    pub fn new() -> Self {
        Self {
            ble: None,
            ant: None,
            ant_was_available: false,
            ant_probe_failed: false,
            trainer_backends: HashMap::new(),
            connected_devices: HashMap::new(),
            storage: None,
            ant_metadata: None,
            ant_last_seen: None,
            listener_handles: HashMap::new(),
            reconnect: ReconnectManager::new(),
        }
    }

    pub fn set_storage(&mut self, storage: Arc<Storage>) {
        self.storage = Some(storage);
    }

    /// Set AntManager and cache its metadata store
    fn set_ant(&mut self, ant: Option<AntManager>) {
        if let Some(ref a) = ant {
            self.ant_metadata = Some(a.metadata_store());
            self.ant_last_seen = Some(a.last_seen_store());
            self.ant_was_available = true;
            self.ant_probe_failed = false;
        }
        self.ant = ant;
    }

    /// Ensure ANT+ is available, re-initializing if it was lost due to a panic.
    /// Skips USB enumeration if a previous probe already found no stick
    /// (the flag is reset on user-initiated scan via `scan_all()`).
    async fn ensure_ant(&mut self) {
        if self.ant.is_some() {
            return;
        }
        if self.ant_probe_failed {
            return;
        }
        if self.ant_was_available {
            warn!("ANT+ manager was lost (panic?), attempting re-initialization");
        }
        let ant = tokio::task::spawn_blocking(|| AntManager::try_new())
            .await
            .unwrap_or(None);
        if ant.is_none() {
            self.ant_probe_failed = true;
        }
        self.set_ant(ant);
    }

    /// Run a blocking closure with the AntManager, guaranteeing put-back even on panic.
    /// Returns Err if no AntManager is available or if spawn_blocking panics.
    async fn with_ant_blocking<F, R>(&mut self, f: F) -> Result<R, AppError>
    where
        F: FnOnce(&mut AntManager) -> R + Send + 'static,
        R: Send + 'static,
    {
        let mut ant = self
            .ant
            .take()
            .ok_or_else(|| AppError::AntPlus("No ANT+ USB stick found".into()))?;

        let result = tokio::task::spawn_blocking(move || {
            let r = f(&mut ant);
            (ant, r)
        })
        .await;

        match result {
            Ok((ant_back, r)) => {
                self.set_ant(Some(ant_back));
                Ok(r)
            }
            Err(e) => {
                // spawn_blocking panicked — AntManager is consumed.
                // ant is already None from take(); ant_was_available remains true
                // so ensure_ant() will reinit on next use.
                log::error!("[ant+] Blocking task panicked: {}", e);
                Err(AppError::AntPlus(format!("ANT+ task panicked: {}", e)))
            }
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
        for (id, info) in &self.connected_devices {
            devices.insert(id.clone(), info.clone());
        }
        // Annotate ANT+ devices with metadata from common data pages
        self.annotate_ant_metadata(&mut devices);

        // Compute cross-transport device groups
        let device_list: Vec<DeviceInfo> = devices.values().cloned().collect();
        let groups = compute_device_groups(&device_list);
        for (id, group_id) in &groups {
            if let Some(info) = devices.get_mut(id) {
                info.device_group = Some(group_id.clone());
            }
        }

        devices.into_values().collect()
    }

    /// Scan for devices on all available transports.
    /// Always includes currently-connected devices in the results.
    /// Loads known devices from storage as a base layer.
    /// BLE and ANT+ scans run concurrently to minimize total scan time.
    pub async fn scan_all(&mut self) -> Result<Vec<DeviceInfo>, AppError> {
        let mut discovered: HashMap<String, DeviceInfo> = HashMap::new();
        let mut scan_found: HashSet<String> = HashSet::new();

        // Load known devices from storage as base layer
        if let Some(ref storage) = self.storage {
            if let Ok(known) = storage.list_known_devices().await {
                for d in known {
                    discovered.insert(d.id.clone(), d);
                }
            }
        }

        // Initialize BLE on first scan
        if self.ble.is_none() {
            match BleManager::new().await {
                Ok(mgr) => self.ble = Some(mgr),
                Err(e) => log::warn!("[ble] Not available: {}", e),
            }
        }

        // Start BLE scan
        if let Some(ref ble) = self.ble {
            if let Err(e) = ble.start_scan().await {
                log::warn!("[ble] Scan start failed: {}", e);
            }
        }

        // Kick off ANT+ probe+scan concurrently while BLE scans.
        // User-initiated scan always retries ANT+ (reset probe failure cache).
        self.ant_probe_failed = false;
        let ant_taken = self.ant.take();
        let ant_task = tokio::task::spawn_blocking(move || {
            let ant = ant_taken.or_else(AntManager::try_new);
            if let Some(mut ant_mgr) = ant {
                let result = ant_mgr.scan();
                (Some(ant_mgr), result.ok())
            } else {
                (None, None)
            }
        });

        // Sleep during BLE scan (ANT+ runs concurrently on blocking thread)
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        // Collect BLE results
        if let Some(ref ble) = self.ble {
            let _ = ble.stop_scan().await;
            match ble.get_discovered_devices().await {
                Ok(devices) => {
                    for d in devices {
                        scan_found.insert(d.id.clone());
                        discovered.insert(d.id.clone(), d);
                    }
                }
                Err(e) => log::warn!("[ble] Discovery failed: {}", e),
            }
        }

        // Collect ANT+ results (may already be done if it finished during BLE sleep)
        match ant_task.await {
            Ok((ant_back, ant_devices)) => {
                if let Some(ant) = ant_back {
                    self.set_ant(Some(ant));
                } else {
                    self.ant_probe_failed = true;
                }
                if let Some(devices) = ant_devices {
                    for d in devices {
                        scan_found.insert(d.id.clone());
                        discovered.insert(d.id.clone(), d);
                    }
                }
            }
            Err(e) => {
                // spawn_blocking panicked — AntManager is lost, will reinit next scan
                log::error!("[ant+] Scan task panicked: {}", e);
            }
        }

        // Connected devices are always considered in range
        for id in self.connected_devices.keys() {
            scan_found.insert(id.clone());
        }

        // Merge: connected devices always appear (with Connected status),
        // plus any newly discovered devices not already connected
        for (id, info) in &self.connected_devices {
            discovered.insert(id.clone(), info.clone());
        }

        // Mark in_range based on whether the device was found in this scan
        for (id, info) in &mut discovered {
            info.in_range = scan_found.contains(id);
        }

        // Annotate ANT+ devices with metadata from common data pages
        self.annotate_ant_metadata(&mut discovered);

        // Compute cross-transport device groups
        let device_list: Vec<DeviceInfo> = discovered.values().cloned().collect();
        let groups = compute_device_groups(&device_list);
        for (id, group_id) in &groups {
            if let Some(info) = discovered.get_mut(id) {
                info.device_group = Some(group_id.clone());
            }
        }

        let result: Vec<DeviceInfo> = discovered.into_values().collect();

        // Persist discovered devices to storage (single transaction)
        if let Some(ref storage) = self.storage {
            if let Err(e) = storage.upsert_known_devices_batch(&result).await {
                log::warn!("Failed to batch-persist devices: {}", e);
            }
        }

        Ok(result)
    }

    /// Connect to a device by ID (routes to BLE or ANT+ based on ID prefix)
    pub async fn connect(
        &mut self,
        device_id: &str,
        tx: broadcast::Sender<SensorReading>,
    ) -> Result<DeviceInfo, AppError> {
        if device_id.starts_with("ant:") {
            self.connect_ant(device_id, tx).await
        } else {
            self.connect_ble(device_id, tx).await
        }
    }

    async fn connect_ble(
        &mut self,
        device_id: &str,
        tx: broadcast::Sender<SensorReading>,
    ) -> Result<DeviceInfo, AppError> {
        if self.ble.is_none() {
            match BleManager::new().await {
                Ok(mgr) => self.ble = Some(mgr),
                Err(e) => return Err(AppError::Ble(format!("BLE init failed: {}", e))),
            }
        }
        let ble = self.ble.as_ref().unwrap();
        let mut info = ble.connect_device(device_id).await?;

        // Read DIS metadata to populate manufacturer/model/serial
        if let Ok(details) = ble.get_device_details(device_id).await {
            info.manufacturer = details.manufacturer;
            info.model_number = details.model_number;
            info.serial_number = details.serial_number;
        }

        // If it's a trainer, create FTMS controller
        if info.device_type == DeviceType::FitnessTrainer {
            let connected = ble.get_connected();
            let connected_lock = connected.lock().await;
            if let Some(peripheral) = connected_lock.get(device_id) {
                if let Ok(controller) = TrainerController::new(peripheral.clone()) {
                    self.trainer_backends.insert(
                        device_id.to_string(),
                        TrainerBackend::Ftms(controller),
                    );
                    info!("[{}] FTMS trainer controller created", device_id);
                }
            }
        }

        // Spawn BLE notification listener (mirrors ANT+ which spawns in AntManager.connect)
        {
            let connected = ble.get_connected();
            let connected_lock = connected.lock().await;
            if let Some(peripheral) = connected_lock.get(device_id) {
                let peripheral = peripheral.clone();
                let device_type = info.device_type;
                let did = device_id.to_string();
                drop(connected_lock);

                let handle = tokio::spawn(async move {
                    listen_to_device(peripheral, device_type, tx, did).await;
                });
                self.listener_handles.insert(device_id.to_string(), handle);
            } else {
                warn!(
                    "[{}] Peripheral not found in connected map after connect",
                    device_id
                );
            }
        }

        self.connected_devices
            .insert(device_id.to_string(), info.clone());
        Ok(info)
    }

    async fn connect_ant(
        &mut self,
        device_id: &str,
        tx: broadcast::Sender<SensorReading>,
    ) -> Result<DeviceInfo, AppError> {
        self.ensure_ant().await;
        // If device isn't discovered yet, run a scan first
        {
            let needs_scan = self
                .ant
                .as_ref()
                .map(|a| !a.is_discovered(device_id))
                .unwrap_or(true);
            if needs_scan {
                self.with_ant_blocking(|ant| {
                    let _ = ant.scan();
                })
                .await?;
            }
        }

        let id = device_id.to_string();
        let info = self
            .with_ant_blocking(move |ant| ant.connect(&id, tx))
            .await??;

        // If it's a trainer, store FE-C backend
        if let Some(ref ant) = self.ant {
            if info.device_type == DeviceType::FitnessTrainer {
                if let Some((usb, channel)) = ant.get_fec_channel(device_id) {
                    self.trainer_backends.insert(
                        device_id.to_string(),
                        TrainerBackend::Fec { usb, channel },
                    );
                    info!("[{}] FE-C trainer controller created", device_id);
                }
            }
        }

        self.connected_devices
            .insert(device_id.to_string(), info.clone());
        Ok(info)
    }

    /// Disconnect a device
    pub async fn disconnect(&mut self, device_id: &str) -> Result<(), AppError> {
        if let Some(handle) = self.listener_handles.remove(device_id) {
            handle.abort();
        }
        self.trainer_backends.remove(device_id);
        self.connected_devices.remove(device_id);

        if device_id.starts_with("ant:") {
            if self.ant.is_some() {
                let id = device_id.to_string();
                self.with_ant_blocking(move |ant| ant.disconnect(&id))
                    .await??;
                Ok(())
            } else {
                Ok(())
            }
        } else {
            let ble = self
                .ble
                .as_ref()
                .ok_or_else(|| AppError::Ble("BLE not initialized".into()))?;
            ble.disconnect_device(device_id).await
        }
    }

    /// Check all connected devices and return IDs of any that have disconnected.
    /// Cleans up internal state (connected_devices, trainer_backends, BLE connected map).
    pub async fn check_connections(&mut self) -> Vec<DeviceInfo> {
        let mut disconnected = Vec::new();

        // Check BLE peripherals via is_connected()
        if let Some(ref ble) = self.ble {
            let connected_arc = ble.get_connected();

            // Collect peripherals to check, then drop the lock before async I/O
            let to_check: Vec<(String, btleplug::platform::Peripheral)> = {
                let connected = connected_arc.lock().await;
                self.connected_devices
                    .keys()
                    .filter(|id| !id.starts_with("ant:"))
                    .filter_map(|id| connected.get(id).map(|p| (id.clone(), p.clone())))
                    .collect()
            };

            for (id, peripheral) in to_check {
                if !peripheral.is_connected().await.unwrap_or(false) {
                    if let Some(info) = self.connected_devices.get(&id) {
                        disconnected.push(info.clone());
                    }
                }
            }

            // Remove from BLE connected map
            if !disconnected.is_empty() {
                let mut connected = connected_arc.lock().await;
                for info in &disconnected {
                    connected.remove(&info.id);
                }
            }
        }

        // Check ANT+ staleness via lock-free last-seen timestamps
        if let Some(ref last_seen_store) = self.ant_last_seen {
            let last_seen = last_seen_store.lock().unwrap_or_else(|e| e.into_inner());
            let ant_ids: Vec<String> = self
                .connected_devices
                .keys()
                .filter(|id| id.starts_with("ant:"))
                .cloned()
                .collect();
            for id in ant_ids {
                if let Some(ts) = last_seen.get(&id) {
                    if let Some(elapsed) = super::ant_listener::atomic_elapsed(ts) {
                        if elapsed > std::time::Duration::from_secs(ANT_STALE_SECS) {
                            if let Some(info) = self.connected_devices.get(&id) {
                                disconnected.push(info.clone());
                            }
                        }
                    }
                    // No timestamp yet (0) → just connected, give it time
                }
            }
        }

        // Clean up internal state for all disconnected devices
        for info in &disconnected {
            warn!("[{}] Connection watchdog: {:?} disconnected", info.id, info.device_type);
            self.connected_devices.remove(&info.id);
            self.trainer_backends.remove(&info.id);
            if let Some(handle) = self.listener_handles.remove(&info.id) {
                handle.abort();
            }
        }

        // Register disconnected devices for auto-reconnect
        for info in &disconnected {
            self.reconnect.register(info.clone());
        }

        disconnected
    }

    /// Attempt reconnects for devices due for retry.
    /// Returns (reconnected, still_trying) device infos.
    pub async fn attempt_reconnects(
        &mut self,
        tx: &broadcast::Sender<SensorReading>,
    ) -> (Vec<DeviceInfo>, Vec<(DeviceInfo, u32)>) {
        let due = self.reconnect.due_for_retry();
        let mut reconnected = Vec::new();
        let mut still_trying = Vec::new();

        for info in due {
            let attempt = self.reconnect.attempt_count(&info.id);
            match self.connect(&info.id, tx.clone()).await {
                Ok(new_info) => {
                    log::info!("[{}] Reconnected on attempt {}", info.id, attempt);
                    self.reconnect.remove(&info.id);
                    reconnected.push(new_info);
                }
                Err(e) => {
                    log::debug!(
                        "[{}] Reconnect attempt {} failed: {}",
                        info.id,
                        attempt,
                        e
                    );
                    still_trying.push((info, attempt));
                }
            }
        }

        (reconnected, still_trying)
    }

    pub fn clear_reconnect_target(&mut self, device_id: &str) {
        self.reconnect.remove(device_id);
    }

    pub fn clear_all_reconnect_targets(&mut self) {
        self.reconnect.clear();
    }

    // Trainer control methods -- C2: FE-C calls wrapped in spawn_blocking

    pub async fn set_target_power(&mut self, device_id: &str, watts: i16) -> Result<(), AppError> {
        match self.trainer_backends.get_mut(device_id) {
            Some(TrainerBackend::Ftms(controller)) => {
                controller.set_target_power(watts).await
            }
            Some(TrainerBackend::Fec { usb, channel }) => {
                let usb = usb.clone();
                let ch = *channel;
                let w = watts.max(0) as u16;
                tokio::task::spawn_blocking(move || {
                    let fec = FecController::new(&usb, ch);
                    fec.set_target_power(w)
                })
                .await
                .map_err(|e| AppError::AntPlus(format!("FEC task failed: {}", e)))?
            }
            None => Err(AppError::Session("No trainer connected".into())),
        }
    }

    pub async fn set_resistance(&mut self, device_id: &str, level: u8) -> Result<(), AppError> {
        match self.trainer_backends.get_mut(device_id) {
            Some(TrainerBackend::Ftms(controller)) => {
                controller.set_resistance(level).await
            }
            Some(TrainerBackend::Fec { usb, channel }) => {
                let usb = usb.clone();
                let ch = *channel;
                let lvl = level;
                tokio::task::spawn_blocking(move || {
                    let fec = FecController::new(&usb, ch);
                    fec.set_resistance(lvl)
                })
                .await
                .map_err(|e| AppError::AntPlus(format!("FEC task failed: {}", e)))?
            }
            None => Err(AppError::Session("No trainer connected".into())),
        }
    }

    pub async fn set_simulation(
        &mut self,
        device_id: &str,
        grade: f32,
        crr: f32,
        cw: f32,
    ) -> Result<(), AppError> {
        match self.trainer_backends.get_mut(device_id) {
            Some(TrainerBackend::Ftms(controller)) => {
                controller.set_simulation(grade, crr, cw).await
            }
            Some(TrainerBackend::Fec { usb, channel }) => {
                let usb = usb.clone();
                let ch = *channel;
                tokio::task::spawn_blocking(move || {
                    let fec = FecController::new(&usb, ch);
                    fec.set_simulation(grade, crr, cw)
                })
                .await
                .map_err(|e| AppError::AntPlus(format!("FEC task failed: {}", e)))?
            }
            None => Err(AppError::Session("No trainer connected".into())),
        }
    }

    pub async fn start_trainer(&mut self, device_id: &str) -> Result<(), AppError> {
        match self.trainer_backends.get_mut(device_id) {
            Some(TrainerBackend::Ftms(controller)) => controller.start().await,
            Some(TrainerBackend::Fec { .. }) => {
                Err(AppError::AntPlus("Start/stop not supported for ANT+ trainers".into()))
            }
            None => Err(AppError::Session("No trainer connected".into())),
        }
    }

    pub async fn stop_trainer(&mut self, device_id: &str) -> Result<(), AppError> {
        match self.trainer_backends.get_mut(device_id) {
            Some(TrainerBackend::Ftms(controller)) => controller.stop().await,
            Some(TrainerBackend::Fec { .. }) => {
                Err(AppError::AntPlus("Start/stop not supported for ANT+ trainers".into()))
            }
            None => Err(AppError::Session("No trainer connected".into())),
        }
    }

    /// Get detailed information about a connected device
    pub async fn get_device_details(&self, device_id: &str) -> Result<DeviceDetails, AppError> {
        if device_id.starts_with("ant:") {
            let info = self.connected_devices.get(device_id)
                .ok_or_else(|| AppError::DeviceNotFound(device_id.to_string()))?;

            // Get metadata from ANT+ Common Data Pages if available
            let meta = self.ant.as_ref().and_then(|ant| ant.get_metadata(device_id));

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
        } else {
            let ble = self.ble.as_ref()
                .ok_or_else(|| AppError::Ble("BLE not initialized".into()))?;
            ble.get_device_details(device_id).await
        }
    }

    /// Annotate ANT+ devices with metadata from common data pages.
    fn annotate_ant_metadata(&self, devices: &mut HashMap<String, DeviceInfo>) {
        if let Some(ref meta_store) = self.ant_metadata {
            let meta = meta_store.lock().unwrap_or_else(|e| e.into_inner());
            for (id, info) in devices.iter_mut() {
                if id.starts_with("ant:") {
                    if let Some(m) = meta.get(id) {
                        if info.manufacturer.is_none() {
                            info.manufacturer = m.manufacturer_id.map(ant_manufacturer_name);
                        }
                        if info.model_number.is_none() {
                            info.model_number = m.model_number.map(|n| n.to_string());
                        }
                        if info.serial_number.is_none() {
                            info.serial_number =
                                m.serial_number.filter(|&s| s != 0).map(|n| n.to_string());
                        }
                    }
                }
            }
        }
    }

    /// Get the connected trainer device ID (for command routing).
    /// Cross-references trainer_backends with connected_devices to return
    /// only a trainer that is actually Connected, avoiding stale entries
    /// left behind during reconnect.
    pub fn connected_trainer_id(&self) -> Option<String> {
        self.trainer_backends
            .keys()
            .find(|id| {
                self.connected_devices
                    .get(*id)
                    .is_some_and(|info| info.status == ConnectionStatus::Connected)
            })
            .cloned()
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
