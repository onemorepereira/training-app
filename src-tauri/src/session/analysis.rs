use serde::{Deserialize, Serialize};

use crate::device::types::SensorReading;
use crate::session::types::{SessionConfig, SessionSummary};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionAnalysis {
    pub timeseries: Vec<TimeseriesPoint>,
    pub power_curve: Vec<PowerCurvePoint>,
    pub power_zone_distribution: Vec<ZoneBucket>,
    pub hr_zone_distribution: Vec<ZoneBucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeseriesPoint {
    pub elapsed_secs: f64,
    pub power: Option<u16>,
    pub heart_rate: Option<u8>,
    pub cadence: Option<f32>,
    pub speed: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerCurvePoint {
    pub duration_secs: u32,
    pub watts: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneBucket {
    pub zone: u8,
    pub duration_secs: f64,
    pub percentage: f64,
}

const MAX_READING_GAP_MS: u64 = 5000;

const POWER_CURVE_DURATIONS: &[u32] = &[
    1, 2, 3, 5, 10, 15, 20, 30, 45, 60, 120, 300, 600, 1200, 1800, 3600,
];

pub fn compute_analysis(
    readings: &[SensorReading],
    session: &SessionSummary,
    config: &SessionConfig,
) -> SessionAnalysis {
    let timeseries = build_timeseries(readings, session.duration_secs);
    let power_curve = compute_power_curve(readings);
    let ftp = session.ftp.unwrap_or(config.ftp);
    let (power_zone_distribution, hr_zone_distribution) =
        compute_zone_distribution(readings, ftp, &config.power_zones, &config.hr_zones);
    SessionAnalysis {
        timeseries,
        power_curve,
        power_zone_distribution,
        hr_zone_distribution,
    }
}

fn build_timeseries(readings: &[SensorReading], duration_secs: u64) -> Vec<TimeseriesPoint> {
    if readings.is_empty() {
        return Vec::new();
    }

    let t0 = readings.iter().map(|r| r.epoch_ms()).min().unwrap();
    let num_slots = duration_secs as usize;

    // Each slot holds the last-seen value for each channel.
    struct Slot {
        power: Option<u16>,
        heart_rate: Option<u8>,
        cadence: Option<f32>,
        speed: Option<f32>,
    }

    let mut slots: Vec<Slot> = (0..num_slots)
        .map(|_| Slot {
            power: None,
            heart_rate: None,
            cadence: None,
            speed: None,
        })
        .collect();

    for reading in readings {
        let elapsed_ms = reading.epoch_ms().saturating_sub(t0);
        let sec = (elapsed_ms / 1000) as usize;
        if sec >= num_slots {
            continue;
        }
        let slot = &mut slots[sec];
        match reading {
            SensorReading::Power { watts, .. } => slot.power = Some(*watts),
            SensorReading::HeartRate { bpm, .. } => slot.heart_rate = Some(*bpm),
            SensorReading::Cadence { rpm, .. } => slot.cadence = Some(*rpm),
            SensorReading::Speed { kmh, .. } => slot.speed = Some(*kmh),
        }
    }

    slots
        .into_iter()
        .enumerate()
        .filter_map(|(i, s)| {
            if s.power.is_none() && s.heart_rate.is_none() && s.cadence.is_none() && s.speed.is_none()
            {
                None
            } else {
                Some(TimeseriesPoint {
                    elapsed_secs: i as f64,
                    power: s.power,
                    heart_rate: s.heart_rate,
                    cadence: s.cadence,
                    speed: s.speed,
                })
            }
        })
        .collect()
}

fn compute_power_curve(readings: &[SensorReading]) -> Vec<PowerCurvePoint> {
    // Extract power readings sorted by time.
    let mut power_data: Vec<(u64, u16)> = readings
        .iter()
        .filter_map(|r| match r {
            SensorReading::Power { watts, epoch_ms, .. } => Some((*epoch_ms, *watts)),
            _ => None,
        })
        .collect();

    if power_data.is_empty() {
        return Vec::new();
    }

    power_data.sort_by_key(|(ms, _)| *ms);

    // Resample to 1-second array with hold-last-value.
    let min_sec = power_data[0].0 / 1000;
    let max_sec = power_data.last().unwrap().0 / 1000;
    let len = (max_sec - min_sec + 1) as usize;

    // Accumulate sum and count per second for averaging.
    let mut sums = vec![0u64; len];
    let mut counts = vec![0u32; len];

    for &(ms, watts) in &power_data {
        let idx = (ms / 1000 - min_sec) as usize;
        sums[idx] += watts as u64;
        counts[idx] += 1;
    }

    // Build the 1-second array: average where data exists, hold-last-value otherwise.
    // Skip leading empty seconds by finding the first populated index.
    let first_populated = counts.iter().position(|&c| c > 0).unwrap();
    let arr_offset = first_populated;
    let arr_len = len - arr_offset;
    let mut arr = vec![0u32; arr_len];

    let mut last_val = 0u32;
    for i in 0..arr_len {
        let src = i + arr_offset;
        if counts[src] > 0 {
            last_val = (sums[src] / counts[src] as u64) as u32;
        }
        arr[i] = last_val;
    }

    // Sliding window for each target duration.
    let mut result = Vec::new();
    for &d in POWER_CURVE_DURATIONS {
        let d_usize = d as usize;
        if d_usize > arr.len() {
            continue;
        }

        let mut window_sum: u64 = arr[..d_usize].iter().map(|&v| v as u64).sum();
        let mut max_sum = window_sum;

        for i in 1..=(arr.len() - d_usize) {
            window_sum = window_sum - arr[i - 1] as u64 + arr[i + d_usize - 1] as u64;
            if window_sum > max_sum {
                max_sum = window_sum;
            }
        }

        result.push(PowerCurvePoint {
            duration_secs: d,
            watts: (max_sum as f64 / d as f64).round() as u16,
        });
    }

    result
}

fn classify_power_zone(watts: u16, ftp: u16, zones: &[u16; 6]) -> u8 {
    let pct = (watts as f32 / ftp.max(1) as f32) * 100.0;
    for (i, &upper) in zones.iter().enumerate() {
        if pct <= upper as f32 {
            return (i + 1) as u8;
        }
    }
    7
}

fn classify_hr_zone(bpm: u8, zones: &[u8; 5]) -> u8 {
    for (i, &upper) in zones.iter().enumerate() {
        if bpm <= upper {
            return (i + 1) as u8;
        }
    }
    5
}

fn compute_zone_distribution(
    readings: &[SensorReading],
    ftp: u16,
    power_zones: &[u16; 6],
    hr_zones: &[u8; 5],
) -> (Vec<ZoneBucket>, Vec<ZoneBucket>) {
    // Power zones (7 zones)
    let mut power_data: Vec<(u64, u16)> = readings
        .iter()
        .filter_map(|r| match r {
            SensorReading::Power { watts, epoch_ms, .. } => Some((*epoch_ms, *watts)),
            _ => None,
        })
        .collect();
    power_data.sort_by_key(|(ms, _)| *ms);

    let mut power_zone_time = [0.0f64; 7];
    for pair in power_data.windows(2) {
        let delta_ms = pair[1].0.saturating_sub(pair[0].0).min(MAX_READING_GAP_MS);
        let zone = classify_power_zone(pair[0].1, ftp, power_zones);
        power_zone_time[(zone - 1) as usize] += delta_ms as f64 / 1000.0;
    }

    let power_total: f64 = power_zone_time.iter().sum();
    let power_zone_dist: Vec<ZoneBucket> = power_zone_time
        .iter()
        .enumerate()
        .map(|(i, &secs)| ZoneBucket {
            zone: (i + 1) as u8,
            duration_secs: secs,
            percentage: if power_total > 0.0 {
                secs / power_total * 100.0
            } else {
                0.0
            },
        })
        .collect();

    // HR zones (5 zones)
    let mut hr_data: Vec<(u64, u8)> = readings
        .iter()
        .filter_map(|r| match r {
            SensorReading::HeartRate { bpm, epoch_ms, .. } => Some((*epoch_ms, *bpm)),
            _ => None,
        })
        .collect();
    hr_data.sort_by_key(|(ms, _)| *ms);

    let mut hr_zone_time = [0.0f64; 5];
    for pair in hr_data.windows(2) {
        let delta_ms = pair[1].0.saturating_sub(pair[0].0).min(MAX_READING_GAP_MS);
        let zone = classify_hr_zone(pair[0].1, hr_zones);
        hr_zone_time[(zone - 1) as usize] += delta_ms as f64 / 1000.0;
    }

    let hr_total: f64 = hr_zone_time.iter().sum();
    let hr_zone_dist: Vec<ZoneBucket> = hr_zone_time
        .iter()
        .enumerate()
        .map(|(i, &secs)| ZoneBucket {
            zone: (i + 1) as u8,
            duration_secs: secs,
            percentage: if hr_total > 0.0 {
                secs / hr_total * 100.0
            } else {
                0.0
            },
        })
        .collect();

    (power_zone_dist, hr_zone_dist)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn power_reading(watts: u16, epoch_ms: u64) -> SensorReading {
        SensorReading::Power {
            watts,
            timestamp: None,
            epoch_ms,
            device_id: String::new(),
            pedal_balance: None,
        }
    }

    fn hr_reading(bpm: u8, epoch_ms: u64) -> SensorReading {
        SensorReading::HeartRate {
            bpm,
            timestamp: None,
            epoch_ms,
            device_id: String::new(),
        }
    }

    fn cadence_reading(rpm: f32, epoch_ms: u64) -> SensorReading {
        SensorReading::Cadence {
            rpm,
            timestamp: None,
            epoch_ms,
            device_id: String::new(),
        }
    }

    fn speed_reading(kmh: f32, epoch_ms: u64) -> SensorReading {
        SensorReading::Speed {
            kmh,
            timestamp: None,
            epoch_ms,
            device_id: String::new(),
        }
    }

    fn test_config() -> SessionConfig {
        SessionConfig::default()
    }

    fn test_session(duration_secs: u64, ftp: u16) -> SessionSummary {
        SessionSummary {
            id: "test".into(),
            start_time: Utc::now(),
            duration_secs,
            ftp: Some(ftp),
            avg_power: None,
            max_power: None,
            normalized_power: None,
            tss: None,
            intensity_factor: None,
            avg_hr: None,
            max_hr: None,
            avg_cadence: None,
            avg_speed: None,
            title: None,
            activity_type: None,
            rpe: None,
            notes: None,
        }
    }

    fn assert_approx(actual: f64, expected: f64, epsilon: f64, msg: &str) {
        assert!(
            (actual - expected).abs() <= epsilon,
            "{msg}: expected {expected} ± {epsilon}, got {actual}"
        );
    }

    // --- Power curve tests ---

    #[test]
    fn power_curve_constant_power() {
        // 60 readings at 200W, 1Hz starting at t=0
        let readings: Vec<SensorReading> =
            (0..60).map(|i| power_reading(200, i * 1000)).collect();

        let curve = compute_power_curve(&readings);

        // All durations ≤ 60s should be 200W
        for pt in &curve {
            assert!(pt.duration_secs <= 60, "should not exceed session length");
            assert_eq!(pt.watts, 200, "constant 200W for {}s duration", pt.duration_secs);
        }
        // Should have entries for 1,2,3,5,10,15,20,30,45,60
        assert_eq!(curve.len(), 10);
    }

    #[test]
    fn power_curve_best_interval() {
        // 10s @ 100W, then 10s @ 300W (t=0..9s @ 100W, t=10..19s @ 300W)
        let mut readings: Vec<SensorReading> = Vec::new();
        for i in 0..10 {
            readings.push(power_reading(100, i * 1000));
        }
        for i in 10..20 {
            readings.push(power_reading(300, i * 1000));
        }

        let curve = compute_power_curve(&readings);

        // 1s best = 300W (best single second in the 300W block)
        let p1 = curve.iter().find(|p| p.duration_secs == 1).unwrap();
        assert_eq!(p1.watts, 300);

        // 10s best = 300W (the 10s block of 300W)
        let p10 = curve.iter().find(|p| p.duration_secs == 10).unwrap();
        assert_eq!(p10.watts, 300);

        // 20s best = average of 100*10 + 300*10 = 4000/20 = 200W
        let p20 = curve.iter().find(|p| p.duration_secs == 20).unwrap();
        assert_eq!(p20.watts, 200);
    }

    #[test]
    fn power_curve_capped_at_session_length() {
        // 30 readings → no entry with duration > 30
        let readings: Vec<SensorReading> =
            (0..30).map(|i| power_reading(150, i * 1000)).collect();

        let curve = compute_power_curve(&readings);
        for pt in &curve {
            assert!(pt.duration_secs <= 30);
        }
    }

    #[test]
    fn power_curve_empty_readings() {
        let curve = compute_power_curve(&[]);
        assert!(curve.is_empty());

        // Also: only HR readings, no power
        let readings = vec![hr_reading(140, 1000)];
        let curve = compute_power_curve(&readings);
        assert!(curve.is_empty());
    }

    // --- Zone distribution tests ---

    #[test]
    fn zone_single_zone() {
        // 10 readings at 100W with FTP=200 → 50% FTP → zone 1 (≤55%)
        // Default power_zones: [55, 75, 90, 105, 120, 150]
        let readings: Vec<SensorReading> =
            (0..10).map(|i| power_reading(100, i * 1000)).collect();
        let config = test_config();

        let (power_zones, _) =
            compute_zone_distribution(&readings, 200, &config.power_zones, &config.hr_zones);

        // 9 seconds of zone time total (9 gaps between 10 readings)
        let total: f64 = power_zones.iter().map(|z| z.duration_secs).sum();
        assert_approx(total, 9.0, 0.01, "total zone time");

        // 100% in zone 1
        assert_approx(power_zones[0].percentage, 100.0, 0.01, "zone 1 percentage");
        assert_approx(power_zones[0].duration_secs, 9.0, 0.01, "zone 1 duration");
    }

    #[test]
    fn zone_split_time() {
        // 5s @ 100W (50% FTP=200 → Z1) then 5s @ 250W (125% FTP → Z6, >120% and ≤150%)
        // Default power_zones: [55, 75, 90, 105, 120, 150]
        let mut readings = Vec::new();
        for i in 0..5 {
            readings.push(power_reading(100, i * 1000));
        }
        for i in 5..10 {
            readings.push(power_reading(250, i * 1000));
        }
        let config = test_config();

        let (power_zones, _) =
            compute_zone_distribution(&readings, 200, &config.power_zones, &config.hr_zones);

        // Gaps: 0→1, 1→2, 2→3, 3→4 at 100W (Z1) = 4s
        //        4→5 at 100W (Z1) = 1s  (reading at t=4 is 100W, gap to t=5)
        //        wait — reading at index 4 is power_reading(100, 4000), index 5 is power_reading(250, 5000)
        //        So the pair (4000,100) → (5000,250): prev.watts=100 → Z1, delta=1s
        //        Total Z1 = 5s (pairs 0-1, 1-2, 2-3, 3-4, 4-5)
        //        Pairs 5-6, 6-7, 7-8, 8-9: prev.watts=250 → Z6, 4s total
        // Total = 9s, Z1 = 5s (55.6%), Z6 = 4s (44.4%)
        let total: f64 = power_zones.iter().map(|z| z.duration_secs).sum();
        assert_approx(total, 9.0, 0.01, "total zone time");
        assert_approx(power_zones[0].duration_secs, 5.0, 0.01, "zone 1 duration");
        assert_approx(power_zones[5].duration_secs, 4.0, 0.01, "zone 6 duration");
    }

    #[test]
    fn zone_gap_capped_at_5s() {
        // Two power readings 10s apart → only 5s counted (MAX_READING_GAP_MS cap)
        let readings = vec![power_reading(100, 0), power_reading(100, 10_000)];
        let config = test_config();

        let (power_zones, _) =
            compute_zone_distribution(&readings, 200, &config.power_zones, &config.hr_zones);

        let total: f64 = power_zones.iter().map(|z| z.duration_secs).sum();
        assert_approx(total, 5.0, 0.01, "gap capped at 5s");
    }

    #[test]
    fn hr_zone_distribution() {
        // Default hr_zones: [120, 140, 160, 175, 190]
        // 5 readings at 100bpm (Z1: ≤120), 1Hz
        // 5 readings at 150bpm (Z3: >140, ≤160), 1Hz
        let mut readings = Vec::new();
        for i in 0..5 {
            readings.push(hr_reading(100, i * 1000));
        }
        for i in 5..10 {
            readings.push(hr_reading(150, i * 1000));
        }
        let config = test_config();

        let (_, hr_zones) =
            compute_zone_distribution(&readings, 200, &config.power_zones, &config.hr_zones);

        let total: f64 = hr_zones.iter().map(|z| z.duration_secs).sum();
        assert_approx(total, 9.0, 0.01, "total HR zone time");

        // Z1 (≤120): pairs at 100bpm → 5s (pairs 0-1,1-2,2-3,3-4,4-5 where prev=100)
        assert_approx(hr_zones[0].duration_secs, 5.0, 0.01, "HR zone 1 duration");
        // Z3 (141-160): pairs at 150bpm → 4s (pairs 5-6,6-7,7-8,8-9)
        assert_approx(hr_zones[2].duration_secs, 4.0, 0.01, "HR zone 3 duration");
    }

    // --- Timeseries tests ---

    #[test]
    fn timeseries_downsamples_to_seconds() {
        // 4Hz power readings over 3 seconds (12 readings, 250ms apart)
        let mut readings = Vec::new();
        for sec in 0..3 {
            for sub in 0..4 {
                let ms = sec * 1000 + sub * 250;
                // Power increases within each second; last value kept
                readings.push(power_reading(200 + (sub as u16), ms));
            }
        }

        let ts = build_timeseries(&readings, 3);

        assert_eq!(ts.len(), 3, "should have 3 second-slots");
        // Last value in each second is the one at sub=3, so watts = 203
        for pt in &ts {
            assert_eq!(pt.power, Some(203));
        }
    }

    #[test]
    fn timeseries_merges_channels() {
        // Power and HR readings at same second
        let readings = vec![
            power_reading(250, 1000),
            hr_reading(145, 1500),
        ];

        let ts = build_timeseries(&readings, 5);

        assert_eq!(ts.len(), 1, "one slot has data");
        let pt = &ts[0];
        // Both readings at 1000ms and 1500ms map to slot 0 (t0=1000, elapsed=0s)
        assert_eq!(pt.elapsed_secs, 0.0);
        assert_eq!(pt.power, Some(250));
        assert_eq!(pt.heart_rate, Some(145));
        assert_eq!(pt.cadence, None);
        assert_eq!(pt.speed, None);
    }

    #[test]
    fn timeseries_empty() {
        let ts = build_timeseries(&[], 60);
        assert!(ts.is_empty());
    }

    // --- compute_analysis FTP fallback ---

    #[test]
    fn compute_analysis_uses_session_ftp_when_present() {
        // Session FTP=250, config FTP=200 (default) → zones should use 250
        let readings = vec![
            power_reading(250, 1000),
            power_reading(250, 2000),
        ];
        let session = test_session(2, 250);
        let config = test_config();

        let analysis = compute_analysis(&readings, &session, &config);

        // 250W at FTP=250 → 100% FTP → zone 4 (threshold: 90-105%)
        // Power zones [55, 75, 90, 105, 120, 150] → Z4 is 90-105% FTP
        let z4 = analysis.power_zone_distribution.iter().find(|z| z.zone == 4);
        assert!(z4.is_some(), "should have zone 4 bucket");
        assert!(z4.unwrap().percentage > 0.0, "zone 4 should have time");
    }

    #[test]
    fn compute_analysis_falls_back_to_config_ftp_when_session_ftp_is_none() {
        let readings = vec![
            power_reading(200, 1000),
            power_reading(200, 2000),
        ];
        // Session has ftp=None
        let mut session = test_session(2, 200);
        session.ftp = None;
        let config = test_config(); // default FTP=200

        let analysis = compute_analysis(&readings, &session, &config);

        // 200W at FTP=200 → 100% FTP → zone 4
        let z4 = analysis.power_zone_distribution.iter().find(|z| z.zone == 4);
        assert!(z4.is_some(), "should have zone 4 bucket using config FTP");
        assert!(z4.unwrap().percentage > 0.0, "zone 4 should have time with config FTP fallback");
    }

    #[test]
    fn compute_analysis_session_ftp_overrides_config_ftp() {
        // Session FTP=100, config FTP=200 → 200W = 200% of session FTP → zone 7
        let readings = vec![
            power_reading(200, 1000),
            power_reading(200, 2000),
        ];
        let session = test_session(2, 100);
        let config = test_config(); // FTP=200

        let analysis = compute_analysis(&readings, &session, &config);

        // 200W at FTP=100 → 200% FTP → zone 7 (>150%)
        let z7 = analysis.power_zone_distribution.iter().find(|z| z.zone == 7);
        assert!(z7.is_some(), "should have zone 7 bucket");
        assert!(z7.unwrap().percentage > 0.0, "200W at FTP=100 should be zone 7");
    }
}
