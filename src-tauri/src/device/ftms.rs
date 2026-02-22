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

/// Encode FTMS Set Target Resistance Level (0x04). Level 0-100% → raw 0-1000 (0.1 resolution).
pub(crate) fn encode_resistance(level: u8) -> Vec<u8> {
    let raw = (level.min(100) as i16) * 10;
    let bytes = raw.to_le_bytes();
    vec![SET_TARGET_RESISTANCE, bytes[0], bytes[1]]
}

/// Encode FTMS Set Indoor Bike Simulation Parameters (0x11).
pub(crate) fn encode_simulation(grade: f32, crr: f32, cw: f32) -> Vec<u8> {
    let wind_speed: i16 = 0;
    let grade_raw = (grade.clamp(-100.0, 100.0) * 100.0) as i16;
    let crr_raw = (crr.clamp(0.0, 0.0255) / 0.0001) as u8;
    let cw_raw = (cw.clamp(0.0, 2.55) / 0.01) as u8;
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
        })
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
        self.write_control_and_wait(&encode_resistance(level))
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
    fn encode_resistance_50_pct() {
        // 50 * 10 = 500 → 500i16 LE = [0xF4, 0x01]
        assert_eq!(encode_resistance(50), vec![0x04, 0xF4, 0x01]);
    }

    #[test]
    fn encode_resistance_clamps_above_100() {
        // 200 → min(100) = 100, 100 * 10 = 1000 → LE = [0xE8, 0x03]
        assert_eq!(encode_resistance(200), vec![0x04, 0xE8, 0x03]);
    }

    #[test]
    fn encode_resistance_zero() {
        assert_eq!(encode_resistance(0), vec![0x04, 0x00, 0x00]);
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
}
