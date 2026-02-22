use rusb::{DeviceHandle, GlobalContext};
use std::time::Duration;

use crate::error::AppError;

const GARMIN_VENDOR_ID: u16 = 0x0FCF;
const ANTUSB_M_PRODUCT_ID: u16 = 0x1009;
const ANTUSB_2_PRODUCT_ID: u16 = 0x1008;

const ANT_SYNC: u8 = 0xA4;
const USB_TIMEOUT: Duration = Duration::from_millis(1000);
const READ_TIMEOUT: Duration = Duration::from_millis(100);

// ANT message IDs
pub const MSG_SYSTEM_RESET: u8 = 0x4A;
pub const MSG_SET_NETWORK_KEY: u8 = 0x46;
pub const MSG_ASSIGN_CHANNEL: u8 = 0x42;
pub const MSG_SET_CHANNEL_ID: u8 = 0x51;
pub const MSG_SET_CHANNEL_PERIOD: u8 = 0x43;
pub const MSG_SET_CHANNEL_FREQUENCY: u8 = 0x45;
pub const MSG_SET_CHANNEL_SEARCH_TIMEOUT: u8 = 0x44;
pub const MSG_OPEN_CHANNEL: u8 = 0x4B;
pub const MSG_CLOSE_CHANNEL: u8 = 0x4C;
pub const MSG_UNASSIGN_CHANNEL: u8 = 0x41;
pub const MSG_REQUEST_MESSAGE: u8 = 0x4D;
pub const MSG_BROADCAST_DATA: u8 = 0x4E;
pub const MSG_ACKNOWLEDGED_DATA: u8 = 0x4F;
pub const MSG_CHANNEL_RESPONSE: u8 = 0x40;
pub const MSG_CHANNEL_ID: u8 = 0x51;

// Channel types
pub const CHANNEL_TYPE_SLAVE: u8 = 0x00; // Receive

// Channel response event codes
pub const EVENT_CHANNEL_CLOSED: u8 = 0x07;
pub const RESPONSE_NO_ERROR: u8 = 0x00;

/// A decoded ANT message
#[derive(Debug, Clone)]
pub struct AntMessage {
    pub msg_id: u8,
    pub data: Vec<u8>,
}

/// Low-level USB driver for ANT sticks.
/// Thread safety: libusb is thread-safe for concurrent operations on different
/// endpoints, so read_bulk (router thread) and write_bulk (scan/connect thread)
/// can run in parallel without a lock.
pub struct AntUsb {
    handle: DeviceHandle<GlobalContext>,
    endpoint_in: u8,
    endpoint_out: u8,
}

impl AntUsb {
    /// Find and open the first ANT USB stick
    pub fn open() -> Result<Self, AppError> {
        let devices = rusb::devices()
            .map_err(|e| AppError::AntPlus(format!("Failed to enumerate USB: {}", e)))?;

        for device in devices.iter() {
            let desc = device
                .device_descriptor()
                .map_err(|e| AppError::AntPlus(format!("Failed to read descriptor: {}", e)))?;

            if desc.vendor_id() == GARMIN_VENDOR_ID
                && (desc.product_id() == ANTUSB_M_PRODUCT_ID
                    || desc.product_id() == ANTUSB_2_PRODUCT_ID)
            {
                let handle = device
                    .open()
                    .map_err(|e| AppError::AntPlus(format!("Failed to open ANT stick: {}", e)))?;

                // Detach kernel driver if attached
                if handle.kernel_driver_active(0).unwrap_or(false) {
                    handle.detach_kernel_driver(0).map_err(|e| {
                        AppError::AntPlus(format!("Failed to detach kernel driver: {}", e))
                    })?;
                }

                handle.claim_interface(0).map_err(|e| {
                    AppError::AntPlus(format!("Failed to claim interface: {}", e))
                })?;

                // Find bulk endpoints
                let config = device
                    .active_config_descriptor()
                    .map_err(|e| AppError::AntPlus(format!("Failed to get config: {}", e)))?;
                let interface = config
                    .interfaces()
                    .next()
                    .ok_or_else(|| AppError::AntPlus("No interfaces found".into()))?;
                let setting = interface
                    .descriptors()
                    .next()
                    .ok_or_else(|| AppError::AntPlus("No interface descriptors".into()))?;

                let mut ep_in = 0u8;
                let mut ep_out = 0u8;
                for ep in setting.endpoint_descriptors() {
                    match ep.direction() {
                        rusb::Direction::In => ep_in = ep.address(),
                        rusb::Direction::Out => ep_out = ep.address(),
                    }
                }

                if ep_in == 0 || ep_out == 0 {
                    return Err(AppError::AntPlus("Could not find bulk endpoints".into()));
                }

                handle
                    .reset()
                    .map_err(|e| AppError::AntPlus(format!("Failed to reset: {}", e)))?;

                // Re-claim after reset
                if handle.kernel_driver_active(0).unwrap_or(false) {
                    let _ = handle.detach_kernel_driver(0);
                }
                handle.claim_interface(0).map_err(|e| {
                    AppError::AntPlus(format!("Failed to reclaim after reset: {}", e))
                })?;

                return Ok(Self {
                    handle,
                    endpoint_in: ep_in,
                    endpoint_out: ep_out,
                });
            }
        }

        Err(AppError::AntPlus("No ANT USB stick found".into()))
    }

    /// Send a raw ANT message
    pub fn send(&self, msg: &AntMessage) -> Result<(), AppError> {
        let packet = encode_message(msg);
        self.handle
            .write_bulk(self.endpoint_out, &packet, USB_TIMEOUT)
            .map_err(|e| AppError::AntPlus(format!("USB write failed: {}", e)))?;
        Ok(())
    }

    /// Try to receive all ANT messages from one USB read (non-blocking, returns empty Vec on timeout).
    /// A single USB read may contain multiple concatenated ANT messages.
    pub fn receive_all(&self) -> Result<Vec<AntMessage>, AppError> {
        let mut buf = [0u8; 64];
        match self
            .handle
            .read_bulk(self.endpoint_in, &mut buf, READ_TIMEOUT)
        {
            Ok(n) if n >= 4 => decode_all_messages(&buf[..n]),
            Ok(_) => Ok(Vec::new()),
            Err(rusb::Error::Timeout) => Ok(Vec::new()),
            Err(e) => Err(AppError::AntPlus(format!("USB read failed: {}", e))),
        }
    }

    /// Check if an ANT USB stick is available without opening it
    pub fn is_available() -> bool {
        let Ok(devices) = rusb::devices() else {
            return false;
        };
        devices.iter().any(|d| {
            d.device_descriptor().map_or(false, |desc| {
                desc.vendor_id() == GARMIN_VENDOR_ID
                    && (desc.product_id() == ANTUSB_M_PRODUCT_ID
                        || desc.product_id() == ANTUSB_2_PRODUCT_ID)
            })
        })
    }
}

impl Drop for AntUsb {
    fn drop(&mut self) {
        let _ = self.send(&AntMessage {
            msg_id: MSG_SYSTEM_RESET,
            data: vec![0x00],
        });
        let _ = self.handle.attach_kernel_driver(0);
    }
}

/// Encode an AntMessage into wire format
fn encode_message(msg: &AntMessage) -> Vec<u8> {
    let len = msg.data.len() as u8;
    let mut packet = Vec::with_capacity(4 + msg.data.len());
    packet.push(ANT_SYNC);
    packet.push(len);
    packet.push(msg.msg_id);
    packet.extend_from_slice(&msg.data);
    let checksum = packet.iter().fold(0u8, |acc, &b| acc ^ b);
    packet.push(checksum);
    packet
}

/// Decode all ANT messages from a buffer (handles multiple concatenated messages)
fn decode_all_messages(buf: &[u8]) -> Result<Vec<AntMessage>, AppError> {
    let mut messages = Vec::new();
    let mut pos = 0;

    while pos < buf.len() {
        // Find next sync byte
        match buf[pos..].iter().position(|&b| b == ANT_SYNC) {
            Some(offset) => pos += offset,
            None => break,
        }

        if buf.len() < pos + 3 {
            break; // Not enough bytes for header
        }

        let len = buf[pos + 1] as usize;
        let msg_id = buf[pos + 2];
        let total = pos + 3 + len + 1; // sync + len + id + data + checksum

        if buf.len() < total {
            break; // Incomplete message
        }

        let data = buf[pos + 3..pos + 3 + len].to_vec();

        // Verify checksum
        let expected: u8 = buf[pos..pos + 3 + len]
            .iter()
            .fold(0u8, |acc, &b| acc ^ b);
        let actual = buf[pos + 3 + len];

        if expected == actual {
            messages.push(AntMessage { msg_id, data });
        } else {
            log::warn!(
                "ANT checksum mismatch at offset {}: expected {:#x}, got {:#x}",
                pos,
                expected,
                actual
            );
        }

        pos = total; // Move past this message
    }

    Ok(messages)
}

#[cfg(test)]
mod tests {
    /// Decode wire bytes into a single AntMessage (used by roundtrip test)
    fn decode_message(buf: &[u8]) -> Result<Option<super::AntMessage>, crate::error::AppError> {
        let messages = super::decode_all_messages(buf)?;
        Ok(messages.into_iter().next())
    }

    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let msg = AntMessage {
            msg_id: MSG_SYSTEM_RESET,
            data: vec![0x00],
        };
        let encoded = encode_message(&msg);
        assert_eq!(encoded[0], ANT_SYNC);
        assert_eq!(encoded[1], 1); // length
        assert_eq!(encoded[2], MSG_SYSTEM_RESET);
        assert_eq!(encoded[3], 0x00); // data

        let decoded = decode_message(&encoded).unwrap().unwrap();
        assert_eq!(decoded.msg_id, MSG_SYSTEM_RESET);
        assert_eq!(decoded.data, vec![0x00]);
    }

    #[test]
    fn test_checksum() {
        let msg = AntMessage {
            msg_id: 0x42,
            data: vec![0x00, 0x00, 0x01],
        };
        let encoded = encode_message(&msg);
        let checksum = encoded.last().unwrap();
        let xor: u8 = encoded[..encoded.len() - 1]
            .iter()
            .fold(0u8, |acc, &b| acc ^ b);
        assert_eq!(*checksum, xor);
    }

    #[test]
    fn test_decode_all_messages_multiple() {
        let msg1 = AntMessage {
            msg_id: MSG_SYSTEM_RESET,
            data: vec![0x00],
        };
        let msg2 = AntMessage {
            msg_id: 0x42,
            data: vec![0x01, 0x02],
        };
        let mut buf = encode_message(&msg1);
        buf.extend_from_slice(&encode_message(&msg2));

        let decoded = decode_all_messages(&buf).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].msg_id, MSG_SYSTEM_RESET);
        assert_eq!(decoded[0].data, vec![0x00]);
        assert_eq!(decoded[1].msg_id, 0x42);
        assert_eq!(decoded[1].data, vec![0x01, 0x02]);
    }

    #[test]
    fn test_decode_all_messages_empty() {
        let decoded = decode_all_messages(&[]).unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_decode_all_messages_truncated() {
        // Only sync + length, no message ID or data
        let buf = [ANT_SYNC, 0x03];
        let decoded = decode_all_messages(&buf).unwrap();
        assert!(decoded.is_empty());
    }
}
