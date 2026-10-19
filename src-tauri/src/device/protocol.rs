use log::debug;
use std::time::{Duration, Instant};
use uuid::Uuid as BtUuid;

use super::types::SensorReading;

pub const HEART_RATE_MEASUREMENT: BtUuid =
    BtUuid::from_u128(0x00002A37_0000_1000_8000_00805f9b34fb);
pub const CYCLING_POWER_MEASUREMENT: BtUuid =
    BtUuid::from_u128(0x00002A63_0000_1000_8000_00805f9b34fb);
pub const CSC_MEASUREMENT: BtUuid = BtUuid::from_u128(0x00002A5B_0000_1000_8000_00805f9b34fb);
pub const INDOOR_BIKE_DATA: BtUuid = BtUuid::from_u128(0x00002AD2_0000_1000_8000_00805f9b34fb);
pub const FTMS_CONTROL_POINT: BtUuid = BtUuid::from_u128(0x00002AD9_0000_1000_8000_00805f9b34fb);

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn decode_heart_rate(data: &[u8], device_id: &str) -> Option<SensorReading> {
    if data.is_empty() {
        return None;
    }
    let flags = data[0];
    let hr_format_16bit = flags & 0x01 != 0;
    let bpm = if hr_format_16bit {
        if data.len() < 3 {
            return None;
        }
        u16::from_le_bytes([data[1], data[2]]) as u8
    } else {
        if data.len() < 2 {
            return None;
        }
        data[1]
    };
    Some(SensorReading::HeartRate {
        bpm,
        timestamp: Some(std::time::Instant::now()),
        epoch_ms: now_epoch_ms(),
        device_id: device_id.to_string(),
    })
}

/// Emit 0 rpm / 0 km/h once a sensor keeps notifying without a new revolution
/// event for this long (rider coasting or stopped).
pub const ZERO_AFTER: Duration = Duration::from_secs(3);

/// Upper plausibility bounds; values above are dropped as glitches.
const MAX_CADENCE_RPM: f32 = 200.0;
const MAX_SPEED_KMH: f32 = 120.0;

/// Cumulative crank revolutions + last crank event time (1/1024 s), as used by
/// CSC Measurement (CSCS v1.0 §3.1) and Cycling Power Measurement (CPS v1.1
/// §3.2.1.6) and the ANT+ cadence pages. Both counters roll over (u16).
#[derive(Debug, Default, Clone)]
pub struct CrankState {
    prev: Option<(u16, u16)>,
    last_event_at: Option<Instant>,
}

impl CrankState {
    /// The first sample only initialises state (no previous event to diff against).
    pub fn update(&mut self, revs: u16, event_time: u16, now: Instant) -> Option<f32> {
        let Some((prev_revs, prev_time)) = self.prev else {
            self.prev = Some((revs, event_time));
            self.last_event_at = Some(now);
            return None;
        };
        let time_diff = event_time.wrapping_sub(prev_time);
        let rev_diff = revs.wrapping_sub(prev_revs);
        if time_diff == 0 || rev_diff == 0 {
            // No new crank event in this notification.
            return self.zero_if_idle(now);
        }
        self.prev = Some((revs, event_time));
        self.last_event_at = Some(now);
        let rpm = rev_diff as f32 / (time_diff as f32 / 1024.0) * 60.0;
        (rpm < MAX_CADENCE_RPM).then_some(rpm)
    }

    fn zero_if_idle(&self, now: Instant) -> Option<f32> {
        let last = self.last_event_at?;
        (now.saturating_duration_since(last) >= ZERO_AFTER).then_some(0.0)
    }
}

/// Cumulative wheel revolutions (u32, not permitted to roll over) + last wheel event
/// time in 1/1024 s (CSCS v1.0 §3.1.1.2).
#[derive(Debug, Default, Clone)]
pub struct WheelState {
    prev: Option<(u32, u16)>,
    last_event_at: Option<Instant>,
}

impl WheelState {
    pub fn update(&mut self, revs: u32, event_time: u16, wheel_mm: u32, now: Instant) -> Option<f32> {
        let Some((prev_revs, prev_time)) = self.prev else {
            self.prev = Some((revs, event_time));
            self.last_event_at = Some(now);
            return None;
        };
        let time_diff = event_time.wrapping_sub(prev_time);
        let rev_diff = revs.wrapping_sub(prev_revs);
        if time_diff == 0 || rev_diff == 0 {
            let last = self.last_event_at?;
            return (now.saturating_duration_since(last) >= ZERO_AFTER).then_some(0.0);
        }
        self.prev = Some((revs, event_time));
        self.last_event_at = Some(now);
        if rev_diff >= 100 {
            return None; // counter reset / glitch
        }
        let distance_m = rev_diff as f32 * wheel_mm as f32 / 1000.0;
        let kmh = distance_m / (time_diff as f32 / 1024.0) * 3.6;
        (kmh < MAX_SPEED_KMH).then_some(kmh)
    }
}

/// Per-connection state of a CSC Measurement stream.
#[derive(Debug, Default, Clone)]
pub struct CscState {
    pub wheel: WheelState,
    pub crank: CrankState,
}

/// Per-connection state of a Cycling Power Measurement stream.
#[derive(Debug, Default, Clone)]
pub struct CpsState {
    pub crank: CrankState,
}

/// Decode Cycling Power Measurement (0x2A63), CPS v1.1 §3.2.1 / GATT Specification
/// Supplement §3.75. Field order after the uint16 flags and sint16 instantaneous power,
/// each present only when its flag bit is set:
///   bit 0 Pedal Power Balance uint8 (1/2 %), bit 2 Accumulated Torque uint16,
///   bit 4 Wheel Revolution Data (uint32 + uint16), bit 5 Crank Revolution Data
///   (uint16 cumulative revs + uint16 last event time, 1/1024 s), ...
/// Returns Power and, when crank data yields a new event, Cadence.
pub fn decode_cycling_power(
    data: &[u8],
    state: &mut CpsState,
    device_id: &str,
    now: Instant,
) -> Vec<SensorReading> {
    if data.len() < 4 {
        return vec![];
    }
    let flags = u16::from_le_bytes([data[0], data[1]]);
    let watts = i16::from_le_bytes([data[2], data[3]]);
    let epoch_ms = now_epoch_ms();
    let timestamp = Some(now);
    let mut offset = 4;
    let mut readings = Vec::new();

    // Pedal Power Balance (bit 0). Bit 1 = reference: 0 unknown, 1 left (§3.2.1.3).
    // With reference Left the value is the left pedal's share; we report the right
    // pedal's share (matching ANT+). With an unknown reference the side is undefined.
    let mut pedal_balance = None;
    if flags & 0x0001 != 0 {
        if let Some(&raw) = data.get(offset) {
            if flags & 0x0002 != 0 {
                let left_pct = ((raw as u16 + 1) / 2).min(100) as u8; // 1/2 % → nearest %
                pedal_balance = Some(100 - left_pct);
            }
        }
        offset += 1;
    }
    if flags & 0x0004 != 0 {
        offset += 2; // Accumulated Torque
    }
    if flags & 0x0010 != 0 {
        offset += 6; // Wheel Revolution Data
    }

    if watts >= 0 {
        readings.push(SensorReading::Power {
            watts: watts as u16,
            timestamp,
            epoch_ms,
            device_id: device_id.to_string(),
            pedal_balance,
        });
    } else {
        debug!("BLE cycling power: negative watts {} from {}", watts, device_id);
    }

    if flags & 0x0020 != 0 && data.len() >= offset + 4 {
        let revs = u16::from_le_bytes([data[offset], data[offset + 1]]);
        let time = u16::from_le_bytes([data[offset + 2], data[offset + 3]]);
        if let Some(rpm) = state.crank.update(revs, time, now) {
            readings.push(SensorReading::Cadence {
                rpm,
                timestamp,
                epoch_ms,
                device_id: device_id.to_string(),
            });
        }
    }
    readings
}

/// Default wheel circumference in mm (700x25c tire)
const DEFAULT_WHEEL_CIRCUMFERENCE_MM: u32 = 2105;

/// Decode CSC Measurement (0x2A5B), CSCS v1.0 §3.1: uint8 flags, then wheel data
/// (bit 0: uint32 revs + uint16 time) and crank data (bit 1: uint16 revs + uint16 time),
/// both times in 1/1024 s. The first notification after (re)connect only initialises.
pub fn decode_csc(data: &[u8], state: &mut CscState, device_id: &str, now: Instant) -> Vec<SensorReading> {
    if data.is_empty() {
        return vec![];
    }
    let flags = data[0];
    let mut offset = 1;
    let mut readings = Vec::new();
    let epoch_ms = now_epoch_ms();
    let timestamp = Some(now);

    if flags & 0x01 != 0 {
        if data.len() >= offset + 6 {
            let revs = u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]);
            let time = u16::from_le_bytes([data[offset + 4], data[offset + 5]]);
            if let Some(kmh) = state.wheel.update(revs, time, DEFAULT_WHEEL_CIRCUMFERENCE_MM, now) {
                readings.push(SensorReading::Speed { kmh, timestamp, epoch_ms, device_id: device_id.to_string() });
            }
        }
        offset += 6;
    }

    if flags & 0x02 != 0 && data.len() >= offset + 4 {
        let revs = u16::from_le_bytes([data[offset], data[offset + 1]]);
        let time = u16::from_le_bytes([data[offset + 2], data[offset + 3]]);
        if let Some(rpm) = state.crank.update(revs, time, now) {
            readings.push(SensorReading::Cadence { rpm, timestamp, epoch_ms, device_id: device_id.to_string() });
        }
    }

    readings
}

pub fn decode_indoor_bike_data(data: &[u8], device_id: &str) -> Vec<SensorReading> {
    if data.len() < 2 {
        return vec![];
    }
    let flags = u16::from_le_bytes([data[0], data[1]]);
    let mut offset = 2;
    let mut readings = Vec::new();
    let epoch_ms = now_epoch_ms();
    let timestamp = Some(std::time::Instant::now());
    let did = device_id.to_string();

    // Instantaneous Speed — present when bit 0 is 0 (FTMS inverted logic)
    if flags & 0x01 == 0 {
        if data.len() >= offset + 2 {
            let raw_speed = u16::from_le_bytes([data[offset], data[offset + 1]]);
            readings.push(SensorReading::Speed {
                kmh: raw_speed as f32 * 0.01,
                timestamp,
                epoch_ms,
                device_id: did.clone(),
            });
        }
        offset += 2;
    }

    // Average Speed (skip)
    if flags & 0x02 != 0 {
        offset += 2;
    }

    // Instantaneous Cadence (0.5 rpm resolution)
    if flags & 0x04 != 0 {
        if data.len() >= offset + 2 {
            let raw_cadence = u16::from_le_bytes([data[offset], data[offset + 1]]);
            readings.push(SensorReading::Cadence {
                rpm: raw_cadence as f32 * 0.5,
                timestamp,
                epoch_ms,
                device_id: did.clone(),
            });
        }
        offset += 2;
    }

    // Average Cadence (skip)
    if flags & 0x08 != 0 {
        offset += 2;
    }

    // Total Distance - 3 bytes (skip)
    if flags & 0x10 != 0 {
        offset += 3;
    }

    // Resistance Level (skip)
    if flags & 0x20 != 0 {
        offset += 2;
    }

    // Instantaneous Power
    if flags & 0x40 != 0 {
        if data.len() >= offset + 2 {
            let raw_power = i16::from_le_bytes([data[offset], data[offset + 1]]);
            if raw_power >= 0 {
                readings.push(SensorReading::Power {
                    watts: raw_power as u16,
                    timestamp,
                    epoch_ms,
                    device_id: did.clone(),
                    pedal_balance: None,
                });
            } else {
                debug!("BLE indoor bike: negative power {} from {}", raw_power, device_id);
            }
        }
        offset += 2;
    }

    // Average Power (skip)
    if flags & 0x80 != 0 {
        offset += 2;
    }

    // Expended Energy: total (uint16) + per hour (uint16) + per minute (uint8) = 5 bytes (skip)
    if flags & 0x100 != 0 {
        offset += 5;
    }

    // Heart Rate (uint8 bpm)
    if flags & 0x200 != 0 {
        if data.len() >= offset + 1 {
            let bpm = data[offset];
            if bpm > 0 {
                readings.push(SensorReading::HeartRate {
                    bpm,
                    timestamp,
                    epoch_ms,
                    device_id: did.clone(),
                });
            }
        }
    }

    readings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_approx(actual: f32, expected: f32, epsilon: f32, msg: &str) {
        assert!(
            (actual - expected).abs() < epsilon,
            "{msg}: expected {expected} ± {epsilon}, got {actual}"
        );
    }

    const DEV: &str = "test-device";

    /// Fixed base instant so decoder timing is explicit (no real-clock waits).
    fn t0() -> Instant {
        static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *BASE.get_or_init(Instant::now)
    }

    fn at(secs: f64) -> Instant {
        t0() + Duration::from_secs_f64(secs)
    }

    // ── decode_heart_rate ──────────────────────────────────────────

    #[test]
    fn decode_hr_empty_data() {
        assert!(decode_heart_rate(&[], DEV).is_none());
    }

    #[test]
    fn decode_hr_8bit_format() {
        let data = [0x00, 142]; // flags=0, 8-bit HR
        let r = decode_heart_rate(&data, DEV).unwrap();
        match r {
            SensorReading::HeartRate { bpm, .. } => assert_eq!(bpm, 142),
            _ => panic!("expected HeartRate"),
        }
    }

    #[test]
    fn decode_hr_16bit_format() {
        let hr: u16 = 150;
        let hr_bytes = hr.to_le_bytes();
        let data = [0x01, hr_bytes[0], hr_bytes[1]]; // flags=1, 16-bit HR
        let r = decode_heart_rate(&data, DEV).unwrap();
        match r {
            SensorReading::HeartRate { bpm, .. } => assert_eq!(bpm, 150),
            _ => panic!("expected HeartRate"),
        }
    }

    #[test]
    fn decode_hr_16bit_too_short() {
        let data = [0x01, 0x96]; // flags=1 (16-bit), but only 2 bytes total
        assert!(decode_heart_rate(&data, DEV).is_none());
    }

    // ── decode_cycling_power (CPS v1.1 §3.2.1, GSS §3.75) ───────────

    fn cps(flags: u16, watts: i16, rest: &[u8]) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&flags.to_le_bytes());
        d.extend_from_slice(&watts.to_le_bytes());
        d.extend_from_slice(rest);
        d
    }

    fn crank(revs: u16, time: u16) -> Vec<u8> {
        let mut d = revs.to_le_bytes().to_vec();
        d.extend_from_slice(&time.to_le_bytes());
        d
    }

    fn power_of(r: &SensorReading) -> (u16, Option<u8>) {
        match r {
            SensorReading::Power { watts, pedal_balance, .. } => (*watts, *pedal_balance),
            other => panic!("expected Power, got {:?}", other),
        }
    }

    fn rpm_of(r: &SensorReading) -> f32 {
        match r {
            SensorReading::Cadence { rpm, .. } => *rpm,
            other => panic!("expected Cadence, got {:?}", other),
        }
    }

    fn kmh_of(r: &SensorReading) -> f32 {
        match r {
            SensorReading::Speed { kmh, .. } => *kmh,
            other => panic!("expected Speed, got {:?}", other),
        }
    }

    #[test]
    fn decode_power_short_data() {
        assert!(decode_cycling_power(&[0x00, 0x00, 0xFA], &mut CpsState::default(), DEV, t0()).is_empty());
    }

    #[test]
    fn decode_power_normal_no_balance() {
        let r = decode_cycling_power(&cps(0x0000, 250, &[]), &mut CpsState::default(), DEV, t0());
        assert_eq!(r.len(), 1);
        assert_eq!(power_of(&r[0]), (250, None));
    }

    #[test]
    fn decode_power_negative_watts_rejected() {
        assert!(decode_cycling_power(&cps(0x0000, -1, &[]), &mut CpsState::default(), DEV, t0()).is_empty());
    }

    #[test]
    fn cps_balance_unknown_reference_is_none() {
        // flags bit0 = balance present, bit1 = 0 → reference unknown (§3.2.1.3): side undefined
        let r = decode_cycling_power(&cps(0x0001, 200, &[100]), &mut CpsState::default(), DEV, t0());
        assert_eq!(power_of(&r[0]), (200, None));
    }

    #[test]
    fn cps_balance_left_reference_inverted_rounded() {
        // raw 80 = 40.0 % left → 60 % right
        let r = decode_cycling_power(&cps(0x0003, 200, &[80]), &mut CpsState::default(), DEV, t0());
        assert_eq!(power_of(&r[0]).1, Some(60));
        // raw 81 = 40.5 % left → rounds to 41 % → 59 % right (truncation gave 60)
        let r = decode_cycling_power(&cps(0x0003, 200, &[81]), &mut CpsState::default(), DEV, t0());
        assert_eq!(power_of(&r[0]).1, Some(59));
    }

    #[test]
    fn cps_crank_first_sample_initialises() {
        let mut st = CpsState::default();
        let r = decode_cycling_power(&cps(0x0020, 200, &crank(100, 0)), &mut st, DEV, t0());
        assert_eq!(r.len(), 1, "power only; no previous crank event to diff");
    }

    #[test]
    fn cps_crank_only_cadence_90rpm() {
        // 3 revs in 2048/1024 = 2 s → 1.5 rev/s → 90 rpm
        let mut st = CpsState::default();
        decode_cycling_power(&cps(0x0020, 200, &crank(100, 0)), &mut st, DEV, t0());
        let r = decode_cycling_power(&cps(0x0020, 210, &crank(103, 2048)), &mut st, DEV, at(2.0));
        assert_eq!(r.len(), 2);
        assert_eq!(power_of(&r[0]).0, 210);
        assert_approx(rpm_of(&r[1]), 90.0, 0.01, "crank cadence");
    }

    #[test]
    fn cps_crank_offset_after_balance_torque_wheel() {
        // flags: bit0 balance, bit2 torque, bit4 wheel, bit5 crank → crank at offset 4+1+2+6 = 13
        let flags = 0x0001 | 0x0004 | 0x0010 | 0x0020;
        let pre = |revs: u16, time: u16| {
            let mut rest = vec![0x64]; // balance
            rest.extend_from_slice(&0x1234u16.to_le_bytes()); // accumulated torque
            rest.extend_from_slice(&0x0A0B0C0Du32.to_le_bytes()); // wheel revs
            rest.extend_from_slice(&0x0E0Fu16.to_le_bytes()); // wheel time
            rest.extend_from_slice(&crank(revs, time));
            cps(flags, 180, &rest)
        };
        let mut st = CpsState::default();
        decode_cycling_power(&pre(10, 1024), &mut st, DEV, t0());
        // 2 revs in 2048 ticks = 2 s → 60 rpm
        let r = decode_cycling_power(&pre(12, 3072), &mut st, DEV, at(2.0));
        assert_eq!(r.len(), 2);
        assert_approx(rpm_of(&r[1]), 60.0, 0.01, "crank read from the walked offset");
    }

    #[test]
    fn cps_crank_event_time_rollover() {
        // Last crank event time rolls over every 64 s (§3.2.1.6): 0xFC00 → 0x0000 is 1024 ticks
        let mut st = CpsState::default();
        decode_cycling_power(&cps(0x0020, 200, &crank(0xFFFF, 0xFC00)), &mut st, DEV, t0());
        // revs also roll over: 0xFFFF → 0x0000 is 1 rev
        let r = decode_cycling_power(&cps(0x0020, 200, &crank(0x0000, 0x0000)), &mut st, DEV, at(1.0));
        assert_approx(rpm_of(&r[1]), 60.0, 0.01, "rollover cadence");
    }

    #[test]
    fn cps_truncated_crank_field_still_yields_power() {
        // Crank flag set but only 2 of 4 crank bytes present
        let r = decode_cycling_power(&cps(0x0020, 230, &[0x01, 0x00]), &mut CpsState::default(), DEV, t0());
        assert_eq!(r.len(), 1);
        assert_eq!(power_of(&r[0]).0, 230);
    }

    #[test]
    fn cps_zero_cadence_after_3s_without_crank_event() {
        let mut st = CpsState::default();
        decode_cycling_power(&cps(0x0020, 200, &crank(100, 0)), &mut st, DEV, t0());
        decode_cycling_power(&cps(0x0020, 200, &crank(101, 1024)), &mut st, DEV, at(1.0));
        // Same crank data keeps arriving (coasting)
        let r = decode_cycling_power(&cps(0x0020, 0, &crank(101, 1024)), &mut st, DEV, at(3.9));
        assert_eq!(r.len(), 1, "2.9 s since last event: no cadence yet");
        let r = decode_cycling_power(&cps(0x0020, 0, &crank(101, 1024)), &mut st, DEV, at(4.0));
        assert_approx(rpm_of(&r[1]), 0.0, 0.01, "3 s without a crank event → 0 rpm");
    }

    // ── decode_csc (CSCS v1.0 §3.1) ─────────────────────────────────

    fn csc_wheel(revs: u32, time: u16) -> Vec<u8> {
        let mut d = vec![0x01];
        d.extend_from_slice(&revs.to_le_bytes());
        d.extend_from_slice(&time.to_le_bytes());
        d
    }

    fn csc_crank(revs: u16, time: u16) -> Vec<u8> {
        let mut d = vec![0x02];
        d.extend_from_slice(&crank(revs, time));
        d
    }

    #[test]
    fn decode_csc_empty_data() {
        assert!(decode_csc(&[], &mut CscState::default(), DEV, t0()).is_empty());
    }

    #[test]
    fn csc_first_notification_emits_nothing() {
        // Previously diffed against zero: 3 revs at 3072 ticks looked like 60 rpm.
        let mut st = CscState::default();
        assert!(decode_csc(&csc_crank(3, 3072), &mut st, DEV, t0()).is_empty());
        assert!(decode_csc(&csc_wheel(5000, 2048), &mut st, DEV, t0()).is_empty());
    }

    #[test]
    fn csc_second_notification_computes_cadence_60rpm() {
        let mut st = CscState::default();
        decode_csc(&csc_crank(10, 1000), &mut st, DEV, t0());
        // 1 rev in 1024 ticks = 1 s → 60 rpm
        let r = decode_csc(&csc_crank(11, 2024), &mut st, DEV, at(1.0));
        assert_eq!(r.len(), 1);
        assert_approx(rpm_of(&r[0]), 60.0, 0.01, "crank cadence");
    }

    #[test]
    fn csc_new_state_after_reconnect_reinitialises() {
        let mut st = CscState::default();
        decode_csc(&csc_crank(10, 1000), &mut st, DEV, t0());
        decode_csc(&csc_crank(11, 2024), &mut st, DEV, at(1.0));
        // A reconnect starts a new listener with fresh state: first notification is init-only.
        let mut fresh = CscState::default();
        assert!(decode_csc(&csc_crank(40, 9000), &mut fresh, DEV, at(30.0)).is_empty());
    }

    #[test]
    fn decode_csc_wheel_speed_normal() {
        // 1 rev × 2105 mm / 1 s = 2.105 m/s = 7.578 km/h
        let mut st = CscState::default();
        decode_csc(&csc_wheel(100, 0), &mut st, DEV, t0());
        let r = decode_csc(&csc_wheel(101, 1024), &mut st, DEV, at(1.0));
        assert_eq!(r.len(), 1);
        assert_approx(kmh_of(&r[0]), 7.578, 0.01, "wheel speed");
    }

    #[test]
    fn decode_csc_wheel_time_wraparound() {
        let mut st = CscState::default();
        decode_csc(&csc_wheel(5, 0xFFF0), &mut st, DEV, t0());
        // 0x03F0 - 0xFFF0 wraps to 0x0400 = 1024 ticks
        let r = decode_csc(&csc_wheel(6, 0x03F0), &mut st, DEV, at(1.0));
        assert_approx(kmh_of(&r[0]), 7.578, 0.01, "wraparound speed");
    }

    #[test]
    fn decode_csc_wheel_above_120kmh_filtered() {
        let mut st = CscState::default();
        decode_csc(&csc_wheel(0, 0), &mut st, DEV, t0());
        assert!(decode_csc(&csc_wheel(50, 1), &mut st, DEV, at(0.1)).is_empty());
    }

    #[test]
    fn decode_csc_wheel_rev_diff_ge_100_filtered() {
        let mut st = CscState::default();
        decode_csc(&csc_wheel(0, 0), &mut st, DEV, t0());
        assert!(decode_csc(&csc_wheel(100, 1024), &mut st, DEV, at(1.0)).is_empty());
    }

    #[test]
    fn decode_csc_crank_above_200rpm_filtered() {
        let mut st = CscState::default();
        decode_csc(&csc_crank(0, 0), &mut st, DEV, t0());
        assert!(decode_csc(&csc_crank(50, 1), &mut st, DEV, at(0.1)).is_empty());
    }

    #[test]
    fn decode_csc_combined_wheel_and_crank() {
        let both = |w: u32, wt: u16, c: u16, ct: u16| {
            let mut d = vec![0x03];
            d.extend_from_slice(&w.to_le_bytes());
            d.extend_from_slice(&wt.to_le_bytes());
            d.extend_from_slice(&crank(c, ct));
            d
        };
        let mut st = CscState::default();
        decode_csc(&both(0, 0, 0, 0), &mut st, DEV, t0());
        let r = decode_csc(&both(1, 1024, 1, 1024), &mut st, DEV, at(1.0));
        assert_eq!(r.len(), 2);
        assert_approx(kmh_of(&r[0]), 7.578, 0.01, "speed");
        assert_approx(rpm_of(&r[1]), 60.0, 0.01, "cadence");
    }

    #[test]
    fn csc_zero_cadence_after_3s_without_crank_event() {
        let mut st = CscState::default();
        decode_csc(&csc_crank(10, 1000), &mut st, DEV, t0());
        decode_csc(&csc_crank(11, 2024), &mut st, DEV, at(1.0));
        assert!(decode_csc(&csc_crank(11, 2024), &mut st, DEV, at(2.5)).is_empty());
        let r = decode_csc(&csc_crank(11, 2024), &mut st, DEV, at(4.0));
        assert_approx(rpm_of(&r[0]), 0.0, 0.01, "coasting → 0 rpm");
        let r = decode_csc(&csc_crank(11, 2024), &mut st, DEV, at(5.0));
        assert_approx(rpm_of(&r[0]), 0.0, 0.01, "repeats while still notifying");
    }

    #[test]
    fn csc_zero_speed_after_3s_without_wheel_event() {
        let mut st = CscState::default();
        decode_csc(&csc_wheel(100, 0), &mut st, DEV, t0());
        decode_csc(&csc_wheel(101, 1024), &mut st, DEV, at(1.0));
        let r = decode_csc(&csc_wheel(101, 1024), &mut st, DEV, at(4.0));
        assert_approx(kmh_of(&r[0]), 0.0, 0.01, "stopped → 0 km/h");
    }

    #[test]
    fn cadence_resumes_after_zero_on_new_event() {
        let mut st = CscState::default();
        decode_csc(&csc_crank(10, 1000), &mut st, DEV, t0());
        decode_csc(&csc_crank(11, 2024), &mut st, DEV, at(1.0));
        decode_csc(&csc_crank(11, 2024), &mut st, DEV, at(4.5)); // → 0
        // Pedalling again: 2 revs in 2048 ticks → 60 rpm
        let r = decode_csc(&csc_crank(13, 4072), &mut st, DEV, at(5.0));
        assert_approx(rpm_of(&r[0]), 60.0, 0.01, "normal cadence after coasting");
    }

    // ── decode_indoor_bike_data ────────────────────────────────────

    #[test]
    fn decode_indoor_bike_short_data() {
        assert!(decode_indoor_bike_data(&[0x00], DEV).is_empty());
    }

    #[test]
    fn decode_indoor_bike_speed_present() {
        // bit0=0 → speed present (inverted logic), raw=3000 → 30.0 km/h
        let flags: u16 = 0x0000;
        let raw_speed: u16 = 3000;
        let mut data = Vec::new();
        data.extend_from_slice(&flags.to_le_bytes());
        data.extend_from_slice(&raw_speed.to_le_bytes());
        let readings = decode_indoor_bike_data(&data, DEV);
        assert_eq!(readings.len(), 1);
        match &readings[0] {
            SensorReading::Speed { kmh, .. } => assert_approx(*kmh, 30.0, 0.01, "bike speed"),
            _ => panic!("expected Speed"),
        }
    }

    #[test]
    fn decode_indoor_bike_speed_not_present_when_bit0_set() {
        // FTMS inverted logic: bit0=1 means speed is NOT present
        let flags: u16 = 0x0001;
        let mut data = Vec::new();
        data.extend_from_slice(&flags.to_le_bytes());
        // No speed bytes follow — only the 2-byte flags field
        let readings = decode_indoor_bike_data(&data, DEV);
        assert!(readings.is_empty(), "bit0=1 should suppress speed");
    }

    #[test]
    fn decode_indoor_bike_cadence_and_power() {
        // bit2=1 (cadence), bit6=1 (power); speed is always mandatory
        let flags: u16 = 0x0004 | 0x0040;
        let raw_speed: u16 = 3000; // 30.0 km/h — mandatory
        let raw_cadence: u16 = 180; // 180 * 0.5 = 90.0 rpm
        let raw_power: i16 = 200;
        let mut data = Vec::new();
        data.extend_from_slice(&flags.to_le_bytes());
        data.extend_from_slice(&raw_speed.to_le_bytes());
        data.extend_from_slice(&raw_cadence.to_le_bytes());
        data.extend_from_slice(&raw_power.to_le_bytes());
        let readings = decode_indoor_bike_data(&data, DEV);
        assert_eq!(readings.len(), 3);
        match &readings[0] {
            SensorReading::Speed { kmh, .. } => assert_approx(*kmh, 30.0, 0.01, "speed"),
            _ => panic!("expected Speed"),
        }
        match &readings[1] {
            SensorReading::Cadence { rpm, .. } => assert_approx(*rpm, 90.0, 0.01, "cadence"),
            _ => panic!("expected Cadence"),
        }
        match &readings[2] {
            SensorReading::Power { watts, .. } => assert_eq!(*watts, 200),
            _ => panic!("expected Power"),
        }
    }

    #[test]
    fn decode_indoor_bike_negative_power_filtered() {
        // bit6=1 (power); speed is mandatory
        let flags: u16 = 0x0040;
        let raw_speed: u16 = 0; // 0 km/h
        let raw_power: i16 = -5;
        let mut data = Vec::new();
        data.extend_from_slice(&flags.to_le_bytes());
        data.extend_from_slice(&raw_speed.to_le_bytes());
        data.extend_from_slice(&raw_power.to_le_bytes());
        let readings = decode_indoor_bike_data(&data, DEV);
        // Only speed (0.0 km/h), negative power filtered
        assert_eq!(readings.len(), 1);
        assert!(matches!(&readings[0], SensorReading::Speed { .. }));
    }

    #[test]
    fn decode_indoor_bike_hr_and_zero_hr() {
        // bit9=1 (HR); speed is mandatory
        let flags: u16 = 0x0200;

        // bpm=140 → Speed + HeartRate
        let mut data = Vec::new();
        data.extend_from_slice(&flags.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes()); // speed (mandatory)
        data.push(140);
        let readings = decode_indoor_bike_data(&data, DEV);
        assert_eq!(readings.len(), 2); // speed + HR
        match &readings[1] {
            SensorReading::HeartRate { bpm, .. } => assert_eq!(*bpm, 140),
            _ => panic!("expected HeartRate"),
        }

        // bpm=0 → HR filtered, only speed
        let mut data_zero = Vec::new();
        data_zero.extend_from_slice(&flags.to_le_bytes());
        data_zero.extend_from_slice(&0u16.to_le_bytes()); // speed (mandatory)
        data_zero.push(0);
        let readings_zero = decode_indoor_bike_data(&data_zero, DEV);
        assert_eq!(readings_zero.len(), 1); // speed only
        assert!(matches!(&readings_zero[0], SensorReading::Speed { .. }));
    }

    #[test]
    fn decode_indoor_bike_skips_optional_fields() {
        // Enable all skip-only fields + HR to verify offset accumulation.
        // Speed is mandatory (+2), then skip fields: bit1(+2), bit3(+2), bit4(+3), bit5(+2), bit7(+2), bit8(+5) = 16 bytes
        // bit9=1 (HR at offset 2+2+16=20)
        let flags: u16 = 0x0002 | 0x0008 | 0x0010 | 0x0020 | 0x0080 | 0x0100 | 0x0200;
        assert_eq!(flags, 0x03BA);
        let mut data = Vec::new();
        data.extend_from_slice(&flags.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes()); // mandatory speed (2 bytes)
        data.extend_from_slice(&[0u8; 16]); // 16 bytes of skipped fields
        data.push(155); // HR bpm at offset 20
        assert_eq!(data.len(), 21);
        let readings = decode_indoor_bike_data(&data, DEV);
        assert_eq!(readings.len(), 2); // speed + HR
        match &readings[1] {
            SensorReading::HeartRate { bpm, .. } => assert_eq!(*bpm, 155),
            _ => panic!("expected HeartRate"),
        }
    }
}
