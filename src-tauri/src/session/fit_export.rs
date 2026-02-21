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

    // --- file_id message (global 0) ---
    // Fields: type(0, enum/u8), manufacturer(1, u16), product(2, u16), serial_number(3, u32z), time_created(4, u32)
    w.write_definition(0, 0, &[
        (0, 1, 0),   // type: enum
        (1, 2, 132), // manufacturer: uint16
        (2, 2, 132), // product: uint16
        (3, 4, 140), // serial_number: uint32z
        (4, 4, 134), // time_created: uint32
    ]);
    let mut file_id_data = Vec::new();
    file_id_data.push(4); // type = activity
    file_id_data.extend_from_slice(&1u16.to_le_bytes()); // manufacturer = Garmin (for compat)
    file_id_data.extend_from_slice(&1u16.to_le_bytes()); // product
    file_id_data.extend_from_slice(&0u32.to_le_bytes()); // serial
    file_id_data.extend_from_slice(&start_ts.to_le_bytes()); // time_created
    w.write_data(0, &file_id_data);

    // --- record messages (global 20) ---
    // Fields: timestamp(253, u32), power(7, u16), heart_rate(3, u8), cadence(4, u8), speed(6, u16)
    w.write_definition(1, 20, &[
        (253, 4, 134), // timestamp: uint32
        (7, 2, 132),   // power: uint16
        (3, 1, 2),     // heart_rate: uint8
        (4, 1, 2),     // cadence: uint8
        (6, 2, 132),   // speed: uint16 (m/s * 1000)
    ]);

    let mut last_hr: u8 = 0xFF; // invalid
    let mut last_cadence: u8 = 0xFF;
    let mut last_speed: u16 = 0xFFFF; // invalid

    for reading in readings {
        match reading {
            SensorReading::HeartRate { bpm, .. } => {
                last_hr = *bpm;
            }
            SensorReading::Cadence { rpm, .. } => {
                last_cadence = (*rpm).min(254.0) as u8;
            }
            SensorReading::Speed { kmh, .. } => {
                // Convert km/h to m/s * 1000
                let ms_1000 = (kmh / 3.6 * 1000.0) as u16;
                last_speed = ms_1000;
            }
            SensorReading::Power {
                watts, epoch_ms, ..
            } => {
                let ts = unix_to_fit_timestamp(*epoch_ms);
                let mut rec = Vec::with_capacity(10);
                rec.extend_from_slice(&ts.to_le_bytes());
                rec.extend_from_slice(&watts.to_le_bytes());
                rec.push(last_hr);
                rec.push(last_cadence);
                rec.extend_from_slice(&last_speed.to_le_bytes());
                w.write_data(1, &rec);
            }
            SensorReading::TrainerCommand { .. } => {}
        }
    }

    let end_ts = start_ts + summary.duration_secs as u32;

    // --- lap message (global 19) ---
    // Fields: timestamp(253, u32), start_time(2, u32), total_elapsed_time(7, u32), total_timer_time(8, u32)
    w.write_definition(2, 19, &[
        (253, 4, 134), // timestamp
        (2, 4, 134),   // start_time
        (7, 4, 134),   // total_elapsed_time (s * 1000)
        (8, 4, 134),   // total_timer_time (s * 1000)
    ]);
    let elapsed_ms = (summary.duration_secs * 1000) as u32;
    let mut lap_data = Vec::new();
    lap_data.extend_from_slice(&end_ts.to_le_bytes());
    lap_data.extend_from_slice(&start_ts.to_le_bytes());
    lap_data.extend_from_slice(&elapsed_ms.to_le_bytes());
    lap_data.extend_from_slice(&elapsed_ms.to_le_bytes());
    w.write_data(2, &lap_data);

    // --- session message (global 18) ---
    // Fields: timestamp(253, u32), start_time(2, u32), total_elapsed_time(7, u32), total_timer_time(8, u32),
    //         avg_power(20, u16), max_power(21, u16), normalized_power(34, u16), avg_heart_rate(16, u8)
    w.write_definition(3, 18, &[
        (253, 4, 134), // timestamp
        (2, 4, 134),   // start_time
        (7, 4, 134),   // total_elapsed_time
        (8, 4, 134),   // total_timer_time
        (20, 2, 132),  // avg_power
        (21, 2, 132),  // max_power
        (34, 2, 132),  // normalized_power
        (16, 1, 2),    // avg_heart_rate
    ]);
    let mut sess_data = Vec::new();
    sess_data.extend_from_slice(&end_ts.to_le_bytes());
    sess_data.extend_from_slice(&start_ts.to_le_bytes());
    sess_data.extend_from_slice(&elapsed_ms.to_le_bytes());
    sess_data.extend_from_slice(&elapsed_ms.to_le_bytes());
    sess_data.extend_from_slice(&summary.avg_power.unwrap_or(0xFFFF).to_le_bytes());
    sess_data.extend_from_slice(&summary.max_power.unwrap_or(0xFFFF).to_le_bytes());
    sess_data.extend_from_slice(&summary.normalized_power.unwrap_or(0xFFFF).to_le_bytes());
    sess_data.push(summary.avg_hr.unwrap_or(0xFF));
    w.write_data(3, &sess_data);

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
}
