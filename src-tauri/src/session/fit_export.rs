use std::collections::BTreeMap;

use super::gap;
use super::types::SessionSummary;
use crate::device::types::SensorReading;
use crate::error::AppError;

/// FIT epoch offset: seconds between Unix epoch (1970-01-01) and FIT epoch (1989-12-31 00:00:00 UTC)
const FIT_EPOCH_OFFSET: i64 = 631065600;

/// CRC-16/ARC lookup table (polynomial 0xA001, reflected)
fn fit_crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        for bit in 0..8 {
            let b = (byte >> bit) & 1;
            let c = crc & 1;
            crc >>= 1;
            if (b ^ c as u8) != 0 {
                crc ^= 0xA001;
            }
        }
    }
    crc
}

fn unix_to_fit_timestamp(epoch_ms: u64) -> u32 {
    let unix_secs = (epoch_ms / 1000) as i64;
    (unix_secs - FIT_EPOCH_OFFSET).max(0) as u32
}

fn datetime_to_fit_timestamp(dt: &chrono::DateTime<chrono::Utc>) -> u32 {
    let unix_secs = dt.timestamp();
    (unix_secs - FIT_EPOCH_OFFSET).max(0) as u32
}

struct FitWriter {
    data: Vec<u8>,
}

impl FitWriter {
    fn new() -> Self {
        // Reserve space for 14-byte header
        Self {
            data: vec![0u8; 14],
        }
    }

    /// Write a definition message for a given local message type.
    fn write_definition(&mut self, local_msg: u8, global_msg: u16, fields: &[(u8, u8, u8)]) {
        // Record header: definition message (bit 6 set)
        self.data.push(0x40 | (local_msg & 0x0F));
        self.data.push(0); // reserved
        self.data.push(0); // architecture: little-endian
        self.data.extend_from_slice(&global_msg.to_le_bytes());
        self.data.push(fields.len() as u8);
        for &(field_def_num, size, base_type) in fields {
            self.data.push(field_def_num);
            self.data.push(size);
            self.data.push(base_type);
        }
    }

    /// Write a data message for a given local message type.
    fn write_data(&mut self, local_msg: u8, field_data: &[u8]) {
        self.data.push(local_msg & 0x0F);
        self.data.extend_from_slice(field_data);
    }

    /// Finalize the FIT file: write header and append CRC.
    fn finish(mut self) -> Vec<u8> {
        let data_size = (self.data.len() - 14) as u32;

        // Write 14-byte header
        self.data[0] = 14; // header size
        self.data[1] = 0x20; // protocol version 2.0
        let profile_version: u16 = 2132; // profile version 21.32
        self.data[2..4].copy_from_slice(&profile_version.to_le_bytes());
        self.data[4..8].copy_from_slice(&data_size.to_le_bytes());
        self.data[8] = b'.';
        self.data[9] = b'F';
        self.data[10] = b'I';
        self.data[11] = b'T';
        let header_crc = fit_crc16(&self.data[0..12]);
        self.data[12..14].copy_from_slice(&header_crc.to_le_bytes());

        // Append file CRC (over entire file including header)
        let file_crc = fit_crc16(&self.data);
        self.data.extend_from_slice(&file_crc.to_le_bytes());

        self.data
    }
}

// FIT profile 21.217 enums: event 0 = timer; event_type 0 = start, 4 = stop_all.
const EVENT_TIMER: u8 = 0;
const EVENT_TYPE_START: u8 = 0;
const EVENT_TYPE_STOP_ALL: u8 = 4;

/// Write a timer event using the event definition (local message 2).
fn write_timer_event(w: &mut FitWriter, ts: u32, event_type: u8) {
    let mut evt = Vec::with_capacity(6);
    evt.extend_from_slice(&ts.to_le_bytes());
    evt.push(EVENT_TIMER);
    evt.push(event_type);
    w.write_data(2, &evt);
}

/// Per-epoch-second aggregate of all channels, for the merged record stream.
#[derive(Default)]
struct SecondAgg {
    first_ms: u64,
    last_ms: u64,
    power_sum: u64,
    power_n: u32,
    power_last_ms: u64,
    /// (sample ms, value) of the last sample in this second
    hr: Option<(u64, u8)>,
    cadence: Option<(u64, u8)>,
    speed: Option<(u64, u16)>,
    /// Cumulative distance (m × 100) at the last speed sample in this second
    distance_m100: Option<u32>,
}

/// Group readings by epoch-second (sorted by time). Distance is integrated
/// from speed with the shared gap rule: each interval counts at most 5 s.
fn aggregate_seconds(readings: &[SensorReading]) -> BTreeMap<u64, SecondAgg> {
    let mut sorted: Vec<&SensorReading> = readings
        .iter()
        .filter(|r| !matches!(r, SensorReading::TrainerCommand { .. }))
        .collect();
    sorted.sort_by_key(|r| r.epoch_ms());

    let mut seconds: BTreeMap<u64, SecondAgg> = BTreeMap::new();
    let mut distance_m: f64 = 0.0;
    let mut prev_speed: Option<(u64, f64)> = None; // (ms, m/s)

    for r in sorted {
        let ms = r.epoch_ms();
        let agg = seconds.entry(ms / 1000).or_insert_with(|| SecondAgg {
            first_ms: ms,
            ..Default::default()
        });
        agg.last_ms = ms;
        match r {
            SensorReading::Power { watts, .. } => {
                agg.power_sum += *watts as u64;
                agg.power_n += 1;
                agg.power_last_ms = ms;
            }
            SensorReading::HeartRate { bpm, .. } => agg.hr = Some((ms, *bpm)),
            SensorReading::Cadence { rpm, .. } => {
                agg.cadence = Some((ms, rpm.clamp(0.0, 254.0).round() as u8))
            }
            SensorReading::Speed { kmh, .. } => {
                let mps = (*kmh as f64 / 3.6).max(0.0);
                if let Some((p_ms, p_mps)) = prev_speed {
                    distance_m += p_mps * gap::capped_interval_ms(p_ms, ms) as f64 / 1000.0;
                }
                prev_speed = Some((ms, mps));
                agg.speed = Some((ms, (mps * 1000.0).round().min(65534.0) as u16));
                agg.distance_m100 = Some((distance_m * 100.0).round() as u32);
            }
            SensorReading::TrainerCommand { .. } => {}
        }
    }
    seconds
}

/// Export a session as a FIT file.
pub fn export_fit(summary: &SessionSummary, readings: &[SensorReading]) -> Result<Vec<u8>, AppError> {
    let mut w = FitWriter::new();
    let start_ts = datetime_to_fit_timestamp(&summary.start_time);

    // --- file_id message (local 0, global 0) ---
    w.write_definition(0, 0, &[
        (0, 1, 0),   // type: enum
        (1, 2, 132), // manufacturer: uint16
        (2, 2, 132), // product: uint16
        (3, 4, 140), // serial_number: uint32z
        (4, 4, 134), // time_created: uint32
    ]);
    let mut file_id_data = Vec::new();
    file_id_data.push(4); // type = activity
    file_id_data.extend_from_slice(&255u16.to_le_bytes()); // manufacturer = development
    file_id_data.extend_from_slice(&1u16.to_le_bytes()); // product
    file_id_data.extend_from_slice(&0u32.to_le_bytes()); // serial
    file_id_data.extend_from_slice(&start_ts.to_le_bytes()); // time_created
    w.write_data(0, &file_id_data);

    // --- device_info message (local 1, global 23) ---
    // Field numbers per Garmin FIT SDK Profile 21.217 (@garmin/fitsdk profile.js):
    // 2 = manufacturer (uint16); 3 is serial_number (uint32z), not manufacturer.
    w.write_definition(1, 23, &[
        (253, 4, 134), // timestamp: uint32
        (0, 1, 2),     // device_index: uint8
        (2, 2, 132),   // manufacturer: uint16
        (4, 2, 132),   // product: uint16
        (5, 2, 132),   // software_version: uint16
        (27, 16, 7),   // product_name: string (16 bytes)
    ]);
    let mut dev_data = Vec::new();
    dev_data.extend_from_slice(&start_ts.to_le_bytes());
    dev_data.push(0); // device_index = 0 (creator)
    dev_data.extend_from_slice(&255u16.to_le_bytes()); // manufacturer = development
    dev_data.extend_from_slice(&1u16.to_le_bytes()); // product
    dev_data.extend_from_slice(&100u16.to_le_bytes()); // software_version = v1.0.0
    let mut product_name = [0u8; 16];
    let name_bytes = b"My Training App";
    product_name[..name_bytes.len()].copy_from_slice(name_bytes);
    // byte 15 already 0 (null terminator)
    dev_data.extend_from_slice(&product_name);
    w.write_data(1, &dev_data);

    // --- event definition (local 2, global 21) — reused for start and stop ---
    w.write_definition(2, 21, &[
        (253, 4, 134), // timestamp: uint32
        (0, 1, 0),     // event: enum
        (1, 1, 0),     // event_type: enum
    ]);

    // Start event (timer start) — before the first record, per the FIT cookbook
    write_timer_event(&mut w, start_ts, EVENT_TYPE_START);

    // --- record messages (local 3, global 20) ---
    w.write_definition(3, 20, &[
        (253, 4, 134), // timestamp: uint32
        (7, 2, 132),   // power: uint16
        (3, 1, 2),     // heart_rate: uint8
        (4, 1, 2),     // cadence: uint8
        (6, 2, 132),   // speed: uint16 (m/s * 1000)
        (5, 4, 134),   // distance: uint32 (m * 100)
    ]);

    // Records: one per epoch-second that has any sensor sample (merged 1 Hz
    // stream). "Record messages should not be encoded to the file when the
    // timer is paused" (Garmin FIT cookbook, Encoding Activity Files), so seconds
    // without data are skipped. Each channel is held for ≤ 5 s after its last
    // sample and written as invalid after that (shared gap rule).
    let seconds = aggregate_seconds(readings);
    let max_speed_ms1000 = seconds
        .values()
        .filter_map(|a| a.speed.map(|(_, v)| v))
        .max()
        .unwrap_or(0);

    let mut last_power: Option<(u64, u16)> = None;
    let mut last_hr: Option<(u64, u8)> = None;
    let mut last_cadence: Option<(u64, u8)> = None;
    let mut last_speed: Option<(u64, u16)> = None;
    let mut last_distance_m100: Option<u32> = None;
    let mut prev: Option<(u64, u64)> = None; // (sec, last sample ms) of previous record
    let mut last_record_ts: Option<u32> = None;

    fn fresh<T: Copy>(last: Option<(u64, T)>, at_ms: u64) -> Option<T> {
        last.filter(|(ms, _)| !gap::is_gap(*ms, at_ms)).map(|(_, v)| v)
    }

    for (&sec, agg) in &seconds {
        let ts = unix_to_fit_timestamp(sec * 1000);
        // Pause or dropout > 5 s between records: timer stop/start pair. Pauses
        // ≤ 5 s are indistinguishable from sensor holes and get no events.
        if let Some((prev_sec, prev_last_ms)) = prev {
            if gap::is_gap(prev_last_ms, agg.first_ms) {
                write_timer_event(&mut w, unix_to_fit_timestamp(prev_sec * 1000), EVENT_TYPE_STOP_ALL);
                write_timer_event(&mut w, ts, EVENT_TYPE_START);
            }
        }
        if agg.power_n > 0 {
            let mean = (agg.power_sum as f64 / agg.power_n as f64).round() as u16;
            last_power = Some((agg.power_last_ms, mean));
        }
        last_hr = agg.hr.or(last_hr);
        last_cadence = agg.cadence.or(last_cadence);
        last_speed = agg.speed.or(last_speed);
        last_distance_m100 = agg.distance_m100.or(last_distance_m100);

        let at_ms = sec * 1000;
        let mut rec = Vec::with_capacity(17);
        rec.extend_from_slice(&ts.to_le_bytes());
        rec.extend_from_slice(&fresh(last_power, at_ms).unwrap_or(0xFFFF).to_le_bytes());
        rec.push(fresh(last_hr, at_ms).unwrap_or(0xFF));
        rec.push(fresh(last_cadence, at_ms).unwrap_or(0xFF));
        rec.extend_from_slice(&fresh(last_speed, at_ms).unwrap_or(0xFFFF).to_le_bytes());
        rec.extend_from_slice(&last_distance_m100.unwrap_or(0xFFFF_FFFF).to_le_bytes());
        w.write_data(3, &rec);

        prev = Some((sec, agg.last_ms));
        last_record_ts = Some(ts);
    }

    // Session end can't precede the last record; elapsed = wall-clock span,
    // timer = active duration (never more than elapsed).
    let end_ts = last_record_ts
        .unwrap_or(start_ts)
        .max(start_ts + summary.duration_secs as u32);
    let elapsed_secs = end_ts - start_ts;
    let elapsed_ms = elapsed_secs.saturating_mul(1000);
    let timer_ms = (summary.duration_secs.min(elapsed_secs as u64) * 1000) as u32;

    // Stop event (timer stop_all)
    write_timer_event(&mut w, end_ts, EVENT_TYPE_STOP_ALL);

    // --- lap message (local 4, global 19) ---
    w.write_definition(4, 19, &[
        (253, 4, 134), // timestamp
        (2, 4, 134),   // start_time
        (7, 4, 134),   // total_elapsed_time (s * 1000)
        (8, 4, 134),   // total_timer_time (s * 1000)
        (25, 1, 0),    // sport: enum
        (39, 1, 0),    // sub_sport: enum
    ]);
    let mut lap_data = Vec::new();
    lap_data.extend_from_slice(&end_ts.to_le_bytes());
    lap_data.extend_from_slice(&start_ts.to_le_bytes());
    lap_data.extend_from_slice(&elapsed_ms.to_le_bytes());
    lap_data.extend_from_slice(&timer_ms.to_le_bytes());
    lap_data.push(2); // sport = cycling
    lap_data.push(6); // sub_sport = indoor_cycling
    w.write_data(4, &lap_data);

    // --- session message (local 5, global 18) ---
    let total_distance = summary
        .distance_km
        .map(|km| (km * 100_000.0) as u32)
        .unwrap_or(last_distance_m100.unwrap_or(0xFFFF_FFFF));
    let total_calories = summary
        .work_kj
        .map(|kj| kj.round() as u16)
        .unwrap_or(0xFFFF);
    let avg_speed_ms1000 = summary
        .avg_speed
        .map(|kmh| (kmh / 3.6 * 1000.0) as u16)
        .unwrap_or(0xFFFF);
    let max_speed_val = if max_speed_ms1000 > 0 {
        max_speed_ms1000
    } else {
        0xFFFF
    };
    let tss_x10 = summary.tss.map(|t| (t * 10.0) as u16).unwrap_or(0xFFFF);
    let if_x1000 = summary
        .intensity_factor
        .map(|i| (i * 1000.0) as u16)
        .unwrap_or(0xFFFF);

    w.write_definition(5, 18, &[
        (253, 4, 134), // timestamp
        (2, 4, 134),   // start_time
        (7, 4, 134),   // total_elapsed_time
        (8, 4, 134),   // total_timer_time
        (5, 1, 0),     // sport: enum
        (6, 1, 0),     // sub_sport: enum
        (9, 4, 134),   // total_distance: uint32 (m * 100)
        (11, 2, 132),  // total_calories: uint16
        (14, 2, 132),  // avg_speed: uint16 (m/s * 1000)
        (15, 2, 132),  // max_speed: uint16 (m/s * 1000)
        (16, 1, 2),    // avg_heart_rate: uint8
        (17, 1, 2),    // max_heart_rate: uint8
        (18, 1, 2),    // avg_cadence: uint8
        (20, 2, 132),  // avg_power: uint16
        (21, 2, 132),  // max_power: uint16
        (34, 2, 132),  // normalized_power: uint16
        (35, 2, 132),  // tss: uint16 (* 10)
        (36, 2, 132),  // intensity_factor: uint16 (* 1000)
        // Per FIT SDK Profile 21.217: threshold_power is session field 45 (uint16, W).
        // Field 38 is end_position_lat (sint32) and must not be used here.
        (45, 2, 132),  // threshold_power: uint16
    ]);
    let mut sess_data = Vec::new();
    sess_data.extend_from_slice(&end_ts.to_le_bytes());
    sess_data.extend_from_slice(&start_ts.to_le_bytes());
    sess_data.extend_from_slice(&elapsed_ms.to_le_bytes());
    sess_data.extend_from_slice(&timer_ms.to_le_bytes());
    sess_data.push(2); // sport = cycling
    sess_data.push(6); // sub_sport = indoor_cycling
    sess_data.extend_from_slice(&total_distance.to_le_bytes());
    sess_data.extend_from_slice(&total_calories.to_le_bytes());
    sess_data.extend_from_slice(&avg_speed_ms1000.to_le_bytes());
    sess_data.extend_from_slice(&max_speed_val.to_le_bytes());
    sess_data.push(summary.avg_hr.unwrap_or(0xFF));
    sess_data.push(summary.max_hr.unwrap_or(0xFF));
    sess_data.push(
        summary
            .avg_cadence
            .map(|c| c.round() as u8)
            .unwrap_or(0xFF),
    );
    sess_data.extend_from_slice(&summary.avg_power.unwrap_or(0xFFFF).to_le_bytes());
    sess_data.extend_from_slice(&summary.max_power.unwrap_or(0xFFFF).to_le_bytes());
    sess_data.extend_from_slice(&summary.normalized_power.unwrap_or(0xFFFF).to_le_bytes());
    sess_data.extend_from_slice(&tss_x10.to_le_bytes());
    sess_data.extend_from_slice(&if_x1000.to_le_bytes());
    sess_data.extend_from_slice(&summary.ftp.unwrap_or(0xFFFF).to_le_bytes());
    w.write_data(5, &sess_data);

    // --- activity message (local 6, global 34) ---
    w.write_definition(6, 34, &[
        (253, 4, 134), // timestamp: uint32
        (0, 4, 134),   // total_timer_time: uint32 (ms)
        (1, 2, 132),   // num_sessions: uint16
        (2, 1, 0),     // type: enum
        (3, 1, 0),     // event: enum
        (4, 1, 0),     // event_type: enum
    ]);
    let mut act_data = Vec::new();
    act_data.extend_from_slice(&end_ts.to_le_bytes());
    act_data.extend_from_slice(&timer_ms.to_le_bytes());
    act_data.extend_from_slice(&1u16.to_le_bytes()); // num_sessions = 1
    act_data.push(0); // type = manual
    act_data.push(26); // event = activity
    act_data.push(1); // event_type = stop
    w.write_data(6, &act_data);

    Ok(w.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_summary() -> SessionSummary {
        SessionSummary {
            id: "test-1".to_string(),
            start_time: chrono::DateTime::parse_from_rfc3339("2024-06-15T10:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            duration_secs: 3600,
            ftp: Some(200),
            avg_power: Some(180),
            max_power: Some(300),
            normalized_power: Some(190),
            tss: Some(75.0),
            intensity_factor: Some(0.95),
            avg_hr: Some(145),
            max_hr: Some(170),
            avg_cadence: Some(90.0),
            avg_speed: Some(30.0),
            work_kj: None,
            variability_index: None,
            distance_km: None,
            title: None,
            activity_type: None,
            rpe: None,
            notes: None,
            zones: None,
        }
    }

    #[test]
    fn fit_file_starts_with_header() {
        let summary = make_summary();
        let data = export_fit(&summary, &[]).unwrap();
        assert!(data.len() >= 14);
        assert_eq!(data[0], 14); // header size
        assert_eq!(&data[8..12], b".FIT");
    }

    #[test]
    fn fit_timestamp_conversion() {
        // 2024-06-15T10:00:00Z = Unix 1718445600
        // FIT = 1718445600 - 631065600 = 1087380000
        let ts = unix_to_fit_timestamp(1718445600_000);
        assert_eq!(ts, 1087380000);
    }

    #[test]
    fn fit_crc16_empty_is_zero() {
        assert_eq!(fit_crc16(&[]), 0x0000);
    }

    #[test]
    fn fit_crc16_check_value() {
        // CRC-16/ARC standard check value: CRC of "123456789" = 0xBB3D
        let crc = fit_crc16(b"123456789");
        assert_eq!(crc, 0xBB3D);
    }

    #[test]
    fn fit_crc16_self_check_yields_zero() {
        // CRC-16/ARC property: CRC over (data || crc_le) == 0
        let data = b"some payload";
        let crc = fit_crc16(data);
        let mut extended = data.to_vec();
        extended.extend_from_slice(&crc.to_le_bytes());
        assert_eq!(fit_crc16(&extended), 0);
    }

    #[test]
    fn fit_header_crc_matches_recomputed() {
        let data = export_fit(&make_summary(), &[]).unwrap();
        let stored_crc = u16::from_le_bytes([data[12], data[13]]);
        let recomputed = fit_crc16(&data[0..12]);
        assert_eq!(stored_crc, recomputed);
    }

    #[test]
    fn fit_file_crc_matches_recomputed() {
        let data = export_fit(&make_summary(), &[]).unwrap();
        let len = data.len();
        let stored_crc = u16::from_le_bytes([data[len - 2], data[len - 1]]);
        let recomputed = fit_crc16(&data[..len - 2]);
        assert_eq!(stored_crc, recomputed);
    }

    #[test]
    fn fit_file_crc_self_check_yields_zero() {
        // CRC over entire file including appended CRC should be 0
        let data = export_fit(&make_summary(), &[]).unwrap();
        assert_eq!(fit_crc16(&data), 0);
    }

    #[test]
    fn fit_export_with_readings() {
        let summary = summary_with_duration(2);
        let readings = vec![
            SensorReading::Power {
                watts: 200,
                timestamp: None,
                epoch_ms: 1718445600_000,
                device_id: "test".to_string(),
                pedal_balance: None,
            },
            SensorReading::HeartRate {
                bpm: 140,
                timestamp: None,
                epoch_ms: 1718445601_000,
                device_id: "test".to_string(),
            },
            SensorReading::Power {
                watts: 250,
                timestamp: None,
                epoch_ms: 1718445602_000,
                device_id: "test".to_string(),
                pedal_balance: None,
            },
        ];
        let data = export_fit(&summary, &readings).unwrap();
        // Should be larger than just header (14) + CRC (2)
        assert!(data.len() > 16, "FIT file too small: {} bytes", data.len());
    }

    #[test]
    fn fit_export_empty_readings() {
        let summary = make_summary();
        let data = export_fit(&summary, &[]).unwrap();
        // Still valid: file_id + session + lap, just no records
        assert!(data.len() > 16, "FIT file too small: {} bytes", data.len());
        // Check header magic
        assert_eq!(&data[8..12], b".FIT");
    }

    // --- FIT test parser helper ---

    struct FieldDef {
        num: u8,
        size: u8,
        _base_type: u8,
    }

    struct MsgDef {
        global_msg: u16,
        fields: Vec<FieldDef>,
    }

    struct FitMsg {
        global_msg: u16,
        field_bytes: Vec<(u8, Vec<u8>)>, // (field_num, raw bytes)
    }

    impl FitMsg {
        fn field_u8(&self, num: u8) -> Option<u8> {
            self.field_bytes
                .iter()
                .find(|(n, _)| *n == num)
                .map(|(_, b)| b[0])
        }
        fn field_u16(&self, num: u8) -> Option<u16> {
            self.field_bytes
                .iter()
                .find(|(n, _)| *n == num)
                .map(|(_, b)| u16::from_le_bytes([b[0], b[1]]))
        }
        fn field_u32(&self, num: u8) -> Option<u32> {
            self.field_bytes
                .iter()
                .find(|(n, _)| *n == num)
                .map(|(_, b)| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        }
        fn field_string(&self, num: u8) -> Option<String> {
            self.field_bytes.iter().find(|(n, _)| *n == num).map(|(_, b)| {
                let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
                String::from_utf8_lossy(&b[..end]).to_string()
            })
        }
    }

    fn parse_fit_messages(data: &[u8]) -> Vec<FitMsg> {
        let header_size = data[0] as usize;
        let data_size = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
        let mut pos = header_size;
        let end = header_size + data_size;
        let mut defs: std::collections::HashMap<u8, MsgDef> = std::collections::HashMap::new();
        let mut msgs = Vec::new();

        while pos < end {
            let header = data[pos];
            pos += 1;
            let local_msg = header & 0x0F;
            if header & 0x40 != 0 {
                // Definition message
                pos += 1; // reserved
                pos += 1; // architecture
                let global_msg = u16::from_le_bytes([data[pos], data[pos + 1]]);
                pos += 2;
                let num_fields = data[pos] as usize;
                pos += 1;
                let mut fields = Vec::new();
                for _ in 0..num_fields {
                    fields.push(FieldDef {
                        num: data[pos],
                        size: data[pos + 1],
                        _base_type: data[pos + 2],
                    });
                    pos += 3;
                }
                defs.insert(local_msg, MsgDef { global_msg, fields });
            } else {
                // Data message
                let def = &defs[&local_msg];
                let mut field_bytes = Vec::new();
                for f in &def.fields {
                    let bytes = data[pos..pos + f.size as usize].to_vec();
                    field_bytes.push((f.num, bytes));
                    pos += f.size as usize;
                }
                msgs.push(FitMsg {
                    global_msg: def.global_msg,
                    field_bytes,
                });
            }
        }
        msgs
    }

    #[test]
    fn fit_export_includes_device_info() {
        let data = export_fit(&make_summary(), &[]).unwrap();
        let msgs = parse_fit_messages(&data);
        let dev = msgs.iter().find(|m| m.global_msg == 23).expect("no device_info message");
        assert_eq!(dev.field_u16(2), Some(255), "manufacturer should be 255 (development)");
        assert_eq!(
            dev.field_string(27).as_deref(),
            Some("My Training App"),
            "product_name mismatch"
        );
    }

    #[test]
    fn fit_export_includes_sport_cycling() {
        let data = export_fit(&make_summary(), &[]).unwrap();
        let msgs = parse_fit_messages(&data);
        let session = msgs.iter().find(|m| m.global_msg == 18).expect("no session message");
        assert_eq!(session.field_u8(5), Some(2), "sport should be 2 (cycling)");
        assert_eq!(session.field_u8(6), Some(6), "sub_sport should be 6 (indoor_cycling)");
    }

    #[test]
    fn fit_export_includes_activity_message() {
        let data = export_fit(&make_summary(), &[]).unwrap();
        let msgs = parse_fit_messages(&data);
        let act = msgs.iter().find(|m| m.global_msg == 34).expect("no activity message");
        assert_eq!(act.field_u16(1), Some(1), "num_sessions should be 1");
        assert_eq!(act.field_u8(3), Some(26), "event should be 26 (activity)");
    }

    #[test]
    fn fit_export_distance_from_speed_readings() {
        let summary = summary_with_duration(10);
        let base_ms: u64 = 1718445600_000;
        // 10 seconds of speed readings at 36 km/h = 10 m/s
        // Expected distance: 10m/s * 10s = 100m => 100 * 100 = 10000 (m*100)
        let mut readings = Vec::new();
        for i in 0..=10 {
            readings.push(SensorReading::Speed {
                kmh: 36.0,
                timestamp: None,
                epoch_ms: base_ms + i * 1000,
                device_id: "test".to_string(),
            });
            readings.push(SensorReading::Power {
                watts: 200,
                timestamp: None,
                epoch_ms: base_ms + i * 1000,
                device_id: "test".to_string(),
                pedal_balance: None,
            });
        }
        let data = export_fit(&summary, &readings).unwrap();
        let msgs = parse_fit_messages(&data);
        let records: Vec<_> = msgs.iter().filter(|m| m.global_msg == 20).collect();
        let last_record = records.last().expect("no record messages");
        let distance = last_record.field_u32(5).expect("no distance field");
        // 10 intervals of 10 m/s * 1s * 100 = 10000
        assert_eq!(distance, 10000, "cumulative distance should be ~10000 (100m * 100)");
    }

    #[test]
    fn fit_export_session_summary_fields() {
        let mut summary = make_summary();
        summary.max_hr = Some(170);
        summary.avg_cadence = Some(90.0);
        summary.tss = Some(75.0);
        summary.intensity_factor = Some(0.95);
        summary.ftp = Some(200);
        summary.work_kj = Some(648.0);
        summary.distance_km = Some(30.0);

        let data = export_fit(&summary, &[]).unwrap();
        let msgs = parse_fit_messages(&data);
        let session = msgs.iter().find(|m| m.global_msg == 18).expect("no session");

        assert_eq!(session.field_u8(17), Some(170), "max_hr");
        assert_eq!(session.field_u8(18), Some(90), "avg_cadence");
        assert_eq!(session.field_u16(35), Some(750), "tss * 10");
        assert_eq!(session.field_u16(36), Some(950), "intensity_factor * 1000");
        assert_eq!(session.field_u16(45), Some(200), "threshold_power (ftp)");
        assert_eq!(session.field_u16(11), Some(648), "total_calories");
        // 30 km * 100_000 = 3_000_000
        assert_eq!(session.field_u32(9), Some(3_000_000), "total_distance");
    }

    #[test]
    fn fit_export_event_start_and_stop() {
        let data = export_fit(&make_summary(), &[]).unwrap();
        let msgs = parse_fit_messages(&data);
        let events: Vec<_> = msgs.iter().filter(|m| m.global_msg == 21).collect();
        assert_eq!(events.len(), 2, "expected exactly 2 event messages");
        // First event: start
        assert_eq!(events[0].field_u8(0), Some(0), "first event should be timer (0)");
        assert_eq!(events[0].field_u8(1), Some(0), "first event_type should be start (0)");
        // Second event: stop_all
        assert_eq!(events[1].field_u8(0), Some(0), "second event should be timer (0)");
        assert_eq!(events[1].field_u8(1), Some(4), "second event_type should be stop_all (4)");
    }

    #[test]
    fn fit_session_threshold_power_is_field_45() {
        let data = export_fit(&make_summary(), &[]).unwrap();
        let msgs = parse_fit_messages(&data);
        let session = msgs.iter().find(|m| m.global_msg == 18).expect("no session");
        assert_eq!(session.field_u16(45), Some(200), "threshold_power (field 45) = FTP");
        assert!(
            session.field_bytes.iter().all(|(n, _)| *n != 38),
            "field 38 is end_position_lat and must not be written"
        );
    }

    #[test]
    fn fit_device_info_manufacturer_is_field_2() {
        let data = export_fit(&make_summary(), &[]).unwrap();
        let msgs = parse_fit_messages(&data);
        let dev = msgs.iter().find(|m| m.global_msg == 23).expect("no device_info");
        assert_eq!(dev.field_u16(2), Some(255), "manufacturer (field 2) = development");
        assert!(
            dev.field_bytes.iter().all(|(n, _)| *n != 3),
            "field 3 is serial_number (uint32z) and must not carry the manufacturer"
        );
    }

    // --- Independent decoder (fitparser, generated from the FIT SDK profile) ---

    fn decode(data: &[u8]) -> Vec<fitparser::FitDataRecord> {
        fitparser::from_bytes(data).expect("fitparser failed to decode exported FIT file")
    }

    fn named<'a>(rec: &'a fitparser::FitDataRecord, name: &str) -> Option<&'a fitparser::Value> {
        rec.fields().iter().find(|f| f.name() == name).map(|f| f.value())
    }

    fn first_of(
        recs: &[fitparser::FitDataRecord],
        kind: fitparser::profile::MesgNum,
    ) -> &fitparser::FitDataRecord {
        recs.iter().find(|r| r.kind() == kind).expect("message kind missing")
    }

    #[test]
    fn fit_export_decodes_with_fitparser() {
        use fitparser::profile::MesgNum;
        use fitparser::Value;
        let data = export_fit(&make_summary(), &[]).unwrap();
        let recs = decode(&data);
        let session = first_of(&recs, MesgNum::Session);
        assert_eq!(named(session, "threshold_power"), Some(&Value::UInt16(200)));
        assert_eq!(named(session, "end_position_lat"), None);
        assert_eq!(named(session, "normalized_power"), Some(&Value::UInt16(190)));
        assert_eq!(named(session, "training_stress_score"), Some(&Value::Float64(75.0)));
        let dev = first_of(&recs, MesgNum::DeviceInfo);
        assert_eq!(named(dev, "manufacturer"), Some(&Value::String("development".into())));
        assert_eq!(named(dev, "serial_number"), None);
    }

    // --- Record stream, pauses and timer events (A5) ---

    const BASE_MS: u64 = 1718445600_000; // == make_summary().start_time
    const BASE_FIT: u32 = 1087380000; // FIT timestamp of BASE_MS

    fn pw(watts: u16, sec: u64) -> SensorReading {
        SensorReading::Power {
            watts, timestamp: None, epoch_ms: BASE_MS + sec * 1000,
            device_id: "t".into(), pedal_balance: None,
        }
    }
    fn hr_at(bpm: u8, ms_offset: u64) -> SensorReading {
        SensorReading::HeartRate { bpm, timestamp: None, epoch_ms: BASE_MS + ms_offset, device_id: "t".into() }
    }
    fn spd_at(kmh: f32, ms_offset: u64) -> SensorReading {
        SensorReading::Speed { kmh, timestamp: None, epoch_ms: BASE_MS + ms_offset, device_id: "t".into() }
    }

    /// Summary whose active duration matches the readings fed with it.
    fn summary_with_duration(duration_secs: u64) -> SessionSummary {
        let mut s = make_summary();
        s.duration_secs = duration_secs;
        s
    }

    #[test]
    fn fit_end_ts_falls_back_to_active_duration_when_longer_than_span() {
        // Active duration (3600 s) exceeds the data span (records 0..=59 s):
        // end_ts = start + duration, still ≥ the last record; timer ≤ elapsed.
        let readings: Vec<SensorReading> = (0..60).map(|s| pw(200, s)).collect();
        let data = export_fit(&summary_with_duration(3600), &readings).unwrap();
        let msgs = parse_fit_messages(&data);
        let last_record_ts = records(&msgs).last().unwrap().field_u32(253).unwrap();
        assert_eq!(last_record_ts, BASE_FIT + 59);
        for global in [18u16, 19] {
            let m = msgs.iter().find(|m| m.global_msg == global).unwrap();
            let end = m.field_u32(253).unwrap();
            assert_eq!(end, BASE_FIT + 3600, "msg {global} end = start + active duration");
            assert!(end >= last_record_ts);
            let (elapsed, timer) = (m.field_u32(7).unwrap(), m.field_u32(8).unwrap());
            assert_eq!(elapsed, 3_600_000);
            assert!(timer <= elapsed, "msg {global}: timer {timer} > elapsed {elapsed}");
        }
    }

    #[test]
    fn fit_end_ts_is_last_record_when_span_exceeds_duration() {
        // Records span 59 s but only 30 s were active: end = last record,
        // elapsed = 59 s, timer = 30 s.
        let readings: Vec<SensorReading> = (0..60).map(|s| pw(200, s)).collect();
        let data = export_fit(&summary_with_duration(30), &readings).unwrap();
        let msgs = parse_fit_messages(&data);
        let session = msgs.iter().find(|m| m.global_msg == 18).unwrap();
        assert_eq!(session.field_u32(253), Some(BASE_FIT + 59));
        assert_eq!(session.field_u32(7), Some(59_000));
        assert_eq!(session.field_u32(8), Some(30_000));
    }

    fn records(msgs: &[FitMsg]) -> Vec<&FitMsg> {
        msgs.iter().filter(|m| m.global_msg == 20).collect()
    }

    /// 60 s riding, 10-min pause, 60 s riding; active duration 120 s.
    fn paused_ride() -> (SessionSummary, Vec<SensorReading>) {
        let mut summary = make_summary();
        summary.duration_secs = 120;
        let mut readings = Vec::new();
        for s in (0..60).chain(660..720) {
            readings.push(pw(200, s));
            readings.push(hr_at(140, s * 1000 + 200));
        }
        (summary, readings)
    }

    #[test]
    fn fit_hr_speed_only_ride_has_one_record_per_second() {
        let readings: Vec<SensorReading> = (0..60)
            .flat_map(|s| [hr_at(140, s * 1000), spd_at(36.0, s * 1000 + 500)])
            .collect();
        let data = export_fit(&summary_with_duration(60), &readings).unwrap();
        let msgs = parse_fit_messages(&data);
        let recs = records(&msgs);
        assert_eq!(recs.len(), 60, "one record per second of data");
        assert_eq!(recs[0].field_u32(253), Some(BASE_FIT));
        assert_eq!(recs[59].field_u32(253), Some(BASE_FIT + 59));
        assert_eq!(recs[10].field_u8(3), Some(140), "HR present");
        assert_eq!(recs[10].field_u16(7), Some(0xFFFF), "no power sensor → invalid");
        assert_eq!(recs[10].field_u16(6), Some(10_000), "36 km/h = 10 m/s × 1000");
    }

    #[test]
    fn fit_power_at_4hz_yields_one_record_per_second_with_mean_power() {
        let readings: Vec<SensorReading> = (0..10u64)
            .flat_map(|s| {
                [100u16, 200, 300, 400].into_iter().enumerate().map(move |(i, w)| {
                    SensorReading::Power {
                        watts: w, timestamp: None, epoch_ms: BASE_MS + s * 1000 + i as u64 * 250,
                        device_id: "t".into(), pedal_balance: None,
                    }
                })
            })
            .collect();
        let data = export_fit(&summary_with_duration(10), &readings).unwrap();
        let msgs = parse_fit_messages(&data);
        let recs = records(&msgs);
        assert_eq!(recs.len(), 10);
        assert_eq!(recs[3].field_u16(7), Some(250), "(100+200+300+400)/4");
    }

    #[test]
    fn fit_paused_ride_end_ts_not_before_last_record() {
        let (summary, readings) = paused_ride();
        let data = export_fit(&summary, &readings).unwrap();
        let msgs = parse_fit_messages(&data);
        let last_record_ts = records(&msgs).last().unwrap().field_u32(253).unwrap();
        assert_eq!(last_record_ts, BASE_FIT + 719);
        for global in [18u16, 19] {
            let m = msgs.iter().find(|m| m.global_msg == global).unwrap();
            assert!(m.field_u32(253).unwrap() >= last_record_ts, "msg {global} ends before last record");
        }
    }

    #[test]
    fn fit_paused_ride_elapsed_is_wall_clock_timer_is_active() {
        let (summary, readings) = paused_ride();
        let data = export_fit(&summary, &readings).unwrap();
        let msgs = parse_fit_messages(&data);
        for global in [18u16, 19] {
            let m = msgs.iter().find(|m| m.global_msg == global).unwrap();
            assert_eq!(m.field_u32(7), Some(719_000), "msg {global} total_elapsed_time = wall clock");
            assert_eq!(m.field_u32(8), Some(120_000), "msg {global} total_timer_time = active");
        }
        let act = msgs.iter().find(|m| m.global_msg == 34).unwrap();
        assert_eq!(act.field_u32(0), Some(120_000), "activity total_timer_time = active");
    }

    #[test]
    fn fit_writes_timer_stop_start_around_gap() {
        let (summary, readings) = paused_ride();
        let data = export_fit(&summary, &readings).unwrap();
        let msgs = parse_fit_messages(&data);
        let events: Vec<(u32, u8, u8)> = msgs
            .iter()
            .filter(|m| m.global_msg == 21)
            .map(|m| (m.field_u32(253).unwrap(), m.field_u8(0).unwrap(), m.field_u8(1).unwrap()))
            .collect();
        assert_eq!(
            events,
            vec![
                (BASE_FIT, 0, 0),       // timer start
                (BASE_FIT + 59, 0, 4),  // stop_all at last record before the gap
                (BASE_FIT + 660, 0, 0), // start at first record after the gap
                (BASE_FIT + 719, 0, 4), // final stop_all at end_ts
            ]
        );
        // Event order in the file follows time: the pause pair sits between records
        let order: Vec<u16> = msgs.iter().map(|m| m.global_msg).filter(|g| *g == 20 || *g == 21).collect();
        let stop_idx = order.iter().enumerate().filter(|(_, g)| **g == 21).nth(1).unwrap().0;
        assert_eq!(order[..stop_idx].iter().filter(|g| **g == 20).count(), 60);
    }

    #[test]
    fn fit_short_pause_writes_no_timer_events() {
        // 4 s hole (≤ 5 s) is indistinguishable from a sensor hole → no timer events
        let readings: Vec<SensorReading> = (0..10).chain(14..20).map(|s| pw(200, s)).collect();
        let data = export_fit(&summary_with_duration(20), &readings).unwrap();
        let msgs = parse_fit_messages(&data);
        assert_eq!(msgs.iter().filter(|m| m.global_msg == 21).count(), 2);
    }

    #[test]
    fn fit_record_distance_follows_gap_rule() {
        // 36 km/h (10 m/s) for 0..=10 s, 10-min gap, 36 km/h for 610..=620 s
        let readings: Vec<SensorReading> =
            (0..=10).chain(610..=620).map(|s| spd_at(36.0, s * 1000)).collect();
        let data = export_fit(&summary_with_duration(25), &readings).unwrap();
        let msgs = parse_fit_messages(&data);
        let last = *records(&msgs).last().unwrap();
        // (10 s + 5 s capped gap + 10 s) × 10 m/s = 250 m → ×100 = 25000
        assert_eq!(last.field_u32(5), Some(25_000));
    }

    #[test]
    fn fit_hr_invalid_after_5s_without_reading() {
        // Power every second for 20 s; HR strap drops after its reading at 4 s
        let mut readings: Vec<SensorReading> = (0..20).map(|s| pw(200, s)).collect();
        readings.extend((0..=4).map(|s| hr_at(150, s * 1000)));
        let data = export_fit(&summary_with_duration(20), &readings).unwrap();
        let msgs = parse_fit_messages(&data);
        let recs = records(&msgs);
        assert_eq!(recs[9].field_u8(3), Some(150), "5 s after last HR: still valid");
        assert_eq!(recs[10].field_u8(3), Some(0xFF), "> 5 s after last HR: invalid");
    }

    #[test]
    fn fit_paused_ride_decodes_with_fitparser() {
        use fitparser::profile::MesgNum;
        use fitparser::Value;
        let (summary, readings) = paused_ride();
        let data = export_fit(&summary, &readings).unwrap();
        let recs = decode(&data);
        assert_eq!(recs.iter().filter(|r| r.kind() == MesgNum::Record).count(), 120);
        let session = first_of(&recs, MesgNum::Session);
        assert_eq!(named(session, "total_elapsed_time"), Some(&Value::Float64(719.0)));
        assert_eq!(named(session, "total_timer_time"), Some(&Value::Float64(120.0)));
        let event_types: Vec<String> = recs
            .iter()
            .filter(|r| r.kind() == MesgNum::Event)
            .map(|r| named(r, "event_type").unwrap().to_string())
            .collect();
        assert_eq!(event_types, vec!["start", "stop_all", "start", "stop_all"]);
    }

    /// Writes FIT fixtures for the recorded independent check with Python
    /// `fitdecode` (see design.md Verification). Run with
    /// `FIT_FIXTURE_DIR=/some/dir cargo test --lib write_fit_fixtures -- --ignored`.
    #[test]
    #[ignore]
    fn write_fit_fixtures() {
        let dir = std::env::var("FIT_FIXTURE_DIR").expect("set FIT_FIXTURE_DIR");
        let (paused, paused_readings) = paused_ride();
        let hr_speed: Vec<SensorReading> = (0..60)
            .flat_map(|s| [hr_at(140, s * 1000), spd_at(36.0, s * 1000 + 500)])
            .collect();
        let power_hr: Vec<SensorReading> =
            (0..60).flat_map(|s| [pw(200, s), hr_at(140, s * 1000 + 300)]).collect();
        for (name, summary, readings) in [
            ("paused.fit", &paused, &paused_readings),
            ("hr_speed_only.fit", &summary_with_duration(60), &hr_speed),
            ("power_hr.fit", &summary_with_duration(60), &power_hr),
        ] {
            let data = export_fit(summary, readings).unwrap();
            std::fs::write(std::path::Path::new(&dir).join(name), data).unwrap();
        }
    }
}
