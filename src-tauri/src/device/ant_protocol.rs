use super::types::SensorReading;

/// Default wheel circumference in mm (700x25c)
pub const DEFAULT_WHEEL_CIRCUMFERENCE_MM: u32 = 2105;

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Stateful ANT+ decoder that tracks previous values for delta calculations.
///
/// **Note:** The first sample for cadence and speed is used to initialize the
/// decoder state and returns `None` — ANT+ reports cumulative counters, so the
/// first sample has no previous value to compute a delta from. Power is the
/// exception: the first sample returns the instantaneous power field directly
/// (bytes 6-7), avoiding a 1-2s data gap after connecting.
#[derive(Debug, Default)]
pub struct AntDecoder {
    // Power profile state
    prev_power_event_count: u8,
    prev_power_accumulated: u16,
    power_initialized: bool,

    // Cadence profile state
    prev_cadence_event_time: u16,
    prev_cadence_revs: u16,
    cadence_initialized: bool,

    // Speed profile state
    prev_speed_event_time: u16,
    prev_speed_revs: u16,
    speed_initialized: bool,
}

impl AntDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode ANT+ Heart Rate data page
    /// All HR pages have computed HR in byte 7
    pub fn decode_hr(&self, data: &[u8; 8], device_id: &str) -> Option<SensorReading> {
        let bpm = data[7];
        if bpm == 0 {
            return None;
        }
        Some(SensorReading::HeartRate {
            bpm,
            timestamp: Some(std::time::Instant::now()),
            epoch_ms: now_epoch_ms(),
            device_id: device_id.to_string(),
        })
    }

    /// Decode ANT+ Cycling Power Standard Power page (0x10)
    /// Byte 1: update event count
    /// Byte 2: pedal power (bit 7 = differentiation, bits 0-6 = right pedal %)
    /// Byte 4-5: accumulated power (u16 LE)
    /// Byte 6-7: instantaneous power (u16 LE)
    pub fn decode_power(&mut self, data: &[u8; 8], device_id: &str) -> Option<SensorReading> {
        let page = data[0];
        if page != 0x10 {
            return None; // Only decode standard power page
        }

        let event_count = data[1];
        let accumulated = u16::from_le_bytes([data[4], data[5]]);
        let instant_power = u16::from_le_bytes([data[6], data[7]]);

        // Extract pedal balance from byte 2
        let pedal_power_byte = data[2];
        let pedal_balance = if pedal_power_byte & 0x80 != 0 {
            Some(pedal_power_byte & 0x7F) // right pedal contribution %
        } else {
            None
        };

        if !self.power_initialized {
            self.prev_power_event_count = event_count;
            self.prev_power_accumulated = accumulated;
            self.power_initialized = true;
            // Return instant power on first sample so data appears immediately
            return Some(SensorReading::Power {
                watts: instant_power,
                timestamp: Some(std::time::Instant::now()),
                epoch_ms: now_epoch_ms(),
                device_id: device_id.to_string(),
                pedal_balance,
            });
        }

        // Check for new data (event count changed)
        if event_count == self.prev_power_event_count {
            return None;
        }
        self.prev_power_event_count = event_count;
        self.prev_power_accumulated = accumulated;

        Some(SensorReading::Power {
            watts: instant_power,
            timestamp: Some(std::time::Instant::now()),
            epoch_ms: now_epoch_ms(),
            device_id: device_id.to_string(),
            pedal_balance,
        })
    }

    /// Decode ANT+ Cadence sensor data page (page 0 or default)
    /// Byte 4-5: cadence event time (1/1024 s, u16 LE)
    /// Byte 6-7: cumulative cadence revolutions (u16 LE)
    pub fn decode_cadence(&mut self, data: &[u8; 8], device_id: &str) -> Option<SensorReading> {
        let event_time = u16::from_le_bytes([data[4], data[5]]);
        let revs = u16::from_le_bytes([data[6], data[7]]);

        if !self.cadence_initialized {
            self.prev_cadence_event_time = event_time;
            self.prev_cadence_revs = revs;
            self.cadence_initialized = true;
            return None;
        }

        let time_diff = event_time.wrapping_sub(self.prev_cadence_event_time);
        let rev_diff = revs.wrapping_sub(self.prev_cadence_revs);
        self.prev_cadence_event_time = event_time;
        self.prev_cadence_revs = revs;

        if time_diff == 0 || rev_diff == 0 {
            return None;
        }

        let time_secs = time_diff as f32 / 1024.0;
        let rpm = (rev_diff as f32 / time_secs) * 60.0;

        if rpm > 200.0 || rpm < 0.0 {
            return None;
        }

        Some(SensorReading::Cadence {
            rpm,
            timestamp: Some(std::time::Instant::now()),
            epoch_ms: now_epoch_ms(),
            device_id: device_id.to_string(),
        })
    }

    /// Decode ANT+ Speed sensor data page
    /// Byte 4-5: speed event time (1/1024 s, u16 LE)
    /// Byte 6-7: cumulative wheel revolutions (u16 LE)
    pub fn decode_speed(
        &mut self,
        data: &[u8; 8],
        device_id: &str,
        wheel_circumference_mm: u32,
    ) -> Option<SensorReading> {
        let event_time = u16::from_le_bytes([data[4], data[5]]);
        let revs = u16::from_le_bytes([data[6], data[7]]);

        if !self.speed_initialized {
            self.prev_speed_event_time = event_time;
            self.prev_speed_revs = revs;
            self.speed_initialized = true;
            return None;
        }

        let time_diff = event_time.wrapping_sub(self.prev_speed_event_time);
        let rev_diff = revs.wrapping_sub(self.prev_speed_revs);
        self.prev_speed_event_time = event_time;
        self.prev_speed_revs = revs;

        if time_diff == 0 || rev_diff == 0 {
            return None;
        }

        let time_secs = time_diff as f64 / 1024.0;
        let distance_m = rev_diff as f64 * wheel_circumference_mm as f64 / 1000.0;
        let kmh = (distance_m / time_secs) * 3.6;

        if kmh > 120.0 || kmh < 0.0 {
            return None;
        }

        Some(SensorReading::Speed {
            kmh: kmh as f32,
            timestamp: Some(std::time::Instant::now()),
            epoch_ms: now_epoch_ms(),
            device_id: device_id.to_string(),
        })
    }

    /// Decode ANT+ FE-C Specific Trainer Data page (0x19)
    /// Byte 1: update event count
    /// Byte 2: instantaneous cadence (0xFF = invalid)
    /// Byte 3-4: accumulated power (u16 LE)
    /// Byte 5-6: instantaneous power (u16 LE, bits 0-11 only)
    pub fn decode_fec_trainer(&self, data: &[u8; 8], device_id: &str) -> Vec<SensorReading> {
        let page = data[0];
        let mut readings = Vec::new();
        let epoch_ms = now_epoch_ms();
        let timestamp = Some(std::time::Instant::now());
        let did = device_id.to_string();

        if page == 0x19 {
            // Specific Trainer Data
            let cadence = data[2];
            if cadence != 0xFF {
                readings.push(SensorReading::Cadence {
                    rpm: cadence as f32,
                    timestamp,
                    epoch_ms,
                    device_id: did.clone(),
                });
            }

            let instant_power = u16::from_le_bytes([data[5], data[6]]) & 0x0FFF;
            readings.push(SensorReading::Power {
                watts: instant_power,
                timestamp,
                epoch_ms,
                device_id: did,
                pedal_balance: None,
            });
        } else if page == 0x10 {
            // General FE Data
            let speed_raw = u16::from_le_bytes([data[4], data[5]]);
            if speed_raw != 0xFFFF {
                let kmh = speed_raw as f32 * 0.001 * 3.6;
                readings.push(SensorReading::Speed {
                    kmh,
                    timestamp,
                    epoch_ms,
                    device_id: did.clone(),
                });
            }

            let hr = data[6];
            if hr != 0xFF && hr != 0 {
                readings.push(SensorReading::HeartRate {
                    bpm: hr,
                    timestamp,
                    epoch_ms,
                    device_id: did,
                });
            }
        }

        readings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_hr() {
        let decoder = AntDecoder::new();
        let data: [u8; 8] = [0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 142];
        let reading = decoder.decode_hr(&data, "test").unwrap();
        match reading {
            SensorReading::HeartRate { bpm, .. } => assert_eq!(bpm, 142),
            _ => panic!("Expected HeartRate"),
        }
    }

    #[test]
    fn test_decode_hr_zero_bpm() {
        let decoder = AntDecoder::new();
        let data: [u8; 8] = [0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0];
        assert!(decoder.decode_hr(&data, "test").is_none());
    }

    #[test]
    fn decode_power_first_sample_returns_instant_power() {
        let mut decoder = AntDecoder::new();
        // First message returns instant power immediately (bytes 6-7 = 200W)
        let data1: [u8; 8] = [0x10, 1, 0, 0, 0, 0, 200, 0];
        let r1 = decoder.decode_power(&data1, "test").unwrap();
        match r1 {
            SensorReading::Power { watts, pedal_balance, .. } => {
                assert_eq!(watts, 200);
                assert_eq!(pedal_balance, None);
            }
            _ => panic!("Expected Power"),
        }

        // Second message with new event count
        let data2: [u8; 8] = [0x10, 2, 0, 0, 200, 0, 250, 0]; // 250W
        let r2 = decoder.decode_power(&data2, "test").unwrap();
        match r2 {
            SensorReading::Power { watts, pedal_balance, .. } => {
                assert_eq!(watts, 250);
                assert_eq!(pedal_balance, None);
            }
            _ => panic!("Expected Power"),
        }
    }

    #[test]
    fn test_decode_cadence() {
        let mut decoder = AntDecoder::new();
        // First message initializes
        let data1: [u8; 8] = [0x00, 0, 0, 0, 0, 0, 0, 0];
        assert!(decoder.decode_cadence(&data1, "test").is_none());

        // 1 revolution in 1024 ticks (1 second) = 60 RPM
        let data2: [u8; 8] = [0x00, 0, 0, 0, 0x00, 0x04, 0x01, 0x00]; // time=1024, revs=1
        let reading = decoder.decode_cadence(&data2, "test").unwrap();
        match reading {
            SensorReading::Cadence { rpm, .. } => {
                assert!((rpm - 60.0).abs() < 1.0);
            }
            _ => panic!("Expected Cadence"),
        }
    }

    #[test]
    fn test_decode_fec_trainer_page_0x19() {
        let decoder = AntDecoder::new();
        // Page 0x19: event=5, cadence=90, accum_power_lo=100, accum_power_hi=0
        // instant_power bytes [5..6] LE: 0x00FA = 250, masked to 12 bits = 250
        let data: [u8; 8] = [0x19, 5, 90, 0, 100, 0xFA, 0x00, 0x00];
        let readings = decoder.decode_fec_trainer(&data, "test");
        assert_eq!(readings.len(), 2, "should produce cadence + power");

        // First reading: cadence = data[2] = 90 RPM
        match &readings[0] {
            SensorReading::Cadence { rpm, .. } => {
                assert!((rpm - 90.0).abs() < 0.01, "cadence should be 90 RPM, got {}", rpm);
            }
            other => panic!("expected Cadence, got {:?}", other),
        }

        // Second reading: instant_power = u16 LE [0xFA, 0x00] & 0x0FFF = 250W
        match &readings[1] {
            SensorReading::Power { watts, pedal_balance, .. } => {
                assert_eq!(*watts, 250, "power should be 250W");
                assert_eq!(*pedal_balance, None, "FE-C has no pedal balance");
            }
            other => panic!("expected Power, got {:?}", other),
        }
    }

    #[test]
    fn decode_fec_trainer_page_0x19_cadence_0xff_omitted() {
        let decoder = AntDecoder::new();
        // cadence=0xFF means invalid/unavailable → should be omitted
        let data: [u8; 8] = [0x19, 5, 0xFF, 0, 100, 0xC8, 0x00, 0x00];
        let readings = decoder.decode_fec_trainer(&data, "test");
        assert_eq!(readings.len(), 1, "0xFF cadence should be omitted");
        match &readings[0] {
            SensorReading::Power { watts, .. } => {
                // 0x00C8 & 0x0FFF = 200W
                assert_eq!(*watts, 200);
            }
            other => panic!("expected Power, got {:?}", other),
        }
    }

    #[test]
    fn decode_fec_trainer_page_0x19_power_12bit_mask() {
        let decoder = AntDecoder::new();
        // Test that upper 4 bits of byte[6] are masked off
        // bytes [5..6] LE: [0xFF, 0xFF] → 0xFFFF & 0x0FFF = 4095W
        let data: [u8; 8] = [0x19, 5, 80, 0, 0, 0xFF, 0xFF, 0x00];
        let readings = decoder.decode_fec_trainer(&data, "test");
        assert_eq!(readings.len(), 2);
        match &readings[1] {
            SensorReading::Power { watts, .. } => {
                assert_eq!(*watts, 4095, "12-bit mask should cap at 4095");
            }
            other => panic!("expected Power, got {:?}", other),
        }
    }

    // ---- decode_cadence gap tests ----

    #[test]
    fn decode_cadence_above_200rpm_returns_none() {
        let mut decoder = AntDecoder::new();
        let init: [u8; 8] = [0x00, 0, 0, 0, 0x00, 0x00, 0x00, 0x00];
        decoder.decode_cadence(&init, "test");

        // 50 revs in 1 tick → rpm = (50 / (1/1024)) * 60 = way over 200
        let data: [u8; 8] = [0x00, 0, 0, 0, 0x01, 0x00, 0x32, 0x00];
        assert!(decoder.decode_cadence(&data, "test").is_none());
    }

    #[test]
    fn decode_cadence_u16_counter_wraparound() {
        let mut decoder = AntDecoder::new();
        // Init near u16 max: time=0xFFF0, revs=0xFFF0
        let init: [u8; 8] = [0x00, 0, 0, 0, 0xF0, 0xFF, 0xF0, 0xFF];
        decoder.decode_cadence(&init, "test");

        // Wrapped: time = 0xFFF0 + 0x0410 = 0x0400 (wrapping), revs = 0xFFF1 (+1)
        // time_diff = 0x0400 wrapping_sub 0xFFF0 → 0x0410 = 1040 ticks
        // rev_diff = 1
        // rpm = (1 / (1040/1024)) * 60 = 59.077
        let data: [u8; 8] = [0x00, 0, 0, 0, 0x00, 0x04, 0xF1, 0xFF];
        let reading = decoder.decode_cadence(&data, "test").unwrap();
        match reading {
            SensorReading::Cadence { rpm, .. } => {
                // time_diff = 0x0400 wrapping_sub 0xFFF0 = 0x0410 = 1040
                assert!((rpm - 59.077).abs() < 0.1, "expected ~59.077, got {}", rpm);
            }
            _ => panic!("Expected Cadence"),
        }
    }

    #[test]
    fn decode_power_first_sample_includes_pedal_balance() {
        let mut decoder = AntDecoder::new();
        // byte[2] = 0xB2: bit7 set (differentiated), bits 0-6 = 50 (50% right pedal)
        let data: [u8; 8] = [0x10, 1, 0xB2, 0, 0, 0, 180, 0];
        let r = decoder.decode_power(&data, "test").unwrap();
        match r {
            SensorReading::Power { watts, pedal_balance, .. } => {
                assert_eq!(watts, 180);
                assert_eq!(pedal_balance, Some(50));
            }
            _ => panic!("Expected Power"),
        }
    }

    // ---- decode_power gap tests ----

    #[test]
    fn decode_power_same_event_count_returns_none() {
        let mut decoder = AntDecoder::new();
        let data1: [u8; 8] = [0x10, 5, 0, 0, 0, 0, 200, 0];
        decoder.decode_power(&data1, "test"); // init

        // Same event count (5) → no new data → None
        let data2: [u8; 8] = [0x10, 5, 0, 0, 100, 0, 250, 0];
        assert!(decoder.decode_power(&data2, "test").is_none());
    }

    #[test]
    fn decode_power_pedal_balance_present() {
        let mut decoder = AntDecoder::new();
        let data1: [u8; 8] = [0x10, 1, 0, 0, 0, 0, 200, 0];
        decoder.decode_power(&data1, "test"); // init

        // byte[2] = 0x85: bit7 set (differentiated), bits 0-6 = 5 (5% right pedal)
        let data2: [u8; 8] = [0x10, 2, 0x85, 0, 200, 0, 250, 0];
        let reading = decoder.decode_power(&data2, "test").unwrap();
        match reading {
            SensorReading::Power { watts, pedal_balance, .. } => {
                assert_eq!(watts, 250);
                assert_eq!(pedal_balance, Some(5));
            }
            _ => panic!("Expected Power"),
        }
    }

    #[test]
    fn decode_power_wrong_page_returns_none() {
        let mut decoder = AntDecoder::new();
        // Page 0x12 instead of 0x10 → None
        let data: [u8; 8] = [0x12, 1, 0, 0, 0, 0, 200, 0];
        assert!(decoder.decode_power(&data, "test").is_none());
    }

    // ---- decode_fec_trainer page 0x10 tests ----

    #[test]
    fn decode_fec_page_0x10_speed_and_hr() {
        let decoder = AntDecoder::new();
        // Page 0x10, speed at bytes 4-5 = 5000 (5.0 m/s * 1000 = 18 km/h)
        // hr at byte 6 = 140
        let speed_bytes = 5000u16.to_le_bytes();
        let data: [u8; 8] = [0x10, 0, 0, 0, speed_bytes[0], speed_bytes[1], 140, 0];
        let readings = decoder.decode_fec_trainer(&data, "test");
        assert_eq!(readings.len(), 2);
        match &readings[0] {
            SensorReading::Speed { kmh, .. } => {
                // 5000 * 0.001 * 3.6 = 18.0
                assert!((kmh - 18.0).abs() < 0.01, "expected 18.0, got {}", kmh);
            }
            _ => panic!("Expected Speed"),
        }
        match &readings[1] {
            SensorReading::HeartRate { bpm, .. } => assert_eq!(*bpm, 140),
            _ => panic!("Expected HeartRate"),
        }
    }

    #[test]
    fn decode_fec_page_0x10_speed_sentinel_0xffff() {
        let decoder = AntDecoder::new();
        // speed=0xFFFF (invalid) → no Speed reading; hr=120 → 1 reading
        let data: [u8; 8] = [0x10, 0, 0, 0, 0xFF, 0xFF, 120, 0];
        let readings = decoder.decode_fec_trainer(&data, "test");
        assert_eq!(readings.len(), 1);
        match &readings[0] {
            SensorReading::HeartRate { bpm, .. } => assert_eq!(*bpm, 120),
            _ => panic!("Expected HeartRate"),
        }
    }

    // ---- decode_speed tests ----

    #[test]
    fn decode_speed_first_sample_initializes() {
        let mut decoder = AntDecoder::new();
        let data: [u8; 8] = [0x00, 0, 0, 0, 0x00, 0x04, 0x01, 0x00];
        assert!(decoder.decode_speed(&data, "test", 2105).is_none());
        assert!(decoder.speed_initialized);
    }

    #[test]
    fn decode_speed_computes_kmh() {
        let mut decoder = AntDecoder::new();
        // Init: time=0, revs=0
        let init: [u8; 8] = [0x00, 0, 0, 0, 0x00, 0x00, 0x00, 0x00];
        decoder.decode_speed(&init, "test", 2105);

        // 1 rev in 1024 ticks (1 second) with 2105mm wheel
        // speed = (1 * 2.105m / 1.0s) * 3.6 = 7.578 km/h
        let data: [u8; 8] = [0x00, 0, 0, 0, 0x00, 0x04, 0x01, 0x00];
        let reading = decoder.decode_speed(&data, "test", 2105).unwrap();
        match reading {
            SensorReading::Speed { kmh, .. } => {
                assert!((kmh - 7.578).abs() < 0.01, "expected ~7.578, got {}", kmh);
            }
            _ => panic!("Expected Speed"),
        }
    }

    #[test]
    fn decode_speed_u16_wraparound() {
        let mut decoder = AntDecoder::new();
        // Init near u16 max: time=0xFFF0, revs=0xFFF0
        let init: [u8; 8] = [0x00, 0, 0, 0, 0xF0, 0xFF, 0xF0, 0xFF];
        decoder.decode_speed(&init, "test", 2105);

        // Wrap: time=0x03F0 (delta=0x0400=1024 ticks = 1s), revs=0xFFF1 (delta=1)
        let data: [u8; 8] = [0x00, 0, 0, 0, 0xF0, 0x03, 0xF1, 0xFF];
        let reading = decoder.decode_speed(&data, "test", 2105).unwrap();
        match reading {
            SensorReading::Speed { kmh, .. } => {
                assert!((kmh - 7.578).abs() < 0.01, "expected ~7.578, got {}", kmh);
            }
            _ => panic!("Expected Speed"),
        }
    }

    #[test]
    fn decode_speed_above_120kmh_returns_none() {
        let mut decoder = AntDecoder::new();
        let init: [u8; 8] = [0x00, 0, 0, 0, 0x00, 0x00, 0x00, 0x00];
        decoder.decode_speed(&init, "test", 2105);

        // Huge rev count in tiny time → exceeds 120 km/h
        // 200 revs in 1 tick: (200 * 2.105m / (1/1024)s) * 3.6 = way over 120
        let data: [u8; 8] = [0x00, 0, 0, 0, 0x01, 0x00, 0xC8, 0x00];
        assert!(decoder.decode_speed(&data, "test", 2105).is_none());
    }

    #[test]
    fn decode_speed_zero_time_diff_returns_none() {
        let mut decoder = AntDecoder::new();
        let init: [u8; 8] = [0x00, 0, 0, 0, 0x00, 0x04, 0x01, 0x00];
        decoder.decode_speed(&init, "test", 2105);

        // Same time, different revs → time_diff=0 → None
        let data: [u8; 8] = [0x00, 0, 0, 0, 0x00, 0x04, 0x02, 0x00];
        assert!(decoder.decode_speed(&data, "test", 2105).is_none());
    }

    #[test]
    fn decode_speed_different_wheel_circumference() {
        let mut decoder = AntDecoder::new();
        let init: [u8; 8] = [0x00, 0, 0, 0, 0x00, 0x00, 0x00, 0x00];
        decoder.decode_speed(&init, "test", 2290);

        // 1 rev in 1024 ticks (1s) with 2290mm wheel
        // speed = (1 * 2.290m / 1.0s) * 3.6 = 8.244 km/h
        let data: [u8; 8] = [0x00, 0, 0, 0, 0x00, 0x04, 0x01, 0x00];
        let reading = decoder.decode_speed(&data, "test", 2290).unwrap();
        match reading {
            SensorReading::Speed { kmh, .. } => {
                assert!((kmh - 8.244).abs() < 0.01, "expected ~8.244, got {}", kmh);
            }
            _ => panic!("Expected Speed"),
        }
    }

    #[test]
    fn decode_fec_page_0x10_hr_sentinels() {
        let decoder = AntDecoder::new();
        let speed_bytes = 5000u16.to_le_bytes();

        // hr=0xFF → no HR reading
        let data1: [u8; 8] = [0x10, 0, 0, 0, speed_bytes[0], speed_bytes[1], 0xFF, 0];
        let readings1 = decoder.decode_fec_trainer(&data1, "test");
        assert_eq!(readings1.len(), 1); // speed only
        assert!(matches!(&readings1[0], SensorReading::Speed { .. }));

        // hr=0 → no HR reading
        let data2: [u8; 8] = [0x10, 0, 0, 0, speed_bytes[0], speed_bytes[1], 0, 0];
        let readings2 = decoder.decode_fec_trainer(&data2, "test");
        assert_eq!(readings2.len(), 1); // speed only
        assert!(matches!(&readings2[0], SensorReading::Speed { .. }));
    }
}
