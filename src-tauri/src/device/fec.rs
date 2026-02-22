use super::ant::channel::send_acknowledged;
use super::ant::usb::AntUsb;
use crate::error::AppError;

/// Encode target power page (0x31). Power in 0.25W resolution: watts * 4.
fn encode_target_power(watts: u16) -> [u8; 8] {
    let power_raw = watts.saturating_mul(4);
    let bytes = power_raw.to_le_bytes();
    [0x31, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, bytes[0], bytes[1]]
}

/// Encode basic resistance page (0x30). Level 0-100, transmitted as percentage * 2.
fn encode_resistance(level: u8) -> [u8; 8] {
    let resistance_raw = (level as u16).min(100) * 2;
    [
        0x30,
        0xFF,
        0xFF,
        0xFF,
        0xFF,
        0xFF,
        0xFF,
        resistance_raw as u8,
    ]
}

/// Encode simulation parameters page (0x33).
/// grade: -200.0 to +200.0 (0.01% resolution, offset binary with +20000)
/// crr: coefficient of rolling resistance (5e-5 resolution)
/// cw: wind resistance coefficient (0.01 kg/m resolution)
fn encode_simulation(grade: f32, crr: f32, cw: f32) -> [u8; 8] {
    let grade_clamped = grade.clamp(-200.0, 200.0);
    let crr_clamped = crr.clamp(0.0, 0.012_750); // u8 max (255) * 5e-5
    let cw_clamped = cw.clamp(0.0, 2.55); // u8 max (255) * 0.01
    let grade_raw = ((grade_clamped * 100.0) as i16 + 20000) as u16; // offset binary
    let grade_bytes = grade_raw.to_le_bytes();
    let crr_raw = (crr_clamped / 5e-5) as u8;
    let cw_raw = (cw_clamped / 0.01) as u8;
    [
        0x33,
        0xFF,
        0xFF,
        0xFF,
        grade_bytes[0],
        grade_bytes[1],
        crr_raw,
        cw_raw,
    ]
}

/// FE-C trainer control via ANT+ acknowledged messages
pub struct FecController<'a> {
    usb: &'a AntUsb,
    channel_number: u8,
}

impl<'a> FecController<'a> {
    pub fn new(usb: &'a AntUsb, channel_number: u8) -> Self {
        Self {
            usb,
            channel_number,
        }
    }

    /// Set target power (Page 0x31)
    pub fn set_target_power(&self, watts: u16) -> Result<(), AppError> {
        send_acknowledged(self.usb, self.channel_number, &encode_target_power(watts))
    }

    /// Set basic resistance (Page 0x30)
    pub fn set_resistance(&self, level: u8) -> Result<(), AppError> {
        send_acknowledged(self.usb, self.channel_number, &encode_resistance(level))
    }

    /// Set track/simulation parameters (Page 0x33)
    pub fn set_simulation(&self, grade: f32, crr: f32, cw: f32) -> Result<(), AppError> {
        send_acknowledged(
            self.usb,
            self.channel_number,
            &encode_simulation(grade, crr, cw),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Target Power (Page 0x31) ----

    #[test]
    fn encode_target_power_200w() {
        let data = encode_target_power(200);
        assert_eq!(data[0], 0x31);
        // 200 * 4 = 800 → 800u16 LE = [0x20, 0x03]
        assert_eq!(data[6], 0x20);
        assert_eq!(data[7], 0x03);
    }

    #[test]
    fn encode_target_power_zero() {
        let data = encode_target_power(0);
        assert_eq!(data[6], 0x00);
        assert_eq!(data[7], 0x00);
    }

    #[test]
    fn encode_target_power_saturates() {
        // 65535 * 4 would overflow u16 → saturating_mul caps at 65535
        let data = encode_target_power(65535);
        assert_eq!(data[6], 0xFF);
        assert_eq!(data[7], 0xFF);
    }

    // ---- Resistance (Page 0x30) ----

    #[test]
    fn encode_resistance_50_percent() {
        let data = encode_resistance(50);
        assert_eq!(data[0], 0x30);
        // 50 * 2 = 100
        assert_eq!(data[7], 100);
    }

    #[test]
    fn encode_resistance_clamps_above_100() {
        // level=200 → min(200, 100) = 100, 100 * 2 = 200
        let data = encode_resistance(200);
        assert_eq!(data[7], 200);
    }

    // ---- Simulation (Page 0x33) ----

    #[test]
    fn encode_simulation_zero_grade() {
        let data = encode_simulation(0.0, 0.0, 0.0);
        assert_eq!(data[0], 0x33);
        // grade=0 → raw = (0*100 + 20000) = 20000 = 0x4E20 LE = [0x20, 0x4E]
        assert_eq!(data[4], 0x20);
        assert_eq!(data[5], 0x4E);
    }

    #[test]
    fn encode_simulation_negative_grade() {
        let data = encode_simulation(-10.0, 0.0, 0.0);
        // grade=-10.0 → raw = (-10*100 + 20000) = (-1000 + 20000) = 19000 = 0x4A38
        // LE = [0x38, 0x4A]
        assert_eq!(data[4], 0x38);
        assert_eq!(data[5], 0x4A);
    }

    #[test]
    fn encode_simulation_crr_and_cw() {
        let data = encode_simulation(0.0, 0.005, 0.5);
        // crr=0.005 → raw = 0.005 / 5e-5 = 100
        assert_eq!(data[6], 100);
        // cw=0.5 → raw = 0.5 / 0.01 = 50
        assert_eq!(data[7], 50);
    }
}
