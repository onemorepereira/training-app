//! On-disk codec for a ride's sensor log (`sessions/<id>.bin`, and the tail of an autosave).
//!
//! The layout is exactly what bincode 1 (`bincode::serialize`: fixint, little-endian) wrote,
//! kept byte for byte so every file already on disk, and older app versions, keep working:
//!
//! - log = `u64` count, then each reading as `u32` variant tag + its fields in declaration order
//! - `String` = `u64` byte length + UTF-8; `Option` = `u8` 0/1 (+ value); `f32` = IEEE bits
//!
//! The decoder also reads the older layouts still found on disk:
//!
//! - **v0.1.0–v0.3.x:** Power's `pedal_balance` had `skip_serializing_if`, so it is absent when
//!   `None` and `1, value` when `Some`, reading by reading.
//! - **pre-release dev builds:** no `device_id` at all.
//!
//! There is no header, so the layout is tried, not read. Any change to the fields of
//! `SensorReading` must add a new layout here (and should introduce a version marker); never
//! change an existing one. The golden files in `testdata/sensor/` were written by bincode 1.3.3
//! and must not be regenerated.

use std::collections::HashSet;

use log::warn;

use crate::device::types::{CommandSource, SensorReading};
use crate::error::AppError;

const POWER: u32 = 0;
const HEART_RATE: u32 = 1;
const CADENCE: u32 = 2;
const SPEED: u32 = 3;
const TRAINER_COMMAND: u32 = 4;

const ZONE_CONTROL: u32 = 0;
const MANUAL: u32 = 1;

/// Smallest possible encoded reading (pre-release HeartRate: tag + bpm + epoch_ms).
const MIN_READING_BYTES: usize = 4 + 1 + 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// v0.4.0 onwards: `device_id` always, `pedal_balance` always written as an `Option`.
    Current,
    /// v0.1.0–v0.3.x: `pedal_balance` omitted when `None`.
    OptionalBalance,
    /// Pre-release: no `device_id`; `pedal_balance` treated as optional.
    NoDeviceId,
}

const LAYOUTS: [Layout; 3] = [Layout::Current, Layout::OptionalBalance, Layout::NoDeviceId];

/// Encode a sensor log in the current layout.
pub(crate) fn encode(readings: &[SensorReading]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + readings.len() * 32);
    out.extend_from_slice(&(readings.len() as u64).to_le_bytes());
    for r in readings {
        match r {
            SensorReading::Power { watts, epoch_ms, device_id, pedal_balance, .. } => {
                out.extend_from_slice(&POWER.to_le_bytes());
                out.extend_from_slice(&watts.to_le_bytes());
                out.extend_from_slice(&epoch_ms.to_le_bytes());
                put_string(&mut out, device_id);
                match pedal_balance {
                    None => out.push(0),
                    Some(b) => out.extend_from_slice(&[1, *b]),
                }
            }
            SensorReading::HeartRate { bpm, epoch_ms, device_id, .. } => {
                out.extend_from_slice(&HEART_RATE.to_le_bytes());
                out.push(*bpm);
                out.extend_from_slice(&epoch_ms.to_le_bytes());
                put_string(&mut out, device_id);
            }
            SensorReading::Cadence { rpm, epoch_ms, device_id, .. } => {
                out.extend_from_slice(&CADENCE.to_le_bytes());
                out.extend_from_slice(&rpm.to_le_bytes());
                out.extend_from_slice(&epoch_ms.to_le_bytes());
                put_string(&mut out, device_id);
            }
            SensorReading::Speed { kmh, epoch_ms, device_id, .. } => {
                out.extend_from_slice(&SPEED.to_le_bytes());
                out.extend_from_slice(&kmh.to_le_bytes());
                out.extend_from_slice(&epoch_ms.to_le_bytes());
                put_string(&mut out, device_id);
            }
            SensorReading::TrainerCommand { target_watts, epoch_ms, source } => {
                out.extend_from_slice(&TRAINER_COMMAND.to_le_bytes());
                out.extend_from_slice(&target_watts.to_le_bytes());
                out.extend_from_slice(&epoch_ms.to_le_bytes());
                let s = match source {
                    CommandSource::ZoneControl => ZONE_CONTROL,
                    CommandSource::Manual => MANUAL,
                };
                out.extend_from_slice(&s.to_le_bytes());
            }
        }
    }
    out
}

fn put_string(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u64).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

/// Decode a sensor log written in any known layout.
///
/// Each layout must account for every byte; the first that does wins, in the order current,
/// v0.1–v0.3, pre-release. Only if none does are trailing bytes tolerated, and only in the
/// layouts the bincode 1 reader handled (it allowed them, so such a file stays readable). The
/// pre-release layout is never lenient: without `device_id` strings almost any prefix parses.
pub(crate) fn decode(data: &[u8]) -> Result<Vec<SensorReading>, AppError> {
    for layout in LAYOUTS {
        if let Some(readings) = decode_with(data, layout, true) {
            return Ok(readings);
        }
    }
    for layout in [Layout::Current, Layout::OptionalBalance] {
        if let Some(readings) = decode_with(data, layout, false) {
            warn!("Sensor log ({} bytes) has trailing bytes after its readings; ignored them", data.len());
            return Ok(readings);
        }
    }
    Err(AppError::Serialization(format!(
        "Failed to deserialize sensor data: {} bytes match no known sensor log layout",
        data.len()
    )))
}

/// Parse the whole log in one layout. Old layouts can be ambiguous where `pedal_balance` may or
/// may not follow a Power reading (`1` is both `Some` and the first byte of a HeartRate tag), so
/// this is a depth-first search over those choices that keeps only a parse which yields exactly
/// `count` readings and, when `exact`, ends on the last byte. States that cannot finish are
/// remembered, so each (offset, readings so far) is explored at most once.
fn decode_with(data: &[u8], layout: Layout, exact: bool) -> Option<Vec<SensorReading>> {
    let count = usize::try_from(read_u64(data, 0)?).ok()?;
    let start = 8;
    if count == 0 {
        return (!exact || data.len() == start).then(Vec::new);
    }
    if count > (data.len() - start) / MIN_READING_BYTES {
        return None;
    }

    // Each frame is one reading position and the parses of it not yet tried, the preferred one
    // last. Taking a candidate pops it, so a frame only keeps alternatives alive.
    struct Frame {
        pos: usize,
        untried: Vec<(SensorReading, usize)>,
    }
    let frame_at = |pos: usize| {
        let mut untried = read_reading(data, pos, layout);
        untried.reverse();
        Frame { pos, untried }
    };
    let mut dead: HashSet<(usize, usize)> = HashSet::new();
    let mut out: Vec<SensorReading> = Vec::with_capacity(count);
    let mut stack = vec![frame_at(start)];

    while !stack.is_empty() {
        // `out` holds the reading chosen by every frame below the top one.
        let depth = stack.len() - 1;
        out.truncate(depth);
        let frame = &mut stack[depth];
        let Some((reading, end)) = frame.untried.pop() else {
            dead.insert((frame.pos, depth));
            stack.pop();
            continue;
        };
        let done = depth + 1;
        if done == count {
            if !exact || end == data.len() {
                out.push(reading);
                return Some(out);
            }
            continue;
        }
        if dead.contains(&(end, done)) {
            continue;
        }
        out.push(reading);
        stack.push(frame_at(end));
    }
    None
}

/// Every way the bytes at `pos` parse as one reading in `layout`, with the offset after it,
/// most likely first.
fn read_reading(data: &[u8], pos: usize, layout: Layout) -> Vec<(SensorReading, usize)> {
    let mut found = Vec::new();
    let Some(tag) = read_u32(data, pos) else { return found };
    let p = pos + 4;
    let with_id = layout != Layout::NoDeviceId;
    match tag {
        POWER => {
            let Some(watts) = read_u16(data, p) else { return found };
            let Some(epoch_ms) = read_u64(data, p + 2) else { return found };
            let Some((device_id, p)) = read_device_id(data, p + 10, with_id) else { return found };
            let power = |pedal_balance| SensorReading::Power {
                watts,
                timestamp: None,
                epoch_ms,
                device_id: device_id.clone(),
                pedal_balance,
            };
            match layout {
                Layout::Current => match data.get(p) {
                    Some(0) => found.push((power(None), p + 1)),
                    Some(1) => {
                        if let Some(&b) = data.get(p + 1) {
                            found.push((power(Some(b)), p + 2));
                        }
                    }
                    _ => {}
                },
                Layout::OptionalBalance | Layout::NoDeviceId => {
                    // Absent first, as the bincode 1 legacy reader assumed.
                    found.push((power(None), p));
                    if data.get(p) == Some(&1) {
                        if let Some(&b) = data.get(p + 1) {
                            found.push((power(Some(b)), p + 2));
                        }
                    }
                }
            }
        }
        HEART_RATE => {
            let Some(&bpm) = data.get(p) else { return found };
            let Some(epoch_ms) = read_u64(data, p + 1) else { return found };
            if let Some((device_id, end)) = read_device_id(data, p + 9, with_id) {
                found.push((SensorReading::HeartRate { bpm, timestamp: None, epoch_ms, device_id }, end));
            }
        }
        CADENCE | SPEED => {
            let Some(value) = read_u32(data, p).map(f32::from_bits) else { return found };
            let Some(epoch_ms) = read_u64(data, p + 4) else { return found };
            if let Some((device_id, end)) = read_device_id(data, p + 12, with_id) {
                let reading = if tag == CADENCE {
                    SensorReading::Cadence { rpm: value, timestamp: None, epoch_ms, device_id }
                } else {
                    SensorReading::Speed { kmh: value, timestamp: None, epoch_ms, device_id }
                };
                found.push((reading, end));
            }
        }
        TRAINER_COMMAND => {
            let Some(target_watts) = read_u16(data, p) else { return found };
            let Some(epoch_ms) = read_u64(data, p + 2) else { return found };
            let source = match read_u32(data, p + 10) {
                Some(ZONE_CONTROL) => CommandSource::ZoneControl,
                Some(MANUAL) => CommandSource::Manual,
                _ => return found,
            };
            found.push((SensorReading::TrainerCommand { target_watts, epoch_ms, source }, p + 14));
        }
        _ => {}
    }
    found
}

/// `device_id` at `pos`, or an empty one when the layout predates it.
fn read_device_id(data: &[u8], pos: usize, present: bool) -> Option<(String, usize)> {
    if !present {
        return Some((String::new(), pos));
    }
    let len = usize::try_from(read_u64(data, pos)?).ok()?;
    let start = pos + 8;
    let bytes = data.get(start..start.checked_add(len)?)?;
    let s = std::str::from_utf8(bytes).ok()?;
    Some((s.to_owned(), start + len))
}

fn read_u16(data: &[u8], pos: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(pos..pos + 2)?.try_into().ok()?))
}

fn read_u32(data: &[u8], pos: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(pos..pos + 4)?.try_into().ok()?))
}

fn read_u64(data: &[u8], pos: usize) -> Option<u64> {
    Some(u64::from_le_bytes(data.get(pos..pos + 8)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURRENT: &[u8] = include_bytes!("testdata/sensor/current.bin");
    const NO_BALANCE: &[u8] = include_bytes!("testdata/sensor/legacy_no_pedal_balance.bin");
    const V0_3_MIXED: &[u8] = include_bytes!("testdata/sensor/v0_3_mixed_pedal_balance.bin");
    const PRE_DEVICE_ID: &[u8] = include_bytes!("testdata/sensor/pre_device_id.bin");
    const EMPTY: &[u8] = include_bytes!("testdata/sensor/empty.bin");

    fn power(watts: u16, epoch_ms: u64, device_id: &str, pedal_balance: Option<u8>) -> SensorReading {
        SensorReading::Power { watts, timestamp: None, epoch_ms, device_id: device_id.into(), pedal_balance }
    }
    fn hr(bpm: u8, epoch_ms: u64, device_id: &str) -> SensorReading {
        SensorReading::HeartRate { bpm, timestamp: None, epoch_ms, device_id: device_id.into() }
    }
    fn cadence(rpm: f32, epoch_ms: u64, device_id: &str) -> SensorReading {
        SensorReading::Cadence { rpm, timestamp: None, epoch_ms, device_id: device_id.into() }
    }
    fn speed(kmh: f32, epoch_ms: u64, device_id: &str) -> SensorReading {
        SensorReading::Speed { kmh, timestamp: None, epoch_ms, device_id: device_id.into() }
    }
    fn command(target_watts: u16, epoch_ms: u64, source: CommandSource) -> SensorReading {
        SensorReading::TrainerCommand { target_watts, epoch_ms, source }
    }

    /// Debug output distinguishes every field, including -0.0 from 0.0.
    fn assert_same(actual: &[SensorReading], expected: &[SensorReading]) {
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
    }

    /// The readings `testdata/sensor/current.bin` was generated from.
    fn current_readings() -> Vec<SensorReading> {
        vec![
            power(250, 1_760_000_000_123, "ble:AA:BB", Some(52)),
            power(0, 1_760_000_000_124, "", None),
            power(u16::MAX, u64::MAX, "ant:12345", Some(100)),
            hr(142, 1_760_000_001_000, "Polar H10 ❤"),
            hr(0, 0, ""),
            cadence(87.25, 1_760_000_002_000, "cad"),
            speed(31.5, 1_760_000_003_000, "spd"),
            speed(-0.0, 1, "neg-zero"),
            command(180, 1_760_000_004_000, CommandSource::ZoneControl),
            command(0, 1_760_000_005_000, CommandSource::Manual),
        ]
    }

    #[test]
    fn encode_is_byte_identical_to_bincode_1() {
        assert_eq!(encode(&current_readings()), CURRENT);
    }

    #[test]
    fn encode_empty_log_is_bincode_1_empty_vec() {
        assert_eq!(encode(&[]), EMPTY);
    }

    #[test]
    fn encode_heart_rate_matches_hand_computed_bytes() {
        let bytes = encode(&[hr(150, 0x0102_0304_0506_0708, "h")]);
        #[rustfmt::skip]
        let expected = [
            1, 0, 0, 0, 0, 0, 0, 0,             // count = 1
            1, 0, 0, 0,                         // HeartRate
            150,                                // bpm
            8, 7, 6, 5, 4, 3, 2, 1,             // epoch_ms
            1, 0, 0, 0, 0, 0, 0, 0, b'h',       // device_id
        ];
        assert_eq!(bytes, expected);
    }

    #[test]
    fn decodes_current_layout_golden_file() {
        assert_same(&decode(CURRENT).unwrap(), &current_readings());
    }

    #[test]
    fn decodes_empty_log() {
        assert!(decode(EMPTY).unwrap().is_empty());
    }

    fn no_balance_readings() -> Vec<SensorReading> {
        vec![
            power(250, 1_700_000_000_123, "ble:AA:BB", None),
            power(0, 1_700_000_000_124, "", None),
            hr(142, 1_700_000_001_000, "Polar H10 ❤"),
            cadence(87.25, 1_700_000_002_000, "cad"),
            speed(31.5, 1_700_000_003_000, "spd"),
            command(180, 1_700_000_004_000, CommandSource::ZoneControl),
            command(0, 1_700_000_005_000, CommandSource::Manual),
        ]
    }

    #[test]
    fn decodes_pre_v0_4_log_without_any_pedal_balance() {
        assert_same(&decode(NO_BALANCE).unwrap(), &no_balance_readings());
    }

    fn v0_3_mixed_readings() -> Vec<SensorReading> {
        vec![
            power(210, 1_771_000_000_000, "ble:PEDALS", Some(1)),
            hr(1, 1_771_000_000_001, "hr"),
            power(215, 1_771_000_000_250, "ble:PEDALS", None),
            hr(131, 1_771_000_000_251, "hr"),
            power(220, 1_771_000_000_500, "ble:PEDALS", Some(49)),
            cadence(90.5, 1_771_000_000_501, "cad"),
            power(0, 1_771_000_000_750, "", Some(0)),
            speed(28.75, 1_771_000_000_751, "spd"),
            power(199, 1_771_000_001_000, "ble:PEDALS", Some(51)),
        ]
    }

    #[test]
    fn decodes_v0_3_log_mixing_present_and_absent_pedal_balance() {
        // Some(1) followed by a HeartRate (tag bytes 01 00 00 00) is the ambiguous case.
        assert_same(&decode(V0_3_MIXED).unwrap(), &v0_3_mixed_readings());
    }

    #[test]
    fn decodes_pre_release_log_without_device_id() {
        let expected = vec![
            cadence(0.0, 1_771_378_000_000, ""),
            power(0, 1_771_378_000_001, "", None),
            hr(91, 1_771_378_000_100, ""),
            power(187, 1_771_378_000_250, "", None),
            speed(24.5, 1_771_378_000_251, ""),
        ];
        assert_same(&decode(PRE_DEVICE_ID).unwrap(), &expected);
    }

    #[test]
    fn round_trips_through_current_layout() {
        let readings = current_readings();
        assert_same(&decode(&encode(&readings)).unwrap(), &readings);
    }

    #[test]
    fn current_file_is_never_reinterpreted_as_an_old_layout() {
        // A Power with Some(1) followed by a HeartRate is also the old ambiguity; the current
        // layout must win because it accounts for every byte first.
        let readings = vec![power(200, 5, "p", Some(1)), hr(1, 6, "h"), power(201, 7, "p", None)];
        let bytes = encode(&readings);
        assert_eq!(decode_with(&bytes, Layout::Current, true).map(|v| v.len()), Some(3));
        assert_same(&decode(&bytes).unwrap(), &readings);
    }

    /// v0.3 bytes (no `device_id` text) where every Power is a choice point: half carry
    /// `pedal_balance` = Some(1) and are followed by a HeartRate with bpm 1, half have none.
    fn long_v0_3_log(pairs: u64) -> Vec<u8> {
        let mut bytes = (pairs * 4).to_le_bytes().to_vec();
        let hr_bpm_1 = |bytes: &mut Vec<u8>, i: u64| {
            bytes.extend_from_slice(&HEART_RATE.to_le_bytes());
            bytes.push(1);
            bytes.extend_from_slice(&i.to_le_bytes());
            bytes.extend_from_slice(&0u64.to_le_bytes());
        };
        for i in 0..pairs {
            for balance in [Some(1u8), None] {
                bytes.extend_from_slice(&POWER.to_le_bytes());
                bytes.extend_from_slice(&200u16.to_le_bytes());
                bytes.extend_from_slice(&i.to_le_bytes());
                bytes.extend_from_slice(&0u64.to_le_bytes());
                if let Some(b) = balance {
                    bytes.extend_from_slice(&[1, b]);
                }
                hr_bpm_1(&mut bytes, i);
            }
        }
        bytes
    }

    #[test]
    fn old_layout_search_decodes_long_ambiguous_log() {
        let bytes = long_v0_3_log(10_000);
        assert!(decode_with(&bytes, Layout::Current, true).is_none(), "fixture must need the old-layout search");
        let started = std::time::Instant::now();
        let readings = decode(&bytes).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "took {:?}", started.elapsed());
        assert_eq!(readings.len(), 40_000);
        assert_same(&readings[..4], &[power(200, 0, "", Some(1)), hr(1, 0, ""), power(200, 0, "", None), hr(1, 0, "")]);
        assert_same(&readings[39_998..], &[power(200, 9_999, "", None), hr(1, 9_999, "")]);
    }

    #[test]
    fn long_ambiguous_log_that_cannot_parse_fails_fast() {
        let mut bytes = long_v0_3_log(10_000);
        bytes.pop();
        let started = std::time::Instant::now();
        assert!(decode(&bytes).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "took {:?}", started.elapsed());
    }

    #[test]
    fn trailing_bytes_are_tolerated_like_bincode_1() {
        let mut bytes = CURRENT.to_vec();
        bytes.extend_from_slice(&[9, 9, 9]);
        assert_same(&decode(&bytes).unwrap(), &current_readings());
    }

    #[test]
    fn trailing_bytes_after_legacy_log_decode_as_bincode_1_did() {
        let mut bytes = NO_BALANCE.to_vec();
        bytes.extend_from_slice(&[1, 9, 9]);
        assert_same(&decode(&bytes).unwrap(), &no_balance_readings());
    }

    #[test]
    fn trailing_bytes_after_v0_3_log_leave_only_the_last_balance_ambiguous() {
        // The log ends with Power Some(51) = `.. 01 33`; with bytes after it, "absent, then
        // trailing bytes" is an equally valid parse, and absent is preferred.
        let mut bytes = V0_3_MIXED.to_vec();
        bytes.extend_from_slice(&[9, 9, 9]);
        let readings = decode(&bytes).unwrap();
        let expected = v0_3_mixed_readings();
        assert_same(&readings[..expected.len() - 1], &expected[..expected.len() - 1]);
        assert_same(&readings[expected.len() - 1..], &[power(199, 1_771_000_001_000, "ble:PEDALS", None)]);
    }

    #[test]
    fn trailing_bytes_after_empty_log_are_tolerated() {
        let mut bytes = EMPTY.to_vec();
        bytes.push(9);
        assert!(decode(&bytes).unwrap().is_empty());
    }

    #[test]
    fn pre_release_layout_never_tolerates_trailing_bytes() {
        let mut bytes = PRE_DEVICE_ID.to_vec();
        bytes.extend_from_slice(&[9, 9, 9]);
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn truncated_log_is_an_error() {
        let err = decode(&CURRENT[..CURRENT.len() - 3]).unwrap_err();
        assert!(err.to_string().contains("no known sensor log layout"), "{err}");
    }

    #[test]
    fn impossible_count_fails_without_allocating() {
        assert!(decode(&u64::MAX.to_le_bytes()).is_err());
        assert!(decode(&[1, 2, 3]).is_err());
    }

    #[test]
    fn huge_string_length_is_an_error() {
        let mut bytes = 1u64.to_le_bytes().to_vec();
        bytes.extend_from_slice(&HEART_RATE.to_le_bytes());
        bytes.push(120);
        bytes.extend_from_slice(&5u64.to_le_bytes());
        bytes.extend_from_slice(&(1u64 << 40).to_le_bytes());
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn invalid_utf8_device_id_is_an_error() {
        let mut bytes = encode(&[hr(120, 5, "ab")]);
        let n = bytes.len();
        bytes[n - 1] = 0xFF;
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn unknown_trainer_command_source_is_an_error() {
        let mut bytes = encode(&[command(100, 5, CommandSource::Manual)]);
        let n = bytes.len();
        bytes[n - 4] = 7;
        assert!(decode(&bytes).is_err());
    }
}
