use log::{info, warn};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

use super::channel::PROFILE_SPEED_CADENCE;
use super::protocol::{AntDecoder, DEFAULT_WHEEL_CIRCUMFERENCE_MM};
use crate::device::primary::{admit_reading, SharedPrimaryState};
use crate::device::types::{AntDeviceMetadata, DeviceType, SensorReading};

/// Monotonic reference epoch for lock-free timestamps.
/// All `last_seen` values are stored as nanos elapsed since this instant.
static EPOCH: std::sync::LazyLock<std::time::Instant> =
    std::sync::LazyLock::new(std::time::Instant::now);

/// Store the current time as nanos-since-EPOCH into an AtomicI64.
pub fn atomic_now(ts: &AtomicI64) {
    let nanos = EPOCH.elapsed().as_nanos() as i64;
    ts.store(nanos, Ordering::Relaxed);
}

/// Read an AtomicI64 timestamp and return how long ago it was, or None if never set (0).
pub fn atomic_elapsed(ts: &AtomicI64) -> Option<std::time::Duration> {
    let nanos = ts.load(Ordering::Relaxed);
    if nanos == 0 {
        return None;
    }
    let now_nanos = EPOCH.elapsed().as_nanos() as i64;
    let elapsed_nanos = (now_nanos - nanos).max(0) as u64;
    Some(std::time::Duration::from_nanos(elapsed_nanos))
}

/// Test helper: make `ts` read as having been set `age` ago.
#[cfg(test)]
pub fn atomic_set_age(ts: &AtomicI64, age: std::time::Duration) {
    let nanos = EPOCH.elapsed().as_nanos() as i64 - age.as_nanos() as i64;
    // 0 means "never set"; nudge an exact zero so the value stays meaningful.
    ts.store(if nanos == 0 { -1 } else { nanos }, Ordering::Relaxed);
}

/// Decode ANT+ Common Data Page 80: Manufacturer's Information
/// Byte 3: HW revision
/// Bytes 4-5: Manufacturer ID (u16 LE)
/// Bytes 6-7: Model number (u16 LE)
fn decode_common_page_80(data: &[u8; 8], meta: &mut AntDeviceMetadata) {
    meta.hw_revision = Some(data[3]);
    meta.manufacturer_id = Some(u16::from_le_bytes([data[4], data[5]]));
    meta.model_number = Some(u16::from_le_bytes([data[6], data[7]]));
}

/// Decode ANT+ Common Data Page 81: Product Information
/// Byte 2: SW revision supplemental (0xFF = not used)
/// Byte 3: SW revision main
/// Bytes 4-7: Serial number (u32 LE, 0xFFFFFFFF = not available)
fn decode_common_page_81(data: &[u8; 8], meta: &mut AntDeviceMetadata) {
    let sw_main = data[3];
    let sw_sup = data[2];
    if sw_sup != 0xFF && sw_sup != 0 {
        meta.sw_revision = Some(format!("{}.{}", sw_main, sw_sup));
    } else {
        meta.sw_revision = Some(format!("{}", sw_main));
    }
    let serial = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    if serial != 0xFFFFFFFF && serial != 0 {
        meta.serial_number = Some(serial);
    }
}

/// Decode ANT+ Common Data Page 82: Battery Status
/// Byte 2: fractional battery voltage (1/256 V)
/// Byte 3: coarse battery voltage (bits 0-3) + descriptor (bits 4-7)
/// Byte 7: battery level % (0xFF = not available)
fn decode_common_page_82(data: &[u8; 8], meta: &mut AntDeviceMetadata) {
    let level = data[7];
    if level != 0xFF {
        meta.battery_level = Some(level);
    }
    let frac = data[2] as f32 / 256.0;
    let coarse = (data[3] & 0x0F) as f32;
    let voltage = coarse + frac;
    if voltage > 0.0 {
        meta.battery_voltage = Some(voltage);
    }
}

/// Background pages 2 (manufacturer) and 3 (product) of the paged ANT+ profiles:
/// Heart Rate (HRM D00000693 Rev 1.13 §5.3, Tables 6-7) and the separate speed /
/// cadence sensors (same layout per ANT's nRF Connect SDK `ant_bsc` page encoders).
/// Byte 0 bit 7 is the page-change toggle; bytes 1-3 must not be interpreted until
/// the toggle has been seen both set and clear (legacy devices never toggle).
#[derive(Debug, Default)]
pub struct BackgroundPages {
    seen_toggle_clear: bool,
    seen_toggle_set: bool,
}

impl BackgroundPages {
    /// Record the toggle bit; returns the page number if it is a background page
    /// worth decoding (2 or 3) and the device is known to use paged data.
    pub fn observe(&mut self, page_byte: u8) -> Option<u8> {
        if page_byte & 0x80 != 0 {
            self.seen_toggle_set = true;
        } else {
            self.seen_toggle_clear = true;
        }
        let page = page_byte & 0x7F;
        (self.seen_toggle_set && self.seen_toggle_clear && (page == 2 || page == 3)).then_some(page)
    }
}

/// Decode background page 2 or 3 into `meta`.
/// Page 2: byte 1 manufacturer ID, bytes 2-3 serial number upper 16 bits (the lower
/// 16 bits are the device number). Page 3: byte 1 HW version, byte 2 SW version,
/// byte 3 model number.
pub fn decode_background_page(page: u8, data: &[u8; 8], device_number: u16, meta: &mut AntDeviceMetadata) {
    match page {
        2 => {
            meta.manufacturer_id = Some(data[1] as u16);
            let upper = u16::from_le_bytes([data[2], data[3]]) as u32;
            meta.serial_number = Some((upper << 16) | device_number as u32);
        }
        3 => {
            meta.hw_revision = Some(data[1]);
            meta.sw_revision = Some(data[2].to_string());
            meta.model_number = Some(data[3] as u16);
        }
        _ => {}
    }
}

/// Device number from an `ant:<type>:<number>` id.
fn device_number_of(device_id: &str) -> Option<u16> {
    device_id.rsplit(':').next()?.parse().ok()
}

/// Listen for ANT+ data pages on a per-channel mpsc receiver and broadcast SensorReadings.
/// The router thread extracts 8-byte data pages from USB broadcast messages and sends them
/// via the mpsc channel, so this function never touches USB directly.
/// This runs in a blocking thread (call from tokio::task::spawn_blocking).
pub fn listen_ant_channel(
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    initial: Option<[u8; 8]>,
    device_type: DeviceType,
    tx: broadcast::Sender<SensorReading>,
    stop: Arc<AtomicBool>,
    device_id: String,
    metadata_store: Arc<Mutex<HashMap<String, AntDeviceMetadata>>>,
    device_type_id: u8,
    last_seen: Arc<AtomicI64>,
    primaries: Option<SharedPrimaryState>,
) {
    let mut decoder = AntDecoder::new();
    // HR and the separate speed/cadence sensors use toggled background pages.
    let paged_profile = matches!(device_type_id, 120 | 122 | 123);
    let device_number = device_number_of(&device_id);
    let mut background = BackgroundPages::default();

    info!("[{}] ANT+ channel listener started for {:?}", device_id, device_type);

    // The page that proved the device is present (consumed by connect) is decoded first.
    let mut pending = initial.map(|p| p.to_vec());

    while !stop.load(Ordering::Relaxed) {
        // recv_timeout so we periodically check the stop flag
        let next = match pending.take() {
            Some(p) => Ok(p),
            None => rx.recv_timeout(std::time::Duration::from_millis(200)),
        };
        let page_data = match next {
            Ok(data) => data,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                info!("[{}] ANT+ channel sender dropped, stopping listener for {:?}", device_id, device_type);
                break;
            }
        };

        if page_data.len() < 8 {
            continue;
        }

        let data: [u8; 8] = match page_data[..8].try_into() {
            Ok(arr) => arr,
            Err(_) => continue,
        };

        // Update last-data timestamp for connection watchdog (lock-free, every page)
        let page_num = data[0];
        atomic_now(&last_seen);

        // Decode ANT+ Common Data Pages — only lock metadata for these rare pages
        // The combined S&C page (121) has no page byte: byte 0 is cadence time data.
        let has_pages = device_type_id != PROFILE_SPEED_CADENCE.device_type_id;
        if has_pages && (page_num == 0x50 || page_num == 0x51 || page_num == 0x52) {
            let mut store = metadata_store.lock().unwrap_or_else(|e| e.into_inner());
            let meta = store.entry(device_id.clone()).or_default();
            match page_num {
                0x50 => decode_common_page_80(&data, meta),
                0x51 => decode_common_page_81(&data, meta),
                0x52 => decode_common_page_82(&data, meta),
                _ => {}
            }
            continue;
        }

        if paged_profile {
            if let (Some(page), Some(dn)) = (background.observe(data[0]), device_number) {
                let mut store = metadata_store.lock().unwrap_or_else(|e| e.into_inner());
                decode_background_page(page, &data, dn, store.entry(device_id.clone()).or_default());
            }
        }

        let readings: Vec<SensorReading> = match device_type {
            DeviceType::HeartRate => decoder.decode_hr(&data, &device_id).into_iter().collect(),
            DeviceType::Power => decoder.decode_power_page(&data, &device_id),
            DeviceType::CadenceSpeed => match device_type_id {
                123 => decoder
                    .decode_speed(&data, &device_id, DEFAULT_WHEEL_CIRCUMFERENCE_MM)
                    .into_iter()
                    .collect(),
                121 => decoder.decode_combined_sc(&data, &device_id, DEFAULT_WHEEL_CIRCUMFERENCE_MM),
                _ => decoder.decode_cadence(&data, &device_id).into_iter().collect(),
            },
            DeviceType::FitnessTrainer => decoder.decode_fec_trainer(&data, &device_id),
        };

        for reading in readings {
            if let Some(ref p) = primaries {
                if !admit_reading(p, &reading, std::time::Instant::now()) {
                    continue;
                }
            }
            if tx.send(reading).is_err() {
                warn!("[{}] No receivers for ANT+ readings, stopping listener", device_id);
                return;
            }
        }
    }

    info!("[{}] ANT+ channel listener stopped for {:?}", device_id, device_type);
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Page 80: Manufacturer's Information ----

    #[test]
    fn decode_page_80_manufacturer_info() {
        let mut meta = AntDeviceMetadata::default();
        // byte[3]=hw_rev=3, bytes[4-5]=mfg_id=0x0089(137), bytes[6-7]=model=0x1234(4660)
        let data: [u8; 8] = [0x50, 0xFF, 0xFF, 3, 0x89, 0x00, 0x34, 0x12];
        decode_common_page_80(&data, &mut meta);
        assert_eq!(meta.hw_revision, Some(3));
        assert_eq!(meta.manufacturer_id, Some(137));
        assert_eq!(meta.model_number, Some(4660));
    }

    // ---- Page 81: Product Information ----

    #[test]
    fn decode_page_81_sw_revision_with_supplemental() {
        let mut meta = AntDeviceMetadata::default();
        // sw_sup=5, sw_main=3 → "3.5"
        let data: [u8; 8] = [0x51, 0xFF, 5, 3, 0x78, 0x56, 0x34, 0x12];
        decode_common_page_81(&data, &mut meta);
        assert_eq!(meta.sw_revision.as_deref(), Some("3.5"));
    }

    #[test]
    fn decode_page_81_sw_revision_without_supplemental() {
        // sw_sup=0xFF → main only "3"
        let mut meta = AntDeviceMetadata::default();
        let data1: [u8; 8] = [0x51, 0xFF, 0xFF, 3, 0x01, 0x00, 0x00, 0x00];
        decode_common_page_81(&data1, &mut meta);
        assert_eq!(meta.sw_revision.as_deref(), Some("3"));

        // sw_sup=0 → main only "3"
        let mut meta2 = AntDeviceMetadata::default();
        let data2: [u8; 8] = [0x51, 0xFF, 0, 3, 0x01, 0x00, 0x00, 0x00];
        decode_common_page_81(&data2, &mut meta2);
        assert_eq!(meta2.sw_revision.as_deref(), Some("3"));
    }

    #[test]
    fn decode_page_81_serial_sentinels() {
        // 0xFFFFFFFF → None
        let mut meta1 = AntDeviceMetadata::default();
        let data1: [u8; 8] = [0x51, 0xFF, 5, 3, 0xFF, 0xFF, 0xFF, 0xFF];
        decode_common_page_81(&data1, &mut meta1);
        assert_eq!(meta1.serial_number, None);

        // 0x00000000 → None
        let mut meta2 = AntDeviceMetadata::default();
        let data2: [u8; 8] = [0x51, 0xFF, 5, 3, 0x00, 0x00, 0x00, 0x00];
        decode_common_page_81(&data2, &mut meta2);
        assert_eq!(meta2.serial_number, None);

        // Valid serial 0x12345678
        let mut meta3 = AntDeviceMetadata::default();
        let data3: [u8; 8] = [0x51, 0xFF, 5, 3, 0x78, 0x56, 0x34, 0x12];
        decode_common_page_81(&data3, &mut meta3);
        assert_eq!(meta3.serial_number, Some(0x12345678));
    }

    // ---- Page 82: Battery Status ----

    #[test]
    fn decode_page_82_battery_level_and_voltage() {
        let mut meta = AntDeviceMetadata::default();
        // level=85 (byte[7]), frac=128 (byte[2] = 0.5V), coarse=3 (byte[3] & 0x0F)
        // voltage = 3.0 + 128/256 = 3.5V
        let data: [u8; 8] = [0x52, 0xFF, 128, 3, 0xFF, 0xFF, 0xFF, 85];
        decode_common_page_82(&data, &mut meta);
        assert_eq!(meta.battery_level, Some(85));
        assert!((meta.battery_voltage.unwrap() - 3.5).abs() < 0.01);
    }

    #[test]
    fn decode_page_82_sentinels() {
        // level=0xFF → None
        let mut meta1 = AntDeviceMetadata::default();
        let data1: [u8; 8] = [0x52, 0xFF, 128, 3, 0xFF, 0xFF, 0xFF, 0xFF];
        decode_common_page_82(&data1, &mut meta1);
        assert_eq!(meta1.battery_level, None);
        // voltage should still be set (frac=128, coarse=3 → 3.5)
        assert!(meta1.battery_voltage.is_some());

        // frac=0, coarse=0 → voltage=0.0 → not stored
        let mut meta2 = AntDeviceMetadata::default();
        let data2: [u8; 8] = [0x52, 0xFF, 0, 0, 0xFF, 0xFF, 0xFF, 85];
        decode_common_page_82(&data2, &mut meta2);
        assert_eq!(meta2.battery_level, Some(85));
        assert_eq!(meta2.battery_voltage, None);
    }

    // ---- HR / S&C background pages 2-3 (HRM Rev 1.13 §5.3) ----

    #[test]
    fn hr_page2_ignored_before_toggle_seen() {
        let mut bg = BackgroundPages::default();
        // Legacy device: toggle bit never changes → bytes 1-3 are not page data
        assert_eq!(bg.observe(0x02), None);
        assert_eq!(bg.observe(0x02), None);
    }

    #[test]
    fn page_toggle_gate_opens_after_both_states() {
        let mut bg = BackgroundPages::default();
        assert_eq!(bg.observe(0x84), None); // page 4, toggle set
        assert_eq!(bg.observe(0x04), None); // page 4, toggle clear → gate open, not a bg page
        assert_eq!(bg.observe(0x82), Some(2), "toggle bit masked off the page number");
        assert_eq!(bg.observe(0x03), Some(3));
    }

    #[test]
    fn hr_page2_manufacturer_and_serial_with_toggle_bit_set() {
        let mut meta = AntDeviceMetadata::default();
        // page 2 (0x82 with toggle), mfr 32 (Wahoo), serial upper 0x1234; HR fields after
        let data: [u8; 8] = [0x82, 32, 0x34, 0x12, 0x00, 0x04, 9, 142];
        decode_background_page(2, &data, 0xABCD, &mut meta);
        assert_eq!(meta.manufacturer_id, Some(32));
        assert_eq!(meta.serial_number, Some(0x1234_ABCD));
    }

    #[test]
    fn hr_page3_hw_sw_model() {
        let mut meta = AntDeviceMetadata::default();
        let data: [u8; 8] = [0x03, 5, 7, 42, 0x00, 0x04, 9, 142];
        decode_background_page(3, &data, 1, &mut meta);
        assert_eq!(meta.hw_revision, Some(5));
        assert_eq!(meta.sw_revision.as_deref(), Some("7"));
        assert_eq!(meta.model_number, Some(42));
    }

    #[test]
    fn sc_page2_serial_combines_device_number() {
        let mut meta = AntDeviceMetadata::default();
        // speed sensor page 2: mfr 69 (Stages), serial upper 0x0001; device number 23456
        let data: [u8; 8] = [0x02, 69, 0x01, 0x00, 0x00, 0x04, 0x10, 0x00];
        decode_background_page(2, &data, 23456, &mut meta);
        assert_eq!(meta.manufacturer_id, Some(69));
        assert_eq!(meta.serial_number, Some((1 << 16) | 23456));
    }

    #[test]
    fn device_number_parsed_from_ant_id() {
        assert_eq!(device_number_of("ant:120:4242"), Some(4242));
        assert_eq!(device_number_of("not-ant"), None);
    }
}
