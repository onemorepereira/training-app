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
    w.write_definition(1, 23, &[
        (253, 4, 134), // timestamp: uint32
        (0, 1, 2),     // device_index: uint8
        (3, 2, 132),   // manufacturer: uint16
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

    // Start event (timer start)
    let mut start_evt = Vec::new();
    start_evt.extend_from_slice(&start_ts.to_le_bytes());
    start_evt.push(0); // event = timer
    start_evt.push(0); // event_type = start
    w.write_data(2, &start_evt);

    // --- record messages (local 3, global 20) ---
    w.write_definition(3, 20, &[
        (253, 4, 134), // timestamp: uint32
        (7, 2, 132),   // power: uint16
        (3, 1, 2),     // heart_rate: uint8
        (4, 1, 2),     // cadence: uint8
        (6, 2, 132),   // speed: uint16 (m/s * 1000)
        (5, 4, 134),   // distance: uint32 (m * 100)
    ]);

    let mut last_hr: u8 = 0xFF;
    let mut last_cadence: u8 = 0xFF;
    let mut last_speed: u16 = 0xFFFF;
    let mut cumulative_distance_m100: u32 = 0;
    let mut last_speed_ms: f64 = 0.0;
    let mut last_speed_epoch_ms: Option<u64> = None;
    let mut max_speed_ms1000: u16 = 0;

    for reading in readings {
        match reading {
            SensorReading::HeartRate { bpm, .. } => {
                last_hr = *bpm;
            }
            SensorReading::Cadence { rpm, .. } => {
                last_cadence = (*rpm).min(254.0) as u8;
            }
            SensorReading::Speed { kmh, epoch_ms, .. } => {
                if let Some(prev) = last_speed_epoch_ms {
                    let dt_s = (*epoch_ms as f64 - prev as f64) / 1000.0;
                    cumulative_distance_m100 += (last_speed_ms * dt_s * 100.0) as u32;
                }
                last_speed_ms = *kmh as f64 / 3.6;
                last_speed_epoch_ms = Some(*epoch_ms);
                let ms_1000 = (*kmh / 3.6 * 1000.0) as u16;
                last_speed = ms_1000;
                max_speed_ms1000 = max_speed_ms1000.max(ms_1000);
            }
            SensorReading::Power {
                watts, epoch_ms, ..
            } => {
                let ts = unix_to_fit_timestamp(*epoch_ms);
                let mut rec = Vec::with_capacity(14);
                rec.extend_from_slice(&ts.to_le_bytes());
                rec.extend_from_slice(&watts.to_le_bytes());
                rec.push(last_hr);
                rec.push(last_cadence);
                rec.extend_from_slice(&last_speed.to_le_bytes());
                rec.extend_from_slice(&cumulative_distance_m100.to_le_bytes());
                w.write_data(3, &rec);
            }
            SensorReading::TrainerCommand { .. } => {}
        }
    }

    let end_ts = start_ts + summary.duration_secs as u32;
    let elapsed_ms = (summary.duration_secs * 1000) as u32;

    // Stop event (timer stop_all)
    let mut stop_evt = Vec::new();
    stop_evt.extend_from_slice(&end_ts.to_le_bytes());
    stop_evt.push(0); // event = timer
    stop_evt.push(4); // event_type = stop_all
    w.write_data(2, &stop_evt);

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
    lap_data.extend_from_slice(&elapsed_ms.to_le_bytes());
    lap_data.push(2); // sport = cycling
    lap_data.push(6); // sub_sport = indoor_cycling
    w.write_data(4, &lap_data);

    // --- session message (local 5, global 18) ---
    let total_distance = summary
        .distance_km
        .map(|km| (km * 100_000.0) as u32)
        .unwrap_or(cumulative_distance_m100);
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
        (38, 2, 132),  // threshold_power: uint16
    ]);
    let mut sess_data = Vec::new();
    sess_data.extend_from_slice(&end_ts.to_le_bytes());
    sess_data.extend_from_slice(&start_ts.to_le_bytes());
    sess_data.extend_from_slice(&elapsed_ms.to_le_bytes());
    sess_data.extend_from_slice(&elapsed_ms.to_le_bytes());
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
    act_data.extend_from_slice(&elapsed_ms.to_le_bytes());
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
        let summary = make_summary();
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
        assert_eq!(dev.field_u16(3), Some(255), "manufacturer should be 255 (development)");
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
        let summary = make_summary();
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
        assert_eq!(session.field_u16(38), Some(200), "threshold_power (ftp)");
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
}
