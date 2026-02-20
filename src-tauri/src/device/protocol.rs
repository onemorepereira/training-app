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

pub fn decode_cycling_power(data: &[u8], device_id: &str) -> Option<SensorReading> {
    if data.len() < 4 {
        return None;
    }
    let flags = u16::from_le_bytes([data[0], data[1]]);
    let watts = i16::from_le_bytes([data[2], data[3]]);
    if watts < 0 {
        return None;
    }

    // Pedal Power Balance: flag bit 0 = present, bit 1 = reference (1 = left pedal)
    // Field is uint8 at offset 4, resolution 1/2 %
    let pedal_balance = if flags & 0x01 != 0 && data.len() >= 5 {
        let raw = data[4]; // percentage in 1/2% resolution
        let pct = raw / 2; // approximate to whole percent
        if flags & 0x02 != 0 {
            // Reference is left pedal — invert to right pedal for consistency with ANT+
            Some(100u8.saturating_sub(pct))
        } else {
            // Reference unknown — report as-is
            Some(pct)
        }
    } else {
        None
    };

    Some(SensorReading::Power {
        watts: watts as u16,
        timestamp: Some(std::time::Instant::now()),
        epoch_ms: now_epoch_ms(),
        device_id: device_id.to_string(),
        pedal_balance,
    })
}

/// Default wheel circumference in mm (700x25c tire)
const DEFAULT_WHEEL_CIRCUMFERENCE_MM: u32 = 2105;

pub fn decode_csc(
    data: &[u8],
    prev_wheel_revs: &mut u32,
    prev_wheel_time: &mut u16,
    prev_crank_revs: &mut u16,
    prev_crank_time: &mut u16,
    device_id: &str,
) -> Vec<SensorReading> {
    if data.is_empty() {
        return vec![];
    }
    let flags = data[0];
    let has_wheel = flags & 0x01 != 0;
    let has_crank = flags & 0x02 != 0;
    let mut offset = 1;
    let mut readings = Vec::new();
    let epoch_ms = now_epoch_ms();
    let timestamp = Some(std::time::Instant::now());

    // Wheel Revolution Data: uint32 cumulative revs + uint16 last event time (1/1024 s)
    if has_wheel {
        if data.len() >= offset + 6 {
            let wheel_revs = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            let wheel_time = u16::from_le_bytes([data[offset + 4], data[offset + 5]]);
            let rev_diff = wheel_revs.wrapping_sub(*prev_wheel_revs);
            let time_diff = wheel_time.wrapping_sub(*prev_wheel_time);
            *prev_wheel_revs = wheel_revs;
            *prev_wheel_time = wheel_time;
            if time_diff > 0 && rev_diff > 0 && rev_diff < 100 {
                let time_secs = time_diff as f32 / 1024.0;
                let distance_m = rev_diff as f32 * DEFAULT_WHEEL_CIRCUMFERENCE_MM as f32 / 1000.0;
                let kmh = (distance_m / time_secs) * 3.6;
                if kmh > 0.0 && kmh < 120.0 {
                    readings.push(SensorReading::Speed {
                        kmh,
                        timestamp,
                        epoch_ms,
                        device_id: device_id.to_string(),
                    });
                }
            }
        }
        offset += 6;
    }

    // Crank Revolution Data: uint16 cumulative revs + uint16 last event time (1/1024 s)
    if has_crank {
        if data.len() >= offset + 4 {
            let crank_revs = u16::from_le_bytes([data[offset], data[offset + 1]]);
            let crank_time = u16::from_le_bytes([data[offset + 2], data[offset + 3]]);
            let rev_diff = crank_revs.wrapping_sub(*prev_crank_revs);
            let time_diff = crank_time.wrapping_sub(*prev_crank_time);
            *prev_crank_revs = crank_revs;
            *prev_crank_time = crank_time;
            if time_diff > 0 && rev_diff > 0 {
                let time_secs = time_diff as f32 / 1024.0;
                let rpm = (rev_diff as f32 / time_secs) * 60.0;
                if rpm > 0.0 && rpm < 200.0 {
                    readings.push(SensorReading::Cadence {
                        rpm,
                        timestamp,
                        epoch_ms,
                        device_id: device_id.to_string(),
                    });
                }
            }
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

    // ── decode_cycling_power ───────────────────────────────────────

    #[test]
    fn decode_power_short_data() {
        assert!(decode_cycling_power(&[0x00, 0x00, 0xFA], DEV).is_none());
    }

    #[test]
    fn decode_power_normal_no_balance() {
        let flags: u16 = 0x0000;
        let watts: i16 = 250;
        let mut data = Vec::new();
        data.extend_from_slice(&flags.to_le_bytes());
        data.extend_from_slice(&watts.to_le_bytes());
        let r = decode_cycling_power(&data, DEV).unwrap();
        match r {
            SensorReading::Power {
                watts: w,
                pedal_balance,
                ..
            } => {
                assert_eq!(w, 250);
                assert_eq!(pedal_balance, None);
            }
            _ => panic!("expected Power"),
        }
    }

    #[test]
    fn decode_power_negative_watts_rejected() {
        let flags: u16 = 0x0000;
        let watts: i16 = -1;
        let mut data = Vec::new();
        data.extend_from_slice(&flags.to_le_bytes());
        data.extend_from_slice(&watts.to_le_bytes());
        assert!(decode_cycling_power(&data, DEV).is_none());
    }

    #[test]
    fn decode_power_pedal_balance_right_ref() {
        let flags: u16 = 0x0001; // balance present, bit1=0 (ref unknown)
        let watts: i16 = 200;
        let raw_balance: u8 = 100; // 100/2 = 50%
        let mut data = Vec::new();
        data.extend_from_slice(&flags.to_le_bytes());
        data.extend_from_slice(&watts.to_le_bytes());
        data.push(raw_balance);
        let r = decode_cycling_power(&data, DEV).unwrap();
        match r {
            SensorReading::Power {
                pedal_balance, ..
            } => assert_eq!(pedal_balance, Some(50)),
            _ => panic!("expected Power"),
        }
    }

    #[test]
    fn decode_power_pedal_balance_left_ref_inverted() {
        let flags: u16 = 0x0003; // balance present + left pedal ref
        let watts: i16 = 200;
        let raw_balance: u8 = 80; // 80/2=40, inverted: 100-40=60
        let mut data = Vec::new();
        data.extend_from_slice(&flags.to_le_bytes());
        data.extend_from_slice(&watts.to_le_bytes());
        data.push(raw_balance);
        let r = decode_cycling_power(&data, DEV).unwrap();
        match r {
            SensorReading::Power {
                pedal_balance, ..
            } => assert_eq!(pedal_balance, Some(60)),
            _ => panic!("expected Power"),
        }
    }

    // ── decode_csc ─────────────────────────────────────────────────

    #[test]
    fn decode_csc_empty_data() {
        let mut wr = 0u32;
        let mut wt = 0u16;
        let mut cr = 0u16;
        let mut ct = 0u16;
        assert!(decode_csc(&[], &mut wr, &mut wt, &mut cr, &mut ct, DEV).is_empty());
    }

    #[test]
    fn decode_csc_wheel_speed_normal() {
        // 1 rev × 2105mm / (1024/1024 s) = 2.105 m/s = 7.578 km/h
        let mut wr = 0u32;
        let mut wt = 0u16;
        let mut cr = 0u16;
        let mut ct = 0u16;
        let wheel_revs: u32 = 1;
        let wheel_time: u16 = 1024;
        let mut data = vec![0x01]; // flags: wheel present
        data.extend_from_slice(&wheel_revs.to_le_bytes());
        data.extend_from_slice(&wheel_time.to_le_bytes());
        let readings = decode_csc(&data, &mut wr, &mut wt, &mut cr, &mut ct, DEV);
        assert_eq!(readings.len(), 1);
        match &readings[0] {
            SensorReading::Speed { kmh, .. } => assert_approx(*kmh, 7.578, 0.01, "wheel speed"),
            _ => panic!("expected Speed"),
        }
    }

    #[test]
    fn decode_csc_wheel_time_wraparound() {
        let mut wr = 0u32;
        let mut wt = 0xFFF0u16; // near max
        let mut cr = 0u16;
        let mut ct = 0u16;
        let wheel_revs: u32 = 1;
        let wheel_time: u16 = 0x03F0; // wraps: 0x03F0 - 0xFFF0 = 0x0400 = 1024
        let mut data = vec![0x01];
        data.extend_from_slice(&wheel_revs.to_le_bytes());
        data.extend_from_slice(&wheel_time.to_le_bytes());
        let readings = decode_csc(&data, &mut wr, &mut wt, &mut cr, &mut ct, DEV);
        assert_eq!(readings.len(), 1);
        match &readings[0] {
            SensorReading::Speed { kmh, .. } => assert_approx(*kmh, 7.578, 0.01, "wraparound speed"),
            _ => panic!("expected Speed"),
        }
    }

    #[test]
    fn decode_csc_wheel_above_120kmh_filtered() {
        let mut wr = 0u32;
        let mut wt = 0u16;
        let mut cr = 0u16;
        let mut ct = 0u16;
        let wheel_revs: u32 = 50;
        let wheel_time: u16 = 1;
        let mut data = vec![0x01];
        data.extend_from_slice(&wheel_revs.to_le_bytes());
        data.extend_from_slice(&wheel_time.to_le_bytes());
        let readings = decode_csc(&data, &mut wr, &mut wt, &mut cr, &mut ct, DEV);
        assert!(readings.is_empty());
    }

    #[test]
    fn decode_csc_wheel_rev_diff_ge_100_filtered() {
        let mut wr = 0u32;
        let mut wt = 0u16;
        let mut cr = 0u16;
        let mut ct = 0u16;
        let wheel_revs: u32 = 100; // rev_diff=100, not < 100 → filtered
        let wheel_time: u16 = 1024;
        let mut data = vec![0x01];
        data.extend_from_slice(&wheel_revs.to_le_bytes());
        data.extend_from_slice(&wheel_time.to_le_bytes());
        let readings = decode_csc(&data, &mut wr, &mut wt, &mut cr, &mut ct, DEV);
        assert!(readings.is_empty());
    }

    #[test]
    fn decode_csc_crank_cadence_normal() {
        // 1 rev / (1024/1024 s) × 60 = 60.0 rpm
        let mut wr = 0u32;
        let mut wt = 0u16;
        let mut cr = 0u16;
        let mut ct = 0u16;
        let crank_revs: u16 = 1;
        let crank_time: u16 = 1024;
        let mut data = vec![0x02]; // flags: crank present
        data.extend_from_slice(&crank_revs.to_le_bytes());
        data.extend_from_slice(&crank_time.to_le_bytes());
        let readings = decode_csc(&data, &mut wr, &mut wt, &mut cr, &mut ct, DEV);
        assert_eq!(readings.len(), 1);
        match &readings[0] {
            SensorReading::Cadence { rpm, .. } => assert_approx(*rpm, 60.0, 0.1, "crank cadence"),
            _ => panic!("expected Cadence"),
        }
    }

    #[test]
    fn decode_csc_crank_above_200rpm_filtered() {
        let mut wr = 0u32;
        let mut wt = 0u16;
        let mut cr = 0u16;
        let mut ct = 0u16;
        let crank_revs: u16 = 50;
        let crank_time: u16 = 1; // 50 revs in ~1ms → way above 200 rpm
        let mut data = vec![0x02];
        data.extend_from_slice(&crank_revs.to_le_bytes());
        data.extend_from_slice(&crank_time.to_le_bytes());
        let readings = decode_csc(&data, &mut wr, &mut wt, &mut cr, &mut ct, DEV);
        assert!(readings.is_empty());
    }

    #[test]
    fn decode_csc_combined_wheel_and_crank() {
        let mut wr = 0u32;
        let mut wt = 0u16;
        let mut cr = 0u16;
        let mut ct = 0u16;
        let mut data = vec![0x03]; // flags: wheel + crank
        // Wheel: 1 rev at 1024 ticks
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&1024u16.to_le_bytes());
        // Crank: 1 rev at 1024 ticks
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&1024u16.to_le_bytes());
        let readings = decode_csc(&data, &mut wr, &mut wt, &mut cr, &mut ct, DEV);
        assert_eq!(readings.len(), 2);
        assert!(matches!(&readings[0], SensorReading::Speed { .. }));
        assert!(matches!(&readings[1], SensorReading::Cadence { .. }));
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
