use btleplug::api::{Characteristic, Peripheral as _, WriteType};
use btleplug::platform::Peripheral;
use futures::StreamExt;
use log::{info, warn};
use tokio::time::{sleep, timeout, Duration};

use super::protocol::FTMS_CONTROL_POINT;
use crate::error::{AppError, BleError};

const REQUEST_CONTROL: u8 = 0x00;
const SET_TARGET_RESISTANCE: u8 = 0x04;
const SET_TARGET_POWER: u8 = 0x05;
const START_RESUME: u8 = 0x07;
const STOP_PAUSE: u8 = 0x08;
const SET_INDOOR_BIKE_SIMULATION: u8 = 0x11;

/// Encode FTMS Set Target Power (0x05). Watts clamped to >= 0, sent as sint16 LE.
pub(crate) fn encode_target_power(watts: i16) -> Vec<u8> {
    let bytes = watts.max(0).to_le_bytes();
    vec![SET_TARGET_POWER, bytes[0], bytes[1]]
}

/// Supported Resistance Level Range (0x2AD6), in the 0.1-unit raw values that
/// Set Target Resistance Level uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResistanceRange {
    pub min: i16,
    pub max: i16,
    pub increment: u16,
}

/// Parse 0x2AD6. FTMS v1.0 §4.13 defers the format to the SIG assigned numbers, and
/// the official definitions disagree: the 2017 SIG characteristic XML gives 6 octets
/// (sint16 min, sint16 max, uint16 increment, 0.1 resolution — also what GoldenCheetah
/// parses), the current GATT Specification Supplement §3.231 gives 3 uint8 octets with
/// self-contradictory scaling. Only the 6-octet form is used; any other length (or an
/// empty/inverted range) yields None and resistance keeps the 0-100 % mapping.
pub(crate) fn parse_resistance_range(data: &[u8]) -> Option<ResistanceRange> {
    if data.len() != 6 {
        return None;
    }
    let range = ResistanceRange {
        min: i16::from_le_bytes([data[0], data[1]]),
        max: i16::from_le_bytes([data[2], data[3]]),
        increment: u16::from_le_bytes([data[4], data[5]]),
    };
    (range.max > range.min).then_some(ResistanceRange { increment: range.increment.max(1), ..range })
}

/// Encode FTMS Set Target Resistance Level (0x04): sint16, 0.1 resolution (FTMS v1.0
/// Table 4.15 as amended by Errata Service Release 11, E8991: UINT8 → SINT16).
/// `level` is 0-100 % of the trainer's supported range, snapped to its increment.
/// Without a range, 0-100 % maps to raw 0-1000 (previous behaviour).
pub(crate) fn encode_resistance(level: u8, range: Option<ResistanceRange>) -> Vec<u8> {
    let pct = level.min(100) as f64 / 100.0;
    let raw: i16 = match range {
        None => (level.min(100) as i16) * 10,
        Some(r) => {
            let span = (r.max as f64) - (r.min as f64);
            let inc = r.increment as f64;
            let steps = (span * pct / inc).round();
            ((r.min as f64) + steps * inc).clamp(r.min as f64, r.max as f64) as i16
        }
    };
    let bytes = raw.to_le_bytes();
    vec![SET_TARGET_RESISTANCE, bytes[0], bytes[1]]
}

/// Supported Resistance Level Range characteristic (0x2AD6).
const SUPPORTED_RESISTANCE_RANGE: uuid::Uuid = uuid::Uuid::from_u128(0x00002AD6_0000_1000_8000_00805f9b34fb);

/// Encode FTMS Set Indoor Bike Simulation Parameters (0x11).
/// FTMS v1.0 §4.16.2.18 Table 4.20: wind speed sint16 (0.001 m/s), grade sint16 (0.01 %),
/// Crr uint8 (0.0001), Cw uint8 (0.01 kg/m). Values round to nearest; NaN is treated as 0.
pub(crate) fn encode_simulation(grade: f32, crr: f32, cw: f32) -> Vec<u8> {
    let finite = |v: f32| if v.is_nan() { 0.0 } else { v };
    let wind_speed: i16 = 0;
    // Clamped to ±100 % → raw ±10000, well inside sint16.
    let grade_raw = (finite(grade).clamp(-100.0, 100.0) * 100.0).round() as i16;
    let crr_raw = (finite(crr).clamp(0.0, 0.0255) / 0.0001).round() as u8;
    let cw_raw = (finite(cw).clamp(0.0, 2.55) / 0.01).round() as u8;
    let wind_bytes = wind_speed.to_le_bytes();
    let grade_bytes = grade_raw.to_le_bytes();
    vec![
        SET_INDOOR_BIKE_SIMULATION,
        wind_bytes[0],
        wind_bytes[1],
        grade_bytes[0],
        grade_bytes[1],
        crr_raw,
        cw_raw,
    ]
}

/// FTMS Control Point response op code
const RESPONSE_CODE: u8 = 0x80;

/// FTMS Control Point result codes
const RESULT_SUCCESS: u8 = 0x01;

pub struct TrainerController {
    peripheral: Peripheral,
    control_point: Characteristic,
    indications_enabled: bool,
    resistance_range: Option<ResistanceRange>,
    control_granted: bool,
}

impl TrainerController {
    pub fn new(peripheral: Peripheral) -> Result<Self, AppError> {
        let characteristics = peripheral.characteristics();
        let control_point = characteristics
            .iter()
            .find(|c| c.uuid == FTMS_CONTROL_POINT)
            .cloned()
            .ok_or_else(|| BleError::CharacteristicNotFound("FTMS Control Point".into()))?;
        Ok(Self {
            peripheral,
            control_point,
            indications_enabled: false,
            control_granted: false,
            resistance_range: None,
        })
    }

    /// Read Supported Resistance Level Range (0x2AD6) once at connect. Absent or
    /// unreadable → resistance keeps the legacy 0-100 % → 0-1000 mapping.
    pub async fn read_resistance_range(&mut self) {
        let chars = self.peripheral.characteristics();
        let Some(c) = chars.iter().find(|c| c.uuid == SUPPORTED_RESISTANCE_RANGE) else {
            return;
        };
        match self.peripheral.read(c).await {
            Ok(data) => {
                self.resistance_range = parse_resistance_range(&data);
                match self.resistance_range {
                    Some(r) => info!("FTMS: supported resistance range {:?}", r),
                    // Read once per connect, so this logs once per trainer connection.
                    None => warn!(
                        "FTMS: unsupported Supported Resistance Level Range format {:02X?}; using 0-100 %",
                        data
                    ),
                }
            }
            Err(e) => warn!("FTMS: reading Supported Resistance Level Range failed: {}", e),
        }
    }

    /// Enable indications and request control from the trainer.
    /// Called automatically before any command. Only performs the handshake once.
    async fn ensure_control(&mut self) -> Result<(), AppError> {
        if self.control_granted {
            return Ok(());
        }

        // Step 1: Subscribe to indications on the Control Point (CCCD write)
        if !self.indications_enabled {
            self.peripheral
                .subscribe(&self.control_point)
                .await
                .map_err(|e| BleError::Btleplug(format!("Failed to subscribe to FTMS CP: {}", e)))?;
            self.indications_enabled = true;
            info!("FTMS: indications enabled on Control Point");
            // Give the trainer time to process the CCCD write
            sleep(Duration::from_millis(100)).await;
        }

        // Step 2: Send REQUEST_CONTROL and wait for the trainer's indication response
        self.write_control_and_wait(&[REQUEST_CONTROL]).await?;
        info!("FTMS: REQUEST_CONTROL accepted");

        self.control_granted = true;
        info!("FTMS: control granted");
        Ok(())
    }

    pub async fn set_target_power(&mut self, watts: i16) -> Result<(), AppError> {
        self.ensure_control().await?;
        self.write_control_and_wait(&encode_target_power(watts))
            .await
    }

    /// Resistance mode using FTMS Set Target Resistance Level (0x04).
    /// Parameter is sint16 with 0.1 resolution: level 0-100% maps to raw 0-1000.
    pub async fn set_resistance(&mut self, level: u8) -> Result<(), AppError> {
        self.ensure_control().await?;
        self.write_control_and_wait(&encode_resistance(level, self.resistance_range))
            .await
    }

    pub async fn set_simulation(&mut self, grade: f32, crr: f32, cw: f32) -> Result<(), AppError> {
        self.ensure_control().await?;
        self.write_control_and_wait(&encode_simulation(grade, crr, cw))
            .await
    }

    pub async fn start(&mut self) -> Result<(), AppError> {
        self.ensure_control().await?;
        self.write_control_and_wait(&[START_RESUME]).await
    }

    pub async fn stop(&mut self) -> Result<(), AppError> {
        self.ensure_control().await?;
        self.write_control_and_wait(&[STOP_PAUSE, 0x01]).await
    }

    /// Reset control state (e.g. after a disconnection)
    #[allow(dead_code)]
    pub fn reset_control(&mut self) {
        self.indications_enabled = false;
        self.control_granted = false;
    }

    /// Write a command to the FTMS Control Point and wait for the indication response.
    /// Returns Ok if the trainer accepts, Err if it explicitly rejects.
    /// Timeouts are logged as warnings but treated as success (some trainers don't comply).
    async fn write_control_and_wait(&self, data: &[u8]) -> Result<(), AppError> {
        let op_code = data[0];

        // Subscribe to notification stream BEFORE writing to avoid missing the response
        let mut stream = self
            .peripheral
            .notifications()
            .await
            .map_err(|e| BleError::Btleplug(format!("Failed to get notification stream: {}", e)))?;

        // Write the command
        self.peripheral
            .write(&self.control_point, data, WriteType::WithResponse)
            .await
            .map_err(|e| BleError::Btleplug(format!("Failed to write FTMS control: {}", e)))?;

        // Wait up to 2s for the control point indication response
        let indication = timeout(Duration::from_secs(2), async {
            while let Some(notif) = stream.next().await {
                if notif.uuid == FTMS_CONTROL_POINT
                    && notif.value.len() >= 3
                    && notif.value[0] == RESPONSE_CODE
                    && notif.value[1] == op_code
                {
                    return Some(notif.value);
                }
            }
            None
        })
        .await;

        match indication {
            Ok(Some(response)) => {
                let result_code = response[2];
                if result_code != RESULT_SUCCESS {
                    let msg = match result_code {
                        0x02 => "Op code not supported",
                        0x03 => "Invalid parameter",
                        0x04 => "Operation failed",
                        0x05 => "Control not permitted",
                        _ => "Unknown error",
                    };
                    return Err(BleError::Btleplug(format!(
                        "Trainer rejected command 0x{:02X}: {}",
                        op_code, msg
                    )).into());
                }
            }
            Ok(None) => {
                warn!("FTMS notification stream ended while waiting for response to 0x{:02X}", op_code);
            }
            Err(_) => {
                warn!("FTMS indication response timed out for command 0x{:02X}", op_code);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Target Power (0x05) ----

    #[test]
    fn encode_power_200w() {
        // 200i16 LE = [0xC8, 0x00]
        assert_eq!(encode_target_power(200), vec![0x05, 0xC8, 0x00]);
    }

    #[test]
    fn encode_power_zero() {
        assert_eq!(encode_target_power(0), vec![0x05, 0x00, 0x00]);
    }

    #[test]
    fn encode_power_negative_clamped() {
        // Negative watts clamped to 0
        assert_eq!(encode_target_power(-50), vec![0x05, 0x00, 0x00]);
    }

    #[test]
    fn encode_power_max() {
        // i16::MAX = 32767 → LE = [0xFF, 0x7F]
        assert_eq!(encode_target_power(i16::MAX), vec![0x05, 0xFF, 0x7F]);
    }

    // ---- Resistance (0x04) ----

    #[test]
    fn resistance_without_range_uses_legacy_mapping() {
        // 50 * 10 = 500 → 500i16 LE = [0xF4, 0x01]
        assert_eq!(encode_resistance(50, None), vec![0x04, 0xF4, 0x01]);
    }

    #[test]
    fn encode_resistance_clamps_above_100() {
        // 200 → min(100) = 100, 100 * 10 = 1000 → LE = [0xE8, 0x03]
        assert_eq!(encode_resistance(200, None), vec![0x04, 0xE8, 0x03]);
    }

    #[test]
    fn encode_resistance_zero() {
        assert_eq!(encode_resistance(0, None), vec![0x04, 0x00, 0x00]);
    }

    // ---- Indoor Bike Simulation (0x11) ----

    #[test]
    fn encode_sim_zero_grade() {
        assert_eq!(
            encode_simulation(0.0, 0.0, 0.0),
            vec![0x11, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn encode_sim_negative_grade() {
        let data = encode_simulation(-10.0, 0.005, 0.5);
        assert_eq!(data[0], 0x11);
        // grade: -10.0 * 100 = -1000i16 LE = [0x18, 0xFC]
        assert_eq!(data[3], 0x18);
        assert_eq!(data[4], 0xFC);
        // crr: 0.005 / 0.0001 = 50
        assert_eq!(data[5], 50);
        // cw: 0.5 / 0.01 = 50
        assert_eq!(data[6], 50);
    }

    #[test]
    fn encode_sim_clamps_extremes() {
        let data = encode_simulation(-200.0, 0.1, 10.0);
        assert_eq!(data[0], 0x11);
        // grade clamped to -100.0: -100 * 100 = -10000i16 LE = [0xF0, 0xD8]
        assert_eq!(data[3], 0xF0);
        assert_eq!(data[4], 0xD8);
        // crr clamped to 0.0255: 0.0255 / 0.0001 = 255
        assert_eq!(data[5], 255);
        // cw clamped to 2.55: 2.55 / 0.01 = 255
        assert_eq!(data[6], 255);
    }

    // FTMS v1.0 §4.16.2.18 Table 4.20: grade sint16, 0.01 % resolution.

    #[test]
    fn ftms_grade_2_3_rounds_to_230() {
        let data = encode_simulation(2.3, 0.0, 0.0);
        assert_eq!(i16::from_le_bytes([data[3], data[4]]), 230);
        // 2.296 % → 229.6 raw → nearest 0.01 % is 230 (truncation gave 229)
        let data = encode_simulation(2.296, 0.0, 0.0);
        assert_eq!(i16::from_le_bytes([data[3], data[4]]), 230);
        // -2.296 % → -230 (truncation toward zero gave -229)
        let data = encode_simulation(-2.296, 0.0, 0.0);
        assert_eq!(i16::from_le_bytes([data[3], data[4]]), -230);
    }

    #[test]
    fn ftms_grade_clamped_to_plus_minus_100() {
        let data = encode_simulation(150.0, 0.0, 0.0);
        assert_eq!(i16::from_le_bytes([data[3], data[4]]), 10000);
    }

    #[test]
    fn ftms_grade_nan_encodes_zero() {
        let data = encode_simulation(f32::NAN, 0.0, 0.0);
        assert_eq!(i16::from_le_bytes([data[3], data[4]]), 0);
    }

    #[test]
    fn ftms_crr_rounds_to_nearest() {
        // 0.00406 / 0.0001 = 40.6 → 41 (truncation gave 40)
        assert_eq!(encode_simulation(0.0, 0.00406, 0.0)[5], 41);
    }

    // ---- Supported Resistance Level Range (0x2AD6) ----

    #[test]
    fn resistance_range_parse_6_octet_layout() {
        // min 0, max 1000 (100.0), increment 10 (1.0)
        let r = parse_resistance_range(&[0x00, 0x00, 0xE8, 0x03, 0x0A, 0x00]).unwrap();
        assert_eq!(r, ResistanceRange { min: 0, max: 1000, increment: 10 });
        // signed min: -50 = 0xFFCE
        let r = parse_resistance_range(&[0xCE, 0xFF, 0x32, 0x00, 0x05, 0x00]).unwrap();
        assert_eq!((r.min, r.max), (-50, 50));
    }

    #[test]
    fn resistance_range_3_octet_form_falls_back_to_legacy() {
        // The 3-octet (uint8) form's scaling is ambiguous across SIG sources: don't guess.
        assert_eq!(parse_resistance_range(&[1, 40, 1]), None);
        assert_eq!(encode_resistance(50, parse_resistance_range(&[1, 40, 1])), vec![0x04, 0xF4, 0x01]);
    }

    #[test]
    fn resistance_range_parse_bad_length_is_none() {
        assert_eq!(parse_resistance_range(&[]), None);
        assert_eq!(parse_resistance_range(&[1, 2, 3, 4]), None);
        // inverted 6-octet range: max 10 < min 400
        assert_eq!(parse_resistance_range(&[0x90, 0x01, 0x0A, 0x00, 0x0A, 0x00]), None);
    }

    #[test]
    fn resistance_50pct_scaled_into_range_with_increment() {
        let range = Some(ResistanceRange { min: 10, max: 400, increment: 10 });
        // 10 + round(390 * 0.5 / 10) * 10 = 10 + 20 * 10 = 210 = 0x00D2
        assert_eq!(encode_resistance(50, range), vec![0x04, 0xD2, 0x00]);
        assert_eq!(encode_resistance(0, range), vec![0x04, 10, 0x00]);
        // 400 = 0x0190
        assert_eq!(encode_resistance(100, range), vec![0x04, 0x90, 0x01]);
        assert_eq!(encode_resistance(250, range), vec![0x04, 0x90, 0x01], "clamped to max");
    }

    #[test]
    fn resistance_range_with_negative_min() {
        let range = Some(ResistanceRange { min: -100, max: 100, increment: 5 });
        // -100 + round(200 * 0.25 / 5) * 5 = -100 + 10 * 5 = -50 = 0xFFCE
        assert_eq!(encode_resistance(25, range), vec![0x04, 0xCE, 0xFF]);
    }
}
