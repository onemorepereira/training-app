use log::{info, warn};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use super::ant_channel::*;
use super::ant_listener::listen_ant_channel;
use super::ant_usb::*;
use super::types::*;
use crate::error::AppError;

/// Information about a discovered ANT+ device
#[derive(Debug, Clone)]
struct DiscoveredDevice {
    device_number: u16,
    transmission_type: u8,
    profile: AntProfile,
}

/// An active ANT+ connection
struct ActiveConnection {
    channel_number: u8,
    profile: AntProfile,
    #[allow(dead_code)]
    device_number: u16,
    stop_flag: Arc<AtomicBool>,
    listener_handle: Option<JoinHandle<()>>,
}

/// Maximum channels on an ANT+ USB stick
const MAX_CHANNELS: u8 = 8;

/// Manages ANT+ devices via USB stick.
/// Uses a single router thread that reads all USB messages and dispatches
/// broadcast data to per-channel mpsc senders.
pub struct AntManager {
    usb: Arc<AntUsb>,
    router_stop: Arc<AtomicBool>,
    router_handle: Option<std::thread::JoinHandle<()>>,
    channel_senders: Arc<Mutex<HashMap<u8, std::sync::mpsc::Sender<Vec<u8>>>>>,
    response_queue: Arc<Mutex<Vec<AntMessage>>>,
    discovered: HashMap<String, DiscoveredDevice>,
    connected: HashMap<String, ActiveConnection>,
    /// Metadata from ANT+ Common Data Pages, keyed by device_id
    device_metadata: Arc<Mutex<HashMap<String, AntDeviceMetadata>>>,
}

impl AntManager {
    /// Try to initialize ANT+ (returns None if no USB stick found).
    /// Opens the USB stick, initializes it, and starts the router thread.
    pub fn try_new() -> Option<Self> {
        if !AntUsb::is_available() {
            info!("No ANT+ USB stick detected");
            return None;
        }

        let usb = match AntUsb::open() {
            Ok(usb) => usb,
            Err(e) => {
                warn!("Failed to open ANT+ stick: {}", e);
                return None;
            }
        };

        // init_ant_stick reads directly from USB (router not started yet)
        if let Err(e) = init_ant_stick(&usb) {
            warn!("Failed to initialize ANT+ stick: {}", e);
            return None;
        }

        let usb = Arc::new(usb);
        let router_stop = Arc::new(AtomicBool::new(false));
        let channel_senders: Arc<Mutex<HashMap<u8, std::sync::mpsc::Sender<Vec<u8>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let response_queue: Arc<Mutex<Vec<AntMessage>>> = Arc::new(Mutex::new(Vec::new()));

        // Start the router thread
        let router_handle = {
            let usb = usb.clone();
            let stop = router_stop.clone();
            let senders = channel_senders.clone();
            let queue = response_queue.clone();

            std::thread::spawn(move || {
                router_loop(usb, senders, queue, stop);
            })
        };

        info!("ANT+ USB stick initialized with router thread");
        Some(Self {
            usb,
            router_stop,
            router_handle: Some(router_handle),
            channel_senders,
            response_queue,
            discovered: HashMap::new(),
            connected: HashMap::new(),
            device_metadata: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Find the lowest free channel number above the scan-reserved range.
    /// Channels 0..N are reserved for scanning (one per profile).
    fn allocate_channel(&self) -> Result<u8, AppError> {
        let reserved = ALL_SCAN_PROFILES.len() as u8;
        let used: std::collections::HashSet<u8> = self.connected.values().map(|c| c.channel_number).collect();
        for ch in reserved..MAX_CHANNELS {
            if !used.contains(&ch) {
                return Ok(ch);
            }
        }
        Err(AppError::AntPlus(format!(
            "All ANT+ channels in use ({} connected, {} reserved for scanning)",
            used.len(),
            reserved
        )))
    }

    /// Scan for ANT+ devices. Opens wildcard channels for each profile,
    /// listens for a few seconds, then closes them.
    /// Must be called from a blocking context (spawn_blocking).
    pub fn scan(&mut self) -> Result<Vec<DeviceInfo>, AppError> {
        // Don't clear discovered — merge new results into existing.
        // Previously discovered devices persist across scans.

        // Clean up scan channels from any previous scan that didn't fully close.
        // Try close + unassign on each; ignore errors (channels may already be idle).
        for i in 0..ALL_SCAN_PROFILES.len() {
            let ch = i as u8;
            let _ = self.usb.send(&AntMessage {
                msg_id: MSG_CLOSE_CHANNEL,
                data: vec![ch],
            });
            std::thread::sleep(Duration::from_millis(50));
            let _ = self.usb.send(&AntMessage {
                msg_id: MSG_UNASSIGN_CHANNEL,
                data: vec![ch],
            });
            std::thread::sleep(Duration::from_millis(50));
        }
        // Drain any leftover responses from cleanup
        {
            let mut queue = self.response_queue.lock().unwrap();
            queue.clear();
        }

        // Open wildcard channels for each scannable profile
        let scan_channels: Vec<(u8, AntProfile)> = ALL_SCAN_PROFILES
            .iter()
            .enumerate()
            .map(|(i, profile)| {
                let ch = i as u8;
                let config = AntChannelConfig {
                    channel_number: ch,
                    profile: *profile,
                    device_number: 0,     // wildcard
                    transmission_type: 0, // wildcard
                };
                if let Err(e) = open_channel(&self.usb, &config, &self.response_queue) {
                    warn!(
                        "Failed to open scan channel {} for {:?}: {}",
                        ch, profile.device_type, e
                    );
                }
                (ch, *profile)
            })
            .collect();

        // Register temporary senders for scan channels so router delivers broadcast data
        let scan_receivers: Vec<(u8, std::sync::mpsc::Receiver<Vec<u8>>)> = {
            let mut senders = self.channel_senders.lock().unwrap();
            scan_channels
                .iter()
                .map(|(ch, _)| {
                    let (tx, rx) = std::sync::mpsc::channel();
                    senders.insert(*ch, tx);
                    (*ch, rx)
                })
                .collect()
        };

        // Listen for broadcasts for 4 seconds
        let scan_end = Instant::now() + Duration::from_secs(4);
        while Instant::now() < scan_end {
            // Check scan channel receivers for broadcast data (to trigger Channel ID requests)
            for (ch, rx) in &scan_receivers {
                if rx.try_recv().is_ok() {
                    // Got broadcast data on this channel, request Channel ID
                    let _ = self.usb.send(&AntMessage {
                        msg_id: MSG_REQUEST_MESSAGE,
                        data: vec![*ch, MSG_CHANNEL_ID],
                    });
                }
            }

            // Check the response queue for Channel ID responses
            {
                let mut queue = self.response_queue.lock().unwrap();
                let mut i = 0;
                while i < queue.len() {
                    let msg = &queue[i];
                    if msg.msg_id == MSG_CHANNEL_ID && msg.data.len() >= 5 {
                        let channel = msg.data[0] as usize;
                        let device_number = u16::from_le_bytes([msg.data[1], msg.data[2]]);
                        let device_type_id = msg.data[3];
                        let transmission_type = msg.data[4];

                        if device_number != 0 && channel < scan_channels.len() {
                            let profile = scan_channels[channel].1;
                            // I4: Include device type in ANT+ device ID for uniqueness
                            let id = format!("ant:{}:{}", device_type_id, device_number);
                            if !self.discovered.contains_key(&id) {
                                self.discovered.insert(
                                    id,
                                    DiscoveredDevice {
                                        device_number,
                                        transmission_type,
                                        profile,
                                    },
                                );
                            }
                        }
                        queue.remove(i);
                    } else {
                        i += 1;
                    }
                }
            }

            std::thread::sleep(Duration::from_millis(50));
        }

        // Remove scan senders from router
        {
            let mut senders = self.channel_senders.lock().unwrap();
            for (ch, _) in &scan_channels {
                senders.remove(ch);
            }
        }

        // Close scan channels
        for (ch, _) in &scan_channels {
            let _ = close_channel(&self.usb, *ch, &self.response_queue);
        }

        // Build device info list
        let devices: Vec<DeviceInfo> = self
            .discovered
            .iter()
            .map(|(id, dev)| DeviceInfo {
                id: id.clone(),
                name: Some(format!(
                    "ANT+ {:?} {}",
                    dev.profile.device_type, dev.device_number
                )),
                device_type: dev.profile.device_type,
                status: ConnectionStatus::Disconnected,
                transport: Transport::AntPlus,
                rssi: None,
                battery_level: None,
                last_seen: Some(chrono::Utc::now().to_rfc3339()),
                manufacturer: None,
                model_number: None,
                serial_number: None,
                device_group: None,
            })
            .collect();

        info!(
            "ANT+ scan complete: {} devices found, {} channels in use",
            devices.len(),
            self.connected.len()
        );
        Ok(devices)
    }

    /// Connect to a discovered ANT+ device.
    /// Spawns a listener task via spawn_blocking internally.
    pub fn connect(
        &mut self,
        device_id: &str,
        tx: broadcast::Sender<SensorReading>,
    ) -> Result<DeviceInfo, AppError> {
        let discovered = self
            .discovered
            .get(device_id)
            .ok_or_else(|| AppError::DeviceNotFound(device_id.to_string()))?
            .clone();

        let channel_number = self.allocate_channel()?;

        let config = AntChannelConfig {
            channel_number,
            profile: discovered.profile,
            device_number: discovered.device_number,
            transmission_type: discovered.transmission_type,
        };
        open_channel(&self.usb, &config, &self.response_queue)?;

        // Create mpsc channel for this device and register with router
        let (data_tx, data_rx) = std::sync::mpsc::channel();
        {
            let mut senders = self.channel_senders.lock().unwrap();
            senders.insert(channel_number, data_tx);
        }

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_clone = stop_flag.clone();
        let device_type = discovered.profile.device_type;
        let dtype_id = discovered.profile.device_type_id;
        let did = device_id.to_string();
        let metadata = self.device_metadata.clone();

        let listener_handle = tokio::task::spawn_blocking(move || {
            listen_ant_channel(data_rx, device_type, tx, stop_clone, did, metadata, dtype_id);
        });

        let info = DeviceInfo {
            id: device_id.to_string(),
            name: Some(format!(
                "ANT+ {:?} {}",
                discovered.profile.device_type, discovered.device_number
            )),
            device_type: discovered.profile.device_type,
            status: ConnectionStatus::Connected,
            transport: Transport::AntPlus,
            rssi: None,
            battery_level: None,
            last_seen: Some(chrono::Utc::now().to_rfc3339()),
            manufacturer: None,
            model_number: None,
            serial_number: None,
            device_group: None,
        };

        self.connected.insert(
            device_id.to_string(),
            ActiveConnection {
                channel_number,
                profile: discovered.profile,
                device_number: discovered.device_number,
                stop_flag,
                listener_handle: Some(listener_handle),
            },
        );

        Ok(info)
    }

    /// Disconnect from an ANT+ device
    pub fn disconnect(&mut self, device_id: &str) -> Result<(), AppError> {
        if let Some(mut conn) = self.connected.remove(device_id) {
            // Signal the listener to stop
            conn.stop_flag.store(true, Ordering::Relaxed);

            // Remove the sender from the router so the listener's receiver disconnects
            {
                let mut senders = self.channel_senders.lock().unwrap();
                senders.remove(&conn.channel_number);
            }

            if let Some(handle) = conn.listener_handle.take() {
                handle.abort();
            }
            close_channel(&self.usb, conn.channel_number, &self.response_queue)?;
        }
        Ok(())
    }

    pub fn is_discovered(&self, device_id: &str) -> bool {
        self.discovered.contains_key(device_id)
    }

    /// Get decoded common-page metadata for a connected ANT+ device
    pub fn get_metadata(&self, device_id: &str) -> Option<AntDeviceMetadata> {
        let meta = self.device_metadata.lock().unwrap_or_else(|e| e.into_inner());
        meta.get(device_id).cloned()
    }

    /// Get a clone of the metadata store Arc (for the connection watchdog)
    pub fn metadata_store(&self) -> Arc<Mutex<HashMap<String, AntDeviceMetadata>>> {
        self.device_metadata.clone()
    }

    /// Get the USB handle and channel number for a connected FE-C device (for trainer control)
    pub fn get_fec_channel(&self, device_id: &str) -> Option<(Arc<AntUsb>, u8)> {
        let conn = self.connected.get(device_id)?;
        if conn.profile.device_type != DeviceType::FitnessTrainer {
            return None;
        }
        Some((self.usb.clone(), conn.channel_number))
    }
}

impl Drop for AntManager {
    fn drop(&mut self) {
        // Stop the router thread
        self.router_stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.router_handle.take() {
            let _ = handle.join();
        }

        // Stop all listener threads
        for (_, conn) in &self.connected {
            conn.stop_flag.store(true, Ordering::Relaxed);
        }

        // Remove all channel senders to unblock listeners
        {
            let mut senders = self.channel_senders.lock().unwrap();
            senders.clear();
        }
    }
}

/// The router loop: reads all messages from USB and dispatches them.
/// - Broadcast data (MSG_BROADCAST_DATA): extract channel + 8-byte data page, send to per-channel mpsc
/// - Everything else (responses, Channel IDs, etc.): push to response_queue
fn router_loop(
    usb: Arc<AntUsb>,
    channel_senders: Arc<Mutex<HashMap<u8, std::sync::mpsc::Sender<Vec<u8>>>>>,
    response_queue: Arc<Mutex<Vec<AntMessage>>>,
    stop: Arc<AtomicBool>,
) {
    info!("ANT+ router thread started");

    let mut consecutive_errors = 0u32;
    const MAX_CONSECUTIVE_ERRORS: u32 = 10;

    while !stop.load(Ordering::Relaxed) {
        let messages = match usb.receive_all() {
            Ok(msgs) => {
                consecutive_errors = 0;
                msgs
            }
            Err(e) => {
                consecutive_errors += 1;
                warn!(
                    "ANT+ router USB error ({}/{}): {}",
                    consecutive_errors, MAX_CONSECUTIVE_ERRORS, e
                );
                if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                    warn!("ANT+ router: too many consecutive USB errors, exiting");
                    break;
                }
                let backoff =
                    std::time::Duration::from_millis((consecutive_errors as u64 * 100).min(1000));
                std::thread::sleep(backoff);
                continue;
            }
        };

        for msg in messages {
            if msg.msg_id == MSG_BROADCAST_DATA && msg.data.len() >= 9 {
                // Broadcast data: byte 0 = channel, bytes 1-8 = data page
                let channel = msg.data[0];
                let page_data = msg.data[1..9].to_vec();

                let senders = channel_senders.lock().unwrap();
                if let Some(sender) = senders.get(&channel) {
                    // If send fails, the receiver is gone (disconnected); just ignore
                    let _ = sender.send(page_data);
                }
            } else {
                // Channel responses, Channel IDs, etc. go to the response queue
                let mut queue = response_queue.lock().unwrap();
                queue.push(msg);

                // Prevent unbounded growth: keep only the most recent 256 responses
                if queue.len() > 256 {
                    let excess = queue.len() - 256;
                    queue.drain(..excess);
                }
            }
        }
    }

    info!("ANT+ router thread stopped");
}
