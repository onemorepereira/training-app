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

/// Profile limits (ANT+ FE-C D00001231 Rev 5.0, Tables 8-38 and 8-42).
const GRADE_MIN_PCT: f32 = -200.0;
const GRADE_MAX_PCT: f32 = 200.0;
const CRR_MAX: f32 = 0.0127;
const CW_MAX_KG_PER_M: f32 = 1.86;

/// Encode Track Resistance page (0x33), FE-C Rev 5.0 Table 8-42:
/// `[0x33, 0xFF x4 (reserved), grade_lo, grade_hi, crr]`.
/// Grade: 0.01 % units with a -200 % offset (0x4E20 = 0 %), rounded to nearest.
/// Crr: 5e-5 units. A NaN grade is treated as 0 %.
fn encode_track_resistance(grade: f32, crr: f32) -> [u8; 8] {
    let grade = if grade.is_nan() { 0.0 } else { grade };
    let grade_clamped = grade.clamp(GRADE_MIN_PCT, GRADE_MAX_PCT);
    // Max (200 + 200) * 100 = 40000, fits u16.
    let grade_raw = ((grade_clamped - GRADE_MIN_PCT) * 100.0).round() as u16;
    let crr = if crr.is_nan() { 0.0 } else { crr };
    let crr_raw = (crr.clamp(0.0, CRR_MAX) / 5e-5).round() as u8;
    let g = grade_raw.to_le_bytes();
    [0x33, 0xFF, 0xFF, 0xFF, 0xFF, g[0], g[1], crr_raw]
}

/// Encode Wind Resistance page (0x32), FE-C Rev 5.0 Table 8-38:
/// `[0x32, 0xFF x4 (reserved), cw (0.01 kg/m), wind speed, drafting factor]`.
/// Wind speed and drafting factor are sent as 0xFF ("use default": 0 km/h, 1.00).
fn encode_wind_resistance(cw: f32) -> [u8; 8] {
    [0x32, 0xFF, 0xFF, 0xFF, 0xFF, cw_raw(cw), 0xFF, 0xFF]
}

fn cw_raw(cw: f32) -> u8 {
    let cw = if cw.is_nan() { 0.0 } else { cw };
    (cw.clamp(0.0, CW_MAX_KG_PER_M) / 0.01).round() as u8
}

/// Pages to send for a simulation update. The wind page is only included when the
/// encoded wind coefficient differs from the last one the trainer acknowledged.
/// Returns the pages (wind first) and the encoded cw to remember on success.
fn simulation_pages(grade: f32, crr: f32, cw: f32, last_cw_raw: Option<u8>) -> (Vec<[u8; 8]>, u8) {
    let raw = cw_raw(cw);
    let mut pages = Vec::with_capacity(2);
    if last_cw_raw != Some(raw) {
        pages.push(encode_wind_resistance(cw));
    }
    pages.push(encode_track_resistance(grade, crr));
    (pages, raw)
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

    /// Set simulation parameters: Wind Resistance (0x32, only when cw changed) then
    /// Track Resistance (0x33). Returns the encoded cw the trainer now has.
    pub fn set_simulation(
        &self,
        grade: f32,
        crr: f32,
        cw: f32,
        last_cw_raw: Option<u8>,
    ) -> Result<u8, AppError> {
        let (pages, raw) = simulation_pages(grade, crr, cw, last_cw_raw);
        for page in &pages {
            send_acknowledged(self.usb, self.channel_number, page)?;
        }
        Ok(raw)
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

    // ---- Track Resistance (Page 0x33) ----
    // ANT+ FE-C D00001231 Rev 5.0 Table 8-42: [0x33, FF, FF, FF, FF, grade_lo, grade_hi, crr]
    // grade: 0.01 % units, offset -200 % (0x4E20 = 0 %); Crr: 5e-5 units.

    #[test]
    fn fec_track_resistance_zero_grade_layout() {
        // (0 + 200) * 100 = 20000 = 0x4E20; crr 0.004 / 5e-5 = 80
        assert_eq!(
            encode_track_resistance(0.0, 0.004),
            [0x33, 0xFF, 0xFF, 0xFF, 0xFF, 0x20, 0x4E, 80]
        );
    }

    #[test]
    fn fec_track_resistance_minus_3_percent() {
        // (-3 + 200) * 100 = 19700 = 0x4CF4
        let d = encode_track_resistance(-3.0, 0.004);
        assert_eq!(&d[5..7], &[0xF4, 0x4C]);
    }

    #[test]
    fn fec_track_resistance_plus_5_percent() {
        // (5 + 200) * 100 = 20500 = 0x5014
        let d = encode_track_resistance(5.0, 0.004);
        assert_eq!(&d[5..7], &[0x14, 0x50]);
    }

    #[test]
    fn fec_track_resistance_crr_0_004_is_raw_80() {
        assert_eq!(encode_track_resistance(0.0, 0.004)[7], 80);
        // 0.005 / 5e-5 = 100 (rounded, not truncated to 99)
        assert_eq!(encode_track_resistance(0.0, 0.005)[7], 100);
        // clamp to profile max 0.0127 → 254
        assert_eq!(encode_track_resistance(0.0, 1.0)[7], 254);
    }

    #[test]
    fn fec_grade_2_3_rounds_to_raw_20230() {
        // (2.3 + 200) * 100 = 20230 = 0x4F06 (truncation would give 20229)
        let d = encode_track_resistance(2.3, 0.004);
        assert_eq!(u16::from_le_bytes([d[5], d[6]]), 20230);
        // 2.296 % → 20229.6 → 20230 (sub-resolution input rounds to nearest)
        let d = encode_track_resistance(2.296, 0.004);
        assert_eq!(u16::from_le_bytes([d[5], d[6]]), 20230);
    }

    #[test]
    fn fec_grade_extremes_do_not_overflow() {
        let lo = encode_track_resistance(-1000.0, 0.0);
        assert_eq!(u16::from_le_bytes([lo[5], lo[6]]), 0); // clamped to -200 %
        let hi = encode_track_resistance(1000.0, 0.0);
        assert_eq!(u16::from_le_bytes([hi[5], hi[6]]), 40000); // +200 % = 0x9C40
    }

    #[test]
    fn fec_grade_nan_encodes_zero_percent() {
        let d = encode_track_resistance(f32::NAN, 0.004);
        assert_eq!(u16::from_le_bytes([d[5], d[6]]), 20000);
    }

    // ---- Wind Resistance (Page 0x32) ----
    // FE-C Rev 5.0 Table 8-38: [0x32, FF, FF, FF, FF, cw (0.01 kg/m), wind speed, drafting]

    #[test]
    fn fec_wind_resistance_page_layout_cw_0_51() {
        // 0.51 / 0.01 = 51; wind speed and drafting factor left at 0xFF (profile defaults)
        assert_eq!(
            encode_wind_resistance(0.51),
            [0x32, 0xFF, 0xFF, 0xFF, 0xFF, 51, 0xFF, 0xFF]
        );
        // clamp to profile max 1.86 kg/m → 186
        assert_eq!(encode_wind_resistance(5.0)[5], 186);
    }

    #[test]
    fn fec_wind_page_sent_only_when_cw_changes() {
        // First call: no cw sent yet → wind page (0x32) then track page (0x33)
        let (first, cw_raw) = simulation_pages(1.0, 0.004, 0.51, None);
        assert_eq!(first.len(), 2);
        assert_eq!(first[0][0], 0x32);
        assert_eq!(first[1][0], 0x33);
        assert_eq!(cw_raw, 51);
        // Same cw already acknowledged → only the track page
        let (second, _) = simulation_pages(2.0, 0.004, 0.51, Some(51));
        assert_eq!(second.len(), 1);
        assert_eq!(second[0][0], 0x33);
        // Changed cw → wind page again
        let (third, cw_raw) = simulation_pages(2.0, 0.004, 0.40, Some(51));
        assert_eq!(third.len(), 2);
        assert_eq!(cw_raw, 40);
    }
}
