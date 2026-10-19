use super::usb::*;
use crate::device::types::DeviceType;
use crate::error::{AntError, AppError};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const ANTPLUS_NETWORK_KEY: [u8; 8] = [0xB9, 0xA5, 0x21, 0xFB, 0xBD, 0x72, 0xC3, 0x45];
const NETWORK_NUMBER: u8 = 0;

/// ANT+ profile configuration
#[derive(Debug, Clone, Copy)]
pub struct AntProfile {
    pub device_type_id: u8,
    pub channel_period: u16,
    pub rf_frequency: u8,
    pub device_type: DeviceType,
}

pub const PROFILE_HR: AntProfile = AntProfile {
    device_type_id: 120,
    channel_period: 8070,
    rf_frequency: 57,
    device_type: DeviceType::HeartRate,
};

pub const PROFILE_POWER: AntProfile = AntProfile {
    device_type_id: 11,
    channel_period: 8182,
    rf_frequency: 57,
    device_type: DeviceType::Power,
};

pub const PROFILE_CADENCE: AntProfile = AntProfile {
    device_type_id: 122,
    channel_period: 8102,
    rf_frequency: 57,
    device_type: DeviceType::CadenceSpeed,
};

pub const PROFILE_SPEED: AntProfile = AntProfile {
    device_type_id: 123,
    channel_period: 8118,
    rf_frequency: 57,
    device_type: DeviceType::CadenceSpeed,
};

pub const PROFILE_FEC: AntProfile = AntProfile {
    device_type_id: 17,
    channel_period: 8192,
    rf_frequency: 57,
    device_type: DeviceType::FitnessTrainer,
};

/// Combined bike speed & cadence sensor: device type 121, period 8086, RF 57
/// (ANT+ Bike Speed and Cadence profile; ANT's nRF Connect SDK `ant_bsc.h`).
pub const PROFILE_SPEED_CADENCE: AntProfile = AntProfile {
    device_type_id: 121,
    channel_period: 8086,
    rf_frequency: 57,
    device_type: DeviceType::CadenceSpeed,
};

pub const ALL_SCAN_PROFILES: &[AntProfile] = &[
    PROFILE_HR,
    PROFILE_POWER,
    PROFILE_CADENCE,
    PROFILE_SPEED,
    PROFILE_SPEED_CADENCE,
    PROFILE_FEC,
];

/// Channels on an ANT USB2 / ANTUSB-m stick (datasheets D00001367, D00001513).
pub const MAX_CHANNELS: u8 = 8;

/// Maximum entries in a channel's inclusion/exclusion list (D00000652 §9.5.2.10).
pub const MAX_ID_LIST: usize = 4;

/// Lowest channel number not used by a connection.
pub fn free_channel(used: &std::collections::HashSet<u8>) -> Option<u8> {
    (0..MAX_CHANNELS).find(|ch| !used.contains(ch))
}

/// Split the scan profiles over the channels connections aren't using. With fewer
/// free channels than profiles, profiles are scanned in successive batches.
pub fn scan_batches(free: &[u8], profiles: &[AntProfile]) -> Vec<Vec<(u8, AntProfile)>> {
    if free.is_empty() {
        return vec![];
    }
    profiles
        .chunks(free.len())
        .map(|chunk| free.iter().copied().zip(chunk.iter().copied()).collect())
        .collect()
}

/// Channel ID found by a wildcard channel (device number, device type, transmission type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoundId {
    pub device_number: u16,
    pub device_type: u8,
    pub transmission_type: u8,
}

/// Messages that make a (closed, still assigned) slave channel ignore `excluded`:
/// one Add Channel ID to List (0x59, §9.5.2.10) per entry, then Config ID List
/// (0x5A, §9.5.2.12) with exclude = 1.
pub fn exclusion_messages(ch: u8, excluded: &[FoundId]) -> Vec<AntMessage> {
    let mut msgs: Vec<AntMessage> = excluded
        .iter()
        .take(MAX_ID_LIST)
        .enumerate()
        .map(|(i, id)| {
            let dn = id.device_number.to_le_bytes();
            AntMessage {
                msg_id: MSG_ADD_CHANNEL_ID_TO_LIST,
                data: vec![ch, dn[0], dn[1], id.device_type, id.transmission_type, i as u8],
            }
        })
        .collect();
    msgs.push(AntMessage {
        msg_id: MSG_CONFIG_ID_LIST,
        data: vec![ch, excluded.len().min(MAX_ID_LIST) as u8, 1],
    });
    msgs
}

/// Config ID List with size 0: disables the channel's inclusion/exclusion list.
/// Best effort: only the multi-device scan depends on ID lists, so a stick that
/// rejects 0x5A must not break opening or releasing channels.
fn reset_id_list(usb: &dyn AntTx, ch: u8, response_queue: &Arc<Mutex<Vec<AntMessage>>>) {
    let result = usb
        .send(&AntMessage { msg_id: MSG_CONFIG_ID_LIST, data: vec![ch, 0, 0] })
        .and_then(|_| poll_response(response_queue, ch, MSG_CONFIG_ID_LIST));
    if let Err(e) = result {
        log::warn!("ANT+ ch{}: resetting the ID list failed (continuing): {}", ch, e);
    }
}

/// Per-channel data senders for the router, tagged with a generation so a late
/// EVENT_CHANNEL_CLOSED from an earlier close cannot kill a sender registered after
/// that close was issued (channel numbers are reused).
#[derive(Default)]
pub struct ChannelBook {
    senders: HashMap<u8, (u64, std::sync::mpsc::Sender<Vec<u8>>)>,
    /// Generation stamped when we issued a close on the channel.
    close_issued: HashMap<u8, u64>,
    next_gen: u64,
}

impl ChannelBook {
    fn bump(&mut self) -> u64 {
        self.next_gen += 1;
        self.next_gen
    }

    pub fn register(&mut self, ch: u8) -> std::sync::mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let gen = self.bump();
        self.senders.insert(ch, (gen, tx));
        rx
    }

    pub fn unregister(&mut self, ch: u8) {
        self.senders.remove(&ch);
    }

    /// Record that we are about to close `ch` (call right before sending the close).
    pub fn note_close_issued(&mut self, ch: u8) {
        let gen = self.bump();
        self.close_issued.insert(ch, gen);
    }

    pub fn clear(&mut self) {
        self.senders.clear();
    }

    /// Router step: broadcast pages go to the channel's sender (returns None);
    /// everything else is returned for the response queue. A close/search-timeout
    /// event drops the sender unless that sender is newer than the close we issued.
    pub fn route(&mut self, msg: AntMessage) -> Option<AntMessage> {
        if msg.msg_id == MSG_BROADCAST_DATA && msg.data.len() >= 9 {
            if let Some((_, sender)) = self.senders.get(&msg.data[0]) {
                // A send error means the listener is gone; nothing to do.
                let _ = sender.send(msg.data[1..9].to_vec());
            }
            return None;
        }
        if let Some(ch) = closed_channel_event(&msg) {
            let issued = if msg.data[2] == EVENT_CHANNEL_CLOSED { self.close_issued.remove(&ch) } else { None };
            let sender_gen = self.senders.get(&ch).map(|(g, _)| *g);
            match (sender_gen, issued) {
                (Some(g), Some(close_gen)) if g > close_gen => {
                    log::debug!("ANT+ ch{}: late close event ignored (channel re-registered)", ch);
                }
                (Some(_), _) => {
                    self.senders.remove(&ch);
                    log::warn!("ANT+ ch{} closed by the radio", ch);
                }
                (None, _) => {}
            }
        }
        Some(msg)
    }
}

/// Something that can transmit ANT messages (the USB stick, or a test recorder).
pub trait AntTx {
    fn send(&self, msg: &AntMessage) -> Result<(), AppError>;
}

impl AntTx for AntUsb {
    fn send(&self, msg: &AntMessage) -> Result<(), AppError> {
        AntUsb::send(self, msg)
    }
}

/// Search timeouts in 2.5 s units (D00000652 Rev 5.1 §9.5.2.5 high priority,
/// §9.5.2.15 low priority; 0 = disabled, 0xFF = infinite).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchConfig {
    pub high_priority: u8,
    /// `None` leaves the chip default (2 = 5 s).
    pub low_priority: Option<u8>,
}

/// When false, connection channels use HP = 0 from the start (design ADR-2).
pub const ANT_FAST_ACQUIRE: bool = false;

/// Connection channels: never time out and never run high-priority search, which
/// would interrupt other open channels (§9.5.2.15). A lost device is re-acquired by
/// the background low-priority search on the same channel.
pub const CONNECTION_SEARCH: SearchConfig = SearchConfig {
    high_priority: if ANT_FAST_ACQUIRE { 2 } else { 0 },
    low_priority: Some(0xFF),
};

/// Scan (wildcard) channels: short finite search; the scan window is 4 s.
pub const SCAN_SEARCH: SearchConfig = SearchConfig { high_priority: 2, low_priority: None };

/// How long a connect waits for the first data page before giving up.
pub const ANT_FIRST_PAGE_TIMEOUT: Duration = Duration::from_secs(5);

/// Represents a configured ANT channel
#[derive(Debug)]
pub struct AntChannelConfig {
    pub channel_number: u8,
    pub profile: AntProfile,
    pub device_number: u16,    // 0 = wildcard (scanning)
    pub transmission_type: u8, // 0 = wildcard
}

/// Initialize the ANT stick: reset + set network key.
/// Called before the router thread starts, so reads directly from USB.
pub fn init_ant_stick(usb: &AntUsb) -> Result<(), AppError> {
    // System reset
    usb.send(&AntMessage {
        msg_id: MSG_SYSTEM_RESET,
        data: vec![0x00],
    })?;
    std::thread::sleep(Duration::from_millis(500));

    // Drain any startup messages
    loop {
        let msgs = usb.receive_all()?;
        if msgs.is_empty() {
            break;
        }
    }

    // Set ANT+ network key on network 0
    let mut key_data = vec![NETWORK_NUMBER];
    key_data.extend_from_slice(&ANTPLUS_NETWORK_KEY);
    usb.send(&AntMessage {
        msg_id: MSG_SET_NETWORK_KEY,
        data: key_data,
    })?;
    wait_for_response_direct(usb, MSG_SET_NETWORK_KEY)?;

    Ok(())
}

/// Open a channel for the given configuration.
/// Uses the response_queue (router must be running).
/// If the channel is in a bad state (e.g. leftover from a previous scan),
/// automatically closes+unassigns and retries.
pub fn open_channel(
    usb: &dyn AntTx,
    config: &AntChannelConfig,
    search: SearchConfig,
    response_queue: &Arc<Mutex<Vec<AntMessage>>>,
) -> Result<(), AppError> {
    let ch = config.channel_number;

    // Assign channel (slave/receive)
    usb.send(&AntMessage {
        msg_id: MSG_ASSIGN_CHANNEL,
        data: vec![ch, CHANNEL_TYPE_SLAVE, NETWORK_NUMBER],
    })?;

    if let Err(_) = poll_response(response_queue, ch, MSG_ASSIGN_CHANNEL) {
        // Channel likely in wrong state — force close + unassign and retry
        log::info!("[ant+ ch{}] Assign failed, resetting channel state", ch);
        let _ = usb.send(&AntMessage {
            msg_id: MSG_CLOSE_CHANNEL,
            data: vec![ch],
        });
        std::thread::sleep(Duration::from_millis(200));
        // Drain close-related responses
        {
            let mut queue = response_queue.lock().unwrap_or_else(|e| e.into_inner());
            queue.retain(|msg| {
                !(msg.msg_id == MSG_CHANNEL_RESPONSE && msg.data.first() == Some(&ch))
            });
        }
        let _ = usb.send(&AntMessage {
            msg_id: MSG_UNASSIGN_CHANNEL,
            data: vec![ch],
        });
        std::thread::sleep(Duration::from_millis(100));
        // Drain unassign response
        {
            let mut queue = response_queue.lock().unwrap_or_else(|e| e.into_inner());
            queue.retain(|msg| {
                !(msg.msg_id == MSG_CHANNEL_RESPONSE && msg.data.first() == Some(&ch))
            });
        }

        // Retry assign
        usb.send(&AntMessage {
            msg_id: MSG_ASSIGN_CHANNEL,
            data: vec![ch, CHANNEL_TYPE_SLAVE, NETWORK_NUMBER],
        })?;
        poll_response(response_queue, ch, MSG_ASSIGN_CHANNEL)?;
    }

    // A reused channel number must never inherit a previous scan's exclusion list.
    reset_id_list(usb, ch, response_queue);

    // Set channel ID (device number, device type, transmission type)
    let dn = config.device_number.to_le_bytes();
    usb.send(&AntMessage {
        msg_id: MSG_SET_CHANNEL_ID,
        data: vec![
            ch,
            dn[0],
            dn[1],
            config.profile.device_type_id,
            config.transmission_type,
        ],
    })?;
    poll_response(response_queue, ch, MSG_SET_CHANNEL_ID)?;

    // Set channel period
    let period = config.profile.channel_period.to_le_bytes();
    usb.send(&AntMessage {
        msg_id: MSG_SET_CHANNEL_PERIOD,
        data: vec![ch, period[0], period[1]],
    })?;
    poll_response(response_queue, ch, MSG_SET_CHANNEL_PERIOD)?;

    // Set RF frequency
    usb.send(&AntMessage {
        msg_id: MSG_SET_CHANNEL_FREQUENCY,
        data: vec![ch, config.profile.rf_frequency],
    })?;
    poll_response(response_queue, ch, MSG_SET_CHANNEL_FREQUENCY)?;

    // High-priority search timeout (2.5 s units)
    usb.send(&AntMessage {
        msg_id: MSG_SET_CHANNEL_SEARCH_TIMEOUT,
        data: vec![ch, search.high_priority],
    })?;
    poll_response(response_queue, ch, MSG_SET_CHANNEL_SEARCH_TIMEOUT)?;

    // Low-priority search timeout (2.5 s units, 0xFF = infinite)
    if let Some(lp) = search.low_priority {
        usb.send(&AntMessage {
            msg_id: MSG_SET_LP_SEARCH_TIMEOUT,
            data: vec![ch, lp],
        })?;
        poll_response(response_queue, ch, MSG_SET_LP_SEARCH_TIMEOUT)?;
    }

    // Open channel
    usb.send(&AntMessage {
        msg_id: MSG_OPEN_CHANNEL,
        data: vec![ch],
    })?;
    poll_response(response_queue, ch, MSG_OPEN_CHANNEL)?;

    Ok(())
}

/// Close and unassign a channel.
/// Uses the response_queue (router must be running).
pub fn close_channel(
    usb: &dyn AntTx,
    channel_number: u8,
    response_queue: &Arc<Mutex<Vec<AntMessage>>>,
) -> Result<(), AppError> {
    close_only(usb, channel_number, response_queue)?;
    usb.send(&AntMessage {
        msg_id: MSG_UNASSIGN_CHANNEL,
        data: vec![channel_number],
    })?;
    poll_response(response_queue, channel_number, MSG_UNASSIGN_CHANNEL)?;
    Ok(())
}

/// Close a channel and wait (≤ 2 s) for EVENT_CHANNEL_CLOSED; it stays assigned.
fn close_only(
    usb: &dyn AntTx,
    channel_number: u8,
    response_queue: &Arc<Mutex<Vec<AntMessage>>>,
) -> Result<(), AppError> {
    usb.send(&AntMessage {
        msg_id: MSG_CLOSE_CHANNEL,
        data: vec![channel_number],
    })?;
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        {
            let mut queue = response_queue.lock().unwrap_or_else(|e| e.into_inner());
            // Consume the command response too, so it can't pile up in the queue.
            queue.retain(|m| {
                !(m.msg_id == MSG_CHANNEL_RESPONSE
                    && m.data.len() >= 3
                    && m.data[0] == channel_number
                    && m.data[1] == MSG_CLOSE_CHANNEL)
            });
            if let Some(pos) = queue.iter().position(|msg| {
                msg.msg_id == MSG_CHANNEL_RESPONSE
                    && msg.data.len() >= 3
                    && msg.data[0] == channel_number
                    && msg.data[1] == CHANNEL_EVENT
                    && msg.data[2] == EVENT_CHANNEL_CLOSED
            }) {
                queue.remove(pos);
                return Ok(());
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

/// Re-open a wildcard scan channel so it ignores the devices already found on it.
pub fn reopen_excluding(
    usb: &dyn AntTx,
    ch: u8,
    excluded: &[FoundId],
    response_queue: &Arc<Mutex<Vec<AntMessage>>>,
) -> Result<(), AppError> {
    close_only(usb, ch, response_queue)?;
    for msg in exclusion_messages(ch, excluded) {
        usb.send(&msg)?;
        poll_response(response_queue, ch, msg.msg_id)?;
    }
    usb.send(&AntMessage { msg_id: MSG_OPEN_CHANNEL, data: vec![ch] })?;
    poll_response(response_queue, ch, MSG_OPEN_CHANNEL)
}

/// Release a scan channel: close, reset its exclusion list (while still assigned),
/// then unassign. Only after this may the channel number be reused.
pub fn release_scan_channel(
    usb: &dyn AntTx,
    ch: u8,
    response_queue: &Arc<Mutex<Vec<AntMessage>>>,
) -> Result<(), AppError> {
    close_only(usb, ch, response_queue)?;
    reset_id_list(usb, ch, response_queue);
    usb.send(&AntMessage { msg_id: MSG_UNASSIGN_CHANNEL, data: vec![ch] })?;
    poll_response(response_queue, ch, MSG_UNASSIGN_CHANNEL)
}

/// Wait for the first data page on a freshly opened channel.
/// Returns `None` on timeout or cancellation (checked at least every 50 ms).
pub fn wait_first_page(
    rx: &std::sync::mpsc::Receiver<Vec<u8>>,
    timeout: Duration,
    cancel: &std::sync::atomic::AtomicBool,
) -> Option<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    loop {
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        let slice = (deadline - now).min(Duration::from_millis(50));
        match rx.recv_timeout(slice) {
            Ok(page) if page.len() >= 8 => return Some(page),
            Ok(_) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// If `msg` is a channel event meaning the radio closed the channel by itself
/// (search timeout or channel closed, §9.5.6.1), return the channel number.
pub fn closed_channel_event(msg: &AntMessage) -> Option<u8> {
    if msg.msg_id == MSG_CHANNEL_RESPONSE
        && msg.data.len() >= 3
        && msg.data[1] == CHANNEL_EVENT
        && (msg.data[2] == EVENT_RX_SEARCH_TIMEOUT || msg.data[2] == EVENT_CHANNEL_CLOSED)
    {
        Some(msg.data[0])
    } else {
        None
    }
}

/// Per-attempt wait for an acknowledged transfer result. A slave's acknowledged
/// message goes out right after the next master message (§9.5.5.2): one FE-C channel
/// period is 250 ms (8192/32768 s), so 600 ms covers two periods.
pub const ACK_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(600);

/// Outcome of one acknowledged transfer, read from the response queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AckOutcome {
    Completed,
    Failed(&'static str),
}

/// True for queue entries that report the result of an acknowledged transfer on `ch`.
fn ack_result(msg: &AntMessage, ch: u8) -> Option<AckOutcome> {
    if msg.msg_id != MSG_CHANNEL_RESPONSE || msg.data.len() < 3 || msg.data[0] != ch {
        return None;
    }
    match (msg.data[1], msg.data[2]) {
        (CHANNEL_EVENT, EVENT_TRANSFER_TX_COMPLETED) => Some(AckOutcome::Completed),
        (CHANNEL_EVENT, EVENT_TRANSFER_TX_FAILED) => Some(AckOutcome::Failed("no acknowledgement (TX_FAILED)")),
        // The channel dropped to search: the message was not sent (§9.5.5.2).
        (CHANNEL_EVENT, EVENT_RX_FAIL_GO_TO_SEARCH) => Some(AckOutcome::Failed("trainer lost (GO_TO_SEARCH)")),
        (MSG_ACKNOWLEDGED_DATA, TRANSFER_IN_PROGRESS) => Some(AckOutcome::Failed("transfer in progress")),
        (MSG_ACKNOWLEDGED_DATA, code) if code != RESPONSE_NO_ERROR => Some(AckOutcome::Failed("rejected by stick")),
        _ => None,
    }
}

fn ack_attempt(
    tx: &dyn AntTx,
    queue: &Arc<Mutex<Vec<AntMessage>>>,
    channel_number: u8,
    data: &[u8; 8],
    timeout: Duration,
) -> Result<AckOutcome, AppError> {
    // Drop results of earlier transfers on this channel so they can't be mistaken for ours.
    queue
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|m| ack_result(m, channel_number).is_none());

    let mut msg_data = vec![channel_number];
    msg_data.extend_from_slice(data);
    tx.send(&AntMessage { msg_id: MSG_ACKNOWLEDGED_DATA, data: msg_data })?;

    let deadline = Instant::now() + timeout;
    loop {
        {
            let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(pos) = q.iter().position(|m| ack_result(m, channel_number).is_some()) {
                let msg = q.remove(pos);
                if let Some(outcome) = ack_result(&msg, channel_number) {
                    return Ok(outcome);
                }
            }
        }
        if Instant::now() >= deadline {
            return Ok(AckOutcome::Failed("timed out waiting for TX_COMPLETED"));
        }
        std::thread::sleep(Duration::from_millis(2).min(timeout));
    }
}

/// Send an acknowledged data page and wait for the transfer result
/// (EVENT_TRANSFER_TX_COMPLETED / TX_FAILED, D00000652 Rev 5.1 §9.5.5.2: ANT does not
/// resend failed data). A failed attempt is retried once, unless `trainer_fresh()`
/// says the trainer has gone quiet, in which case the error is returned at once.
pub fn send_acknowledged_and_wait(
    tx: &dyn AntTx,
    queue: &Arc<Mutex<Vec<AntMessage>>>,
    channel_number: u8,
    data: &[u8; 8],
    attempt_timeout: Duration,
    trainer_fresh: &dyn Fn() -> bool,
) -> Result<(), AppError> {
    let first = ack_attempt(tx, queue, channel_number, data, attempt_timeout)?;
    let reason = match first {
        AckOutcome::Completed => return Ok(()),
        AckOutcome::Failed(r) => r,
    };
    if !trainer_fresh() {
        return Err(AntError::Channel(format!(
            "FE-C page 0x{:02X} on ch{} failed ({}); trainer stale, not retrying",
            data[0], channel_number, reason
        ))
        .into());
    }
    log::debug!("FE-C page 0x{:02X} on ch{} failed ({}); retrying once", data[0], channel_number, reason);
    match ack_attempt(tx, queue, channel_number, data, attempt_timeout)? {
        AckOutcome::Completed => Ok(()),
        AckOutcome::Failed(r) => Err(AntError::Channel(format!(
            "FE-C page 0x{:02X} on ch{} failed after retry: {}",
            data[0], channel_number, r
        ))
        .into()),
    }
}

/// Poll the response queue for a channel response to the expected command.
/// Filters by both channel number and message ID to avoid cross-channel confusion.
/// Used after the router thread is running.
pub fn poll_response(
    response_queue: &Arc<Mutex<Vec<AntMessage>>>,
    channel_number: u8,
    expected_msg_id: u8,
) -> Result<(), AppError> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        {
            let mut queue = response_queue.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(pos) = queue.iter().position(|msg| {
                msg.msg_id == MSG_CHANNEL_RESPONSE
                    && msg.data.len() >= 3
                    && msg.data[0] == channel_number
                    && msg.data[1] == expected_msg_id
            }) {
                let msg = queue.remove(pos);
                let code = msg.data[2];
                if code == RESPONSE_NO_ERROR {
                    return Ok(());
                } else {
                    return Err(AntError::Channel(format!(
                        "ANT ch{} command {:#x} failed with code {:#x}",
                        channel_number, expected_msg_id, code
                    )).into());
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(AntError::Channel(format!(
        "Timeout waiting for ch{} response to {:#x}",
        channel_number, expected_msg_id
    )).into())
}

/// Wait for a channel response by reading directly from USB.
/// Used only during init_ant_stick (before the router thread starts).
fn wait_for_response_direct(usb: &AntUsb, expected_msg_id: u8) -> Result<(), AppError> {
    for _ in 0..50 {
        let messages = usb.receive_all()?;
        for msg in messages {
            if msg.msg_id == MSG_CHANNEL_RESPONSE && msg.data.len() >= 3 {
                let response_to = msg.data[1];
                let code = msg.data[2];
                if response_to == expected_msg_id {
                    if code == RESPONSE_NO_ERROR {
                        return Ok(());
                    } else {
                        return Err(AntError::Channel(format!(
                            "ANT command {:#x} failed with code {:#x}",
                            expected_msg_id, code
                        )).into());
                    }
                }
            }
        }
    }
    Err(AntError::Channel(format!(
        "Timeout waiting for response to {:#x}",
        expected_msg_id
    )).into())
}

#[cfg(test)]
pub mod test_stick {
    //! Records sent messages and answers like a stick (RESPONSE_NO_ERROR for every
    //! configuration command, EVENT_CHANNEL_CLOSED after a close).
    use super::*;

    pub struct RecordingStick {
        pub sent: Mutex<Vec<AntMessage>>,
        pub queue: Arc<Mutex<Vec<AntMessage>>>,
        /// Commands answered with an error code (e.g. a stick without ID lists).
        pub reject: Option<u8>,
    }

    impl RecordingStick {
        pub fn new() -> Self {
            Self { sent: Mutex::new(Vec::new()), queue: Arc::new(Mutex::new(Vec::new())), reject: None }
        }
        pub fn rejecting(msg_id: u8) -> Self {
            Self { reject: Some(msg_id), ..Self::new() }
        }
        pub fn sent(&self) -> Vec<(u8, Vec<u8>)> {
            self.sent.lock().unwrap().iter().map(|m| (m.msg_id, m.data.clone())).collect()
        }
    }

    impl AntTx for RecordingStick {
        fn send(&self, msg: &AntMessage) -> Result<(), AppError> {
            self.sent.lock().unwrap().push(msg.clone());
            let ch = msg.data.first().copied().unwrap_or(0);
            let mut q = self.queue.lock().unwrap();
            if msg.msg_id == MSG_CLOSE_CHANNEL {
                q.push(AntMessage { msg_id: MSG_CHANNEL_RESPONSE, data: vec![ch, MSG_CLOSE_CHANNEL, 0] });
                q.push(AntMessage {
                    msg_id: MSG_CHANNEL_RESPONSE,
                    data: vec![ch, CHANNEL_EVENT, EVENT_CHANNEL_CLOSED],
                });
            } else if msg.msg_id != MSG_ACKNOWLEDGED_DATA && msg.msg_id != MSG_REQUEST_MESSAGE {
                let code = if self.reject == Some(msg.msg_id) { 0x28 /* INVALID_MESSAGE */ } else { RESPONSE_NO_ERROR };
                q.push(AntMessage { msg_id: MSG_CHANNEL_RESPONSE, data: vec![ch, msg.msg_id, code] });
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_stick::RecordingStick;
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn hr_config(ch: u8) -> AntChannelConfig {
        AntChannelConfig {
            channel_number: ch,
            profile: PROFILE_HR,
            device_number: 0x1234,
            transmission_type: 1,
        }
    }

    #[test]
    fn ant_connection_channel_search_config_bytes() {
        let stick = RecordingStick::new();
        open_channel(&stick, &hr_config(3), CONNECTION_SEARCH, &stick.queue).unwrap();
        let sent = stick.sent();
        // D00000652 §9.5.2.5: HP timeout 0 = HP search disabled;
        // §9.5.2.15: LP timeout 0xFF = infinite background search
        assert!(sent.contains(&(MSG_SET_CHANNEL_SEARCH_TIMEOUT, vec![3, 0])), "{:?}", sent);
        assert!(sent.contains(&(MSG_SET_LP_SEARCH_TIMEOUT, vec![3, 0xFF])), "{:?}", sent);
        // Channel ID: device 0x1234 LE, HR device type 120, transmission type 1
        assert!(sent.contains(&(MSG_SET_CHANNEL_ID, vec![3, 0x34, 0x12, 120, 1])));
        assert_eq!(sent.last(), Some(&(MSG_OPEN_CHANNEL, vec![3])));
    }

    #[test]
    fn scan_channel_uses_finite_search_and_chip_default_lp() {
        let stick = RecordingStick::new();
        open_channel(&stick, &hr_config(0), SCAN_SEARCH, &stick.queue).unwrap();
        let sent = stick.sent();
        assert!(sent.contains(&(MSG_SET_CHANNEL_SEARCH_TIMEOUT, vec![0, 2])));
        assert!(!sent.iter().any(|(id, _)| *id == MSG_SET_LP_SEARCH_TIMEOUT));
    }

    #[test]
    fn wait_first_page_returns_first_page() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(vec![0x84, 0, 0, 0, 0, 0, 0, 142]).unwrap();
        let page = wait_first_page(&rx, Duration::from_millis(20), &AtomicBool::new(false));
        assert_eq!(page, Some(vec![0x84, 0, 0, 0, 0, 0, 0, 142]));
    }

    #[test]
    fn wait_first_page_times_out_without_data() {
        let (_tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let start = Instant::now();
        assert_eq!(wait_first_page(&rx, Duration::from_millis(20), &AtomicBool::new(false)), None);
        assert!(start.elapsed() < Duration::from_millis(250));
    }

    #[test]
    fn wait_first_page_stops_on_cancel() {
        let (_tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let start = Instant::now();
        assert_eq!(wait_first_page(&rx, Duration::from_secs(5), &AtomicBool::new(true)), None);
        assert!(start.elapsed() < Duration::from_millis(250));
    }

    #[test]
    fn closed_channel_event_detects_search_timeout_and_close() {
        let ev = |code| AntMessage { msg_id: MSG_CHANNEL_RESPONSE, data: vec![4, CHANNEL_EVENT, code] };
        assert_eq!(closed_channel_event(&ev(EVENT_RX_SEARCH_TIMEOUT)), Some(4));
        assert_eq!(closed_channel_event(&ev(EVENT_CHANNEL_CLOSED)), Some(4));
        assert_eq!(closed_channel_event(&ev(EVENT_RX_FAIL_GO_TO_SEARCH)), None, "still searching");
        // A command response whose code byte happens to be 0x07 is not an event
        let resp = AntMessage { msg_id: MSG_CHANNEL_RESPONSE, data: vec![4, MSG_CLOSE_CHANNEL, 0x07] };
        assert_eq!(closed_channel_event(&resp), None);
    }

    // ---- FE-C acknowledged sends (D00000652 Rev 5.1 §9.5.5.2) ----

    use std::collections::VecDeque;

    /// Answers each acknowledged send with the next scripted channel response.
    struct AckStick {
        script: Mutex<VecDeque<Option<AntMessage>>>,
        queue: Arc<Mutex<Vec<AntMessage>>>,
        sends: Mutex<usize>,
    }

    fn event(ch: u8, code: u8) -> Option<AntMessage> {
        Some(AntMessage { msg_id: MSG_CHANNEL_RESPONSE, data: vec![ch, CHANNEL_EVENT, code] })
    }

    impl AckStick {
        fn new(script: Vec<Option<AntMessage>>) -> Self {
            Self {
                script: Mutex::new(script.into()),
                queue: Arc::new(Mutex::new(Vec::new())),
                sends: Mutex::new(0),
            }
        }
        fn sends(&self) -> usize {
            *self.sends.lock().unwrap()
        }
    }

    impl AntTx for AckStick {
        fn send(&self, msg: &AntMessage) -> Result<(), AppError> {
            assert_eq!(msg.msg_id, MSG_ACKNOWLEDGED_DATA);
            *self.sends.lock().unwrap() += 1;
            if let Some(Some(reply)) = self.script.lock().unwrap().pop_front() {
                self.queue.lock().unwrap().push(reply);
            }
            Ok(())
        }
    }

    const PAGE: [u8; 8] = [0x31, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x20, 0x03];
    const T: Duration = Duration::from_millis(10);
    const FRESH: &dyn Fn() -> bool = &|| true;

    #[test]
    fn ack_wait_succeeds_on_tx_completed() {
        let stick = AckStick::new(vec![event(5, EVENT_TRANSFER_TX_COMPLETED)]);
        send_acknowledged_and_wait(&stick, &stick.queue, 5, &PAGE, T, FRESH).unwrap();
        assert_eq!(stick.sends(), 1);
    }

    #[test]
    fn ack_wait_retries_once_on_tx_failed_then_succeeds() {
        let stick = AckStick::new(vec![event(5, EVENT_TRANSFER_TX_FAILED), event(5, EVENT_TRANSFER_TX_COMPLETED)]);
        send_acknowledged_and_wait(&stick, &stick.queue, 5, &PAGE, T, FRESH).unwrap();
        assert_eq!(stick.sends(), 2);
    }

    #[test]
    fn ack_wait_errors_after_two_failures() {
        let stick = AckStick::new(vec![event(5, EVENT_TRANSFER_TX_FAILED), event(5, EVENT_TRANSFER_TX_FAILED)]);
        assert!(send_acknowledged_and_wait(&stick, &stick.queue, 5, &PAGE, T, FRESH).is_err());
        assert_eq!(stick.sends(), 2, "exactly one retry");
    }

    #[test]
    fn ack_wait_times_out_within_bound() {
        let stick = AckStick::new(vec![None, None]);
        let start = Instant::now();
        assert!(send_acknowledged_and_wait(&stick, &stick.queue, 5, &PAGE, T, FRESH).is_err());
        assert!(start.elapsed() < Duration::from_millis(250), "{:?}", start.elapsed());
        assert_eq!(stick.sends(), 2);
    }

    #[test]
    fn ack_wait_ignores_other_channel_events() {
        let stick = AckStick::new(vec![event(4, EVENT_TRANSFER_TX_COMPLETED), None]);
        assert!(send_acknowledged_and_wait(&stick, &stick.queue, 5, &PAGE, T, FRESH).is_err());
        assert_eq!(stick.queue.lock().unwrap().len(), 1, "channel 4's event left for its owner");
    }

    #[test]
    fn ack_wait_treats_go_to_search_as_failure() {
        let stick = AckStick::new(vec![event(5, EVENT_RX_FAIL_GO_TO_SEARCH), event(5, EVENT_TRANSFER_TX_COMPLETED)]);
        send_acknowledged_and_wait(&stick, &stick.queue, 5, &PAGE, T, FRESH).unwrap();
        assert_eq!(stick.sends(), 2, "GO_TO_SEARCH means not sent: retried");
    }

    #[test]
    fn ack_wait_transfer_in_progress_is_failure() {
        let busy = Some(AntMessage { msg_id: MSG_CHANNEL_RESPONSE, data: vec![5, MSG_ACKNOWLEDGED_DATA, TRANSFER_IN_PROGRESS] });
        let stick = AckStick::new(vec![busy, event(5, EVENT_TRANSFER_TX_COMPLETED)]);
        send_acknowledged_and_wait(&stick, &stick.queue, 5, &PAGE, T, FRESH).unwrap();
        assert_eq!(stick.sends(), 2);
    }

    #[test]
    fn ack_retry_skipped_when_trainer_stale() {
        let stick = AckStick::new(vec![event(5, EVENT_TRANSFER_TX_FAILED), event(5, EVENT_TRANSFER_TX_COMPLETED)]);
        let stale: &dyn Fn() -> bool = &|| false;
        assert!(send_acknowledged_and_wait(&stick, &stick.queue, 5, &PAGE, T, stale).is_err());
        assert_eq!(stick.sends(), 1, "no retry for a stale trainer");
    }

    #[test]
    fn ack_wait_purges_stale_tx_result_before_sending() {
        let stick = AckStick::new(vec![None, None]);
        // A completion left over from an earlier transfer must not count for this one.
        stick.queue.lock().unwrap().push(event(5, EVENT_TRANSFER_TX_COMPLETED).unwrap());
        assert!(send_acknowledged_and_wait(&stick, &stick.queue, 5, &PAGE, T, FRESH).is_err());
    }

    // ---- Scan channels, exclusion lists, combined S&C ----

    #[test]
    fn profile_speed_cadence_is_type_121_period_8086() {
        assert_eq!(PROFILE_SPEED_CADENCE.device_type_id, 121);
        assert_eq!(PROFILE_SPEED_CADENCE.channel_period, 8086);
        assert_eq!(PROFILE_SPEED_CADENCE.rf_frequency, 57);
    }

    #[test]
    fn scan_profiles_include_combined_sc() {
        assert!(ALL_SCAN_PROFILES.iter().any(|p| p.device_type_id == 121));
    }

    #[test]
    fn scan_channel_allocation_skips_connection_channels() {
        let used: std::collections::HashSet<u8> = [0, 1, 3].into_iter().collect();
        assert_eq!(free_channel(&used), Some(2));
        let all: std::collections::HashSet<u8> = (0..MAX_CHANNELS).collect();
        assert_eq!(free_channel(&all), None);
        let free: Vec<u8> = (0..MAX_CHANNELS).filter(|c| !used.contains(c)).collect();
        let batches = scan_batches(&free, ALL_SCAN_PROFILES);
        assert_eq!(batches.len(), 2, "5 free channels for 6 profiles → two batches");
        assert!(batches.iter().flatten().all(|(ch, _)| !used.contains(ch)));
    }

    #[test]
    fn scan_profiles_batched_when_channels_scarce() {
        let batches = scan_batches(&[6, 7], ALL_SCAN_PROFILES);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].iter().map(|(c, _)| *c).collect::<Vec<_>>(), vec![6, 7]);
        let types: Vec<u8> = batches.iter().flatten().map(|(_, p)| p.device_type_id).collect();
        assert_eq!(types, vec![120, 11, 122, 123, 121, 17], "every profile scanned once");
        assert!(scan_batches(&[], ALL_SCAN_PROFILES).is_empty());
    }

    #[test]
    fn exclusion_list_messages_bytes() {
        let found = [
            FoundId { device_number: 0x1234, device_type: 120, transmission_type: 1 },
            FoundId { device_number: 0x00AB, device_type: 120, transmission_type: 5 },
        ];
        let msgs: Vec<(u8, Vec<u8>)> = exclusion_messages(2, &found).into_iter().map(|m| (m.msg_id, m.data)).collect();
        assert_eq!(
            msgs,
            vec![
                // 0x59: [ch, dev# LSB, dev# MSB, device type, trans type, list index]
                (MSG_ADD_CHANNEL_ID_TO_LIST, vec![2, 0x34, 0x12, 120, 1, 0]),
                (MSG_ADD_CHANNEL_ID_TO_LIST, vec![2, 0xAB, 0x00, 120, 5, 1]),
                // 0x5A: [ch, list size, exclude = 1]
                (MSG_CONFIG_ID_LIST, vec![2, 2, 1]),
            ]
        );
    }

    #[test]
    fn exclusion_list_capped_at_four_entries() {
        let found: Vec<FoundId> = (1..=6)
            .map(|n| FoundId { device_number: n, device_type: 11, transmission_type: 1 })
            .collect();
        let msgs = exclusion_messages(0, &found);
        assert_eq!(msgs.len(), MAX_ID_LIST + 1);
        assert_eq!(msgs.last().unwrap().data, vec![0, 4, 1]);
    }

    #[test]
    fn scan_channel_release_sends_close_reset_list_unassign_in_order() {
        let stick = RecordingStick::new();
        release_scan_channel(&stick, 2, &stick.queue).unwrap();
        assert_eq!(
            stick.sent(),
            vec![
                (MSG_CLOSE_CHANNEL, vec![2]),
                (MSG_CONFIG_ID_LIST, vec![2, 0, 0]),
                (MSG_UNASSIGN_CHANNEL, vec![2]),
            ]
        );
    }

    #[test]
    fn open_channel_resets_id_list_after_assign() {
        let stick = RecordingStick::new();
        open_channel(&stick, &hr_config(1), SCAN_SEARCH, &stick.queue).unwrap();
        let sent = stick.sent();
        assert_eq!(sent[0].0, MSG_ASSIGN_CHANNEL);
        assert_eq!(sent[1], (MSG_CONFIG_ID_LIST, vec![1, 0, 0]));
    }

    #[test]
    fn reopen_excluding_closes_lists_then_opens() {
        let stick = RecordingStick::new();
        let found = [FoundId { device_number: 7, device_type: 121, transmission_type: 1 }];
        reopen_excluding(&stick, 3, &found, &stick.queue).unwrap();
        let ids: Vec<u8> = stick.sent().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec![MSG_CLOSE_CHANNEL, MSG_ADD_CHANNEL_ID_TO_LIST, MSG_CONFIG_ID_LIST, MSG_OPEN_CHANNEL]);
    }

    // ---- Router channel generations / best-effort ID lists ----

    fn closed_event(ch: u8) -> AntMessage {
        AntMessage { msg_id: MSG_CHANNEL_RESPONSE, data: vec![ch, CHANNEL_EVENT, EVENT_CHANNEL_CLOSED] }
    }

    fn broadcast(ch: u8, page: [u8; 8]) -> AntMessage {
        let mut data = vec![ch];
        data.extend_from_slice(&page);
        AntMessage { msg_id: MSG_BROADCAST_DATA, data }
    }

    #[test]
    fn late_close_event_does_not_kill_recovered_channel() {
        let mut book = ChannelBook::default();
        let _old = book.register(3); // previous connection on ch3
        book.unregister(3);
        book.note_close_issued(3); // close sent; its event is delayed
        let rx = book.register(3); // reconnect reuses ch3
        assert!(book.route(closed_event(3)).is_some(), "event still reaches the response queue");
        book.route(broadcast(3, [0x84, 0, 0, 0, 0, 0, 0, 140]));
        // The recovery connect's first-page wait succeeds the first time.
        let page = wait_first_page(&rx, Duration::from_millis(250), &AtomicBool::new(false));
        assert_eq!(page.map(|p| p[7]), Some(140));
    }

    #[test]
    fn unsolicited_close_event_drops_sender() {
        let mut book = ChannelBook::default();
        let rx = book.register(2);
        book.route(closed_event(2));
        assert!(matches!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected)));
    }

    #[test]
    fn close_event_for_sender_older_than_close_drops_it() {
        let mut book = ChannelBook::default();
        let rx = book.register(4);
        book.note_close_issued(4);
        book.route(closed_event(4));
        assert!(matches!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected)));
    }

    #[test]
    fn open_channel_succeeds_when_stick_rejects_id_list() {
        let stick = RecordingStick::rejecting(MSG_CONFIG_ID_LIST);
        open_channel(&stick, &hr_config(1), CONNECTION_SEARCH, &stick.queue).unwrap();
        assert_eq!(stick.sent().last(), Some(&(MSG_OPEN_CHANNEL, vec![1])));
    }

    #[test]
    fn release_scan_channel_unassigns_when_stick_rejects_id_list() {
        let stick = RecordingStick::rejecting(MSG_CONFIG_ID_LIST);
        release_scan_channel(&stick, 2, &stick.queue).unwrap();
        assert_eq!(stick.sent().last(), Some(&(MSG_UNASSIGN_CHANNEL, vec![2])));
    }

    #[test]
    fn reopen_excluding_reports_unsupported_id_list() {
        let stick = RecordingStick::rejecting(MSG_ADD_CHANNEL_ID_TO_LIST);
        let found = [FoundId { device_number: 7, device_type: 120, transmission_type: 1 }];
        assert!(reopen_excluding(&stick, 0, &found, &stick.queue).is_err(), "scan falls back on this error");
    }
}
