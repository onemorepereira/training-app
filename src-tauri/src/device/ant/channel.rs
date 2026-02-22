use super::usb::*;
use crate::device::types::DeviceType;
use crate::error::{AntError, AppError};
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

pub const ALL_SCAN_PROFILES: &[AntProfile] =
    &[PROFILE_HR, PROFILE_POWER, PROFILE_CADENCE, PROFILE_SPEED, PROFILE_FEC];

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
    usb: &AntUsb,
    config: &AntChannelConfig,
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

    // Set search timeout (30 seconds = 30 * 2.5s units = 12)
    usb.send(&AntMessage {
        msg_id: MSG_SET_CHANNEL_SEARCH_TIMEOUT,
        data: vec![ch, 12],
    })?;
    poll_response(response_queue, ch, MSG_SET_CHANNEL_SEARCH_TIMEOUT)?;

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
    usb: &AntUsb,
    channel_number: u8,
    response_queue: &Arc<Mutex<Vec<AntMessage>>>,
) -> Result<(), AppError> {
    usb.send(&AntMessage {
        msg_id: MSG_CLOSE_CHANNEL,
        data: vec![channel_number],
    })?;

    // Wait for channel closed event
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        {
            let mut queue = response_queue.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(pos) = queue.iter().position(|msg| {
                msg.msg_id == MSG_CHANNEL_RESPONSE
                    && msg.data.len() >= 3
                    && msg.data[0] == channel_number
                    && msg.data[2] == EVENT_CHANNEL_CLOSED
            }) {
                queue.remove(pos);
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    usb.send(&AntMessage {
        msg_id: MSG_UNASSIGN_CHANNEL,
        data: vec![channel_number],
    })?;
    poll_response(response_queue, channel_number, MSG_UNASSIGN_CHANNEL)?;

    Ok(())
}

/// Send an acknowledged data message on a channel (used for FE-C control)
pub fn send_acknowledged(
    usb: &AntUsb,
    channel_number: u8,
    data: &[u8; 8],
) -> Result<(), AppError> {
    let mut msg_data = vec![channel_number];
    msg_data.extend_from_slice(data);
    usb.send(&AntMessage {
        msg_id: MSG_ACKNOWLEDGED_DATA,
        data: msg_data,
    })
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
