use std::collections::VecDeque;

pub struct MetricsCalculator {
    ftp: u16,
    /// Timestamped power readings for time-based rolling averages
    power_history: Vec<(u64, u16)>,
    /// 30-second rolling average buffer for NP (one entry per second)
    np_buffer: VecDeque<f64>,
    fourth_power_sum: f64,
    fourth_power_count: u64,
    /// Tracks which epoch-second we last accumulated into NP buffer
    last_np_second: Option<u64>,
    /// Power samples accumulated within the current epoch-second (for averaging)
    current_second_power: Vec<u16>,
    last_epoch_ms: Option<u64>,
    hr_readings: Vec<u8>,
    cadence_readings: Vec<f32>,
    speed_readings: Vec<f32>,
}

impl MetricsCalculator {
    pub fn new(ftp: u16) -> Self {
        Self {
            ftp: ftp.max(1),
            power_history: Vec::new(),
            np_buffer: VecDeque::with_capacity(31),
            fourth_power_sum: 0.0,
            fourth_power_count: 0,
            last_np_second: None,
            current_second_power: Vec::new(),
            last_epoch_ms: None,
            hr_readings: Vec::new(),
            cadence_readings: Vec::new(),
            speed_readings: Vec::new(),
        }
    }

    pub fn record_power(&mut self, watts: u16, epoch_ms: u64) {
        self.last_epoch_ms = Some(epoch_ms);
        self.power_history.push((epoch_ms, watts));

        // NP: accumulate one sample per epoch-second.
        // Within a second, average all readings to get that second's power.
        let current_second = epoch_ms / 1000;
        match self.last_np_second {
            Some(prev_second) if prev_second == current_second => {
                // Same second — accumulate for averaging
                self.current_second_power.push(watts);
            }
            _ => {
                // New second — flush the previous second's average into NP buffer
                if !self.current_second_power.is_empty() {
                    // Gap detection: if >2 seconds passed, reset NP buffer
                    if let Some(prev) = self.last_np_second {
                        if current_second.saturating_sub(prev) > 2 {
                            self.np_buffer.clear();
                            self.fourth_power_sum = 0.0;
                            self.fourth_power_count = 0;
                            self.current_second_power.clear();
                            self.current_second_power.push(watts);
                            self.last_np_second = Some(current_second);
                            return;
                        }
                    }
                    let avg = self.current_second_power.iter().map(|&w| w as f64).sum::<f64>()
                        / self.current_second_power.len() as f64;
                    self.np_buffer.push_back(avg);
                    if self.np_buffer.len() > 30 {
                        self.np_buffer.pop_front();
                    }
                    // Once we have a full 30-second window, accumulate 4th power
                    if self.np_buffer.len() == 30 {
                        let rolling_avg: f64 = self.np_buffer.iter().sum::<f64>() / 30.0;
                        self.fourth_power_sum += rolling_avg.powi(4);
                        self.fourth_power_count += 1;
                    }
                }
                self.current_second_power.clear();
                self.current_second_power.push(watts);
                self.last_np_second = Some(current_second);
            }
        }
    }

    pub fn record_hr(&mut self, bpm: u8) {
        self.hr_readings.push(bpm);
    }

    pub fn record_cadence(&mut self, rpm: f32) {
        self.cadence_readings.push(rpm);
    }

    pub fn record_speed(&mut self, kmh: f32) {
        self.speed_readings.push(kmh);
    }

    pub fn current_power(&self) -> Option<u16> {
        self.power_history.last().map(|(_, w)| *w)
    }

    pub fn avg_power(&self, window_secs: usize) -> Option<f32> {
        let last_ms = self.last_epoch_ms?;
        if self.power_history.is_empty() {
            return None;
        }
        if window_secs == usize::MAX {
            // Session-wide average
            let sum: f32 = self.power_history.iter().map(|(_, w)| *w as f32).sum();
            return Some(sum / self.power_history.len() as f32);
        }
        let cutoff = last_ms.saturating_sub(window_secs as u64 * 1000);
        let slice: Vec<u16> = self.power_history.iter()
            .rev()
            .take_while(|(ts, _)| *ts >= cutoff)
            .map(|(_, w)| *w)
            .collect();
        if slice.is_empty() {
            return None;
        }
        Some(slice.iter().map(|&w| w as f32).sum::<f32>() / slice.len() as f32)
    }

    pub fn normalized_power(&self) -> Option<f32> {
        if self.fourth_power_count == 0 {
            return None;
        }
        Some((self.fourth_power_sum / self.fourth_power_count as f64).powf(0.25) as f32)
    }

    pub fn intensity_factor(&self) -> Option<f32> {
        self.normalized_power().map(|np| np / self.ftp as f32)
    }

    pub fn tss(&self, active_elapsed_secs: u64) -> Option<f32> {
        let np = self.normalized_power()?;
        let if_ = self.intensity_factor()?;
        let duration_s = active_elapsed_secs as f32;
        Some((duration_s * np * if_) / (self.ftp as f32 * 3600.0) * 100.0)
    }

    pub fn avg_hr(&self) -> Option<u8> {
        if self.hr_readings.is_empty() {
            return None;
        }
        let avg = self.hr_readings.iter().map(|&hr| hr as f32).sum::<f32>()
            / self.hr_readings.len() as f32;
        Some(avg.round() as u8)
    }

    pub fn max_hr(&self) -> Option<u8> {
        self.hr_readings.iter().max().copied()
    }

    pub fn current_hr(&self) -> Option<u8> {
        self.hr_readings.last().copied()
    }

    pub fn current_cadence(&self) -> Option<f32> {
        self.cadence_readings.last().copied()
    }

    pub fn current_speed(&self) -> Option<f32> {
        self.speed_readings.last().copied()
    }

    pub fn max_power(&self) -> Option<u16> {
        self.power_history.iter().map(|(_, w)| *w).max()
    }

    pub fn avg_cadence(&self) -> Option<f32> {
        let nonzero: Vec<f32> = self.cadence_readings.iter().copied().filter(|&v| v > 0.0).collect();
        if nonzero.is_empty() { return None; }
        Some(nonzero.iter().sum::<f32>() / nonzero.len() as f32)
    }

    pub fn avg_speed(&self) -> Option<f32> {
        let nonzero: Vec<f32> = self.speed_readings.iter().copied().filter(|&v| v > 0.0).collect();
        if nonzero.is_empty() { return None; }
        Some(nonzero.iter().sum::<f32>() / nonzero.len() as f32)
    }

    pub fn power_zone(&self, ftp: u16, zones: &[u16; 6]) -> Option<u8> {
        let watts = self.current_power()?;
        let pct = (watts as f32 / ftp.max(1) as f32) * 100.0;
        for (i, &upper) in zones.iter().enumerate() {
            if pct <= upper as f32 {
                return Some((i + 1) as u8);
            }
        }
        Some(7) // above all zone boundaries
    }

    pub fn hr_zone(&self, zones: &[u8; 5]) -> Option<u8> {
        let hr = self.current_hr()?;
        for (i, &upper) in zones.iter().enumerate() {
            if hr <= upper {
                return Some((i + 1) as u8);
            }
        }
        Some(5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_constant_power(calc: &mut MetricsCalculator, watts: u16, n: u64, start_sec: u64) {
        for i in 0..n {
            calc.record_power(watts, (start_sec + i) * 1000);
        }
    }

    fn assert_approx(actual: f32, expected: f32, epsilon: f32, msg: &str) {
        assert!(
            (actual - expected).abs() < epsilon,
            "{msg}: expected {expected} ± {epsilon}, got {actual}"
        );
    }

    // --- Empty State ---

    #[test]
    fn fresh_calculator_all_none() {
        let calc = MetricsCalculator::new(200);
        assert!(calc.normalized_power().is_none());
        assert!(calc.intensity_factor().is_none());
        assert!(calc.tss(3600).is_none());
        assert!(calc.avg_power(usize::MAX).is_none());
        assert!(calc.avg_power(30).is_none());
        assert!(calc.current_power().is_none());
        assert!(calc.max_power().is_none());
        assert!(calc.avg_hr().is_none());
        assert!(calc.max_hr().is_none());
        assert!(calc.current_hr().is_none());
        assert!(calc.current_cadence().is_none());
        assert!(calc.current_speed().is_none());
        assert!(calc.hr_zone(&[120, 140, 160, 180, 200]).is_none());
    }

    // --- NP Algorithm ---

    #[test]
    fn np_constant_power_equals_that_power() {
        let mut calc = MetricsCalculator::new(200);
        // 35 seconds of constant 200W — well past the 31-second threshold
        feed_constant_power(&mut calc, 200, 35, 0);
        let np = calc.normalized_power().unwrap();
        assert_approx(np, 200.0, 0.1, "constant power NP");
    }

    #[test]
    fn np_returns_none_before_30_seconds() {
        let mut calc = MetricsCalculator::new(200);
        // 25 distinct seconds (0..24) → only 24 flushed entries, buffer never reaches 30
        feed_constant_power(&mut calc, 200, 25, 0);
        assert!(calc.normalized_power().is_none());
    }

    #[test]
    fn np_available_at_exactly_31_seconds() {
        let mut calc = MetricsCalculator::new(200);
        // 31 distinct epoch-seconds (0..=30): second 30 triggers flush of second 29,
        // giving the 30th buffer entry and the first fourth-power accumulation
        feed_constant_power(&mut calc, 200, 31, 0);
        assert!(calc.normalized_power().is_some());
    }

    #[test]
    fn np_multiple_readings_same_second_averaged() {
        let mut calc = MetricsCalculator::new(200);
        // Seconds 0-4 at 200W
        feed_constant_power(&mut calc, 200, 5, 0);
        // Second 5: two readings that average to 200W
        calc.record_power(100, 5000);
        calc.record_power(300, 5000);
        // Seconds 6-30 at 200W
        feed_constant_power(&mut calc, 200, 25, 6);

        let np = calc.normalized_power().unwrap();
        assert_approx(np, 200.0, 0.1, "averaged same-second NP");
    }

    #[test]
    fn np_variable_sample_rate_same_result() {
        // 4 Hz sampling at 200W
        let mut calc_4hz = MetricsCalculator::new(200);
        for sec in 0..=30u64 {
            for sub in 0..4u64 {
                calc_4hz.record_power(200, sec * 1000 + sub * 250);
            }
        }
        // 1 Hz sampling at 200W
        let mut calc_1hz = MetricsCalculator::new(200);
        feed_constant_power(&mut calc_1hz, 200, 31, 0);

        let np_4hz = calc_4hz.normalized_power().unwrap();
        let np_1hz = calc_1hz.normalized_power().unwrap();
        assert_approx(np_4hz, 200.0, 0.1, "4Hz NP");
        assert_approx(np_1hz, 200.0, 0.1, "1Hz NP");
    }

    #[test]
    fn np_penalizes_variability() {
        let mut calc = MetricsCalculator::new(200);
        // 30s at 100W then 30s at 300W — mean is 200, but NP > 200
        feed_constant_power(&mut calc, 100, 30, 0);
        feed_constant_power(&mut calc, 300, 30, 30);

        let np = calc.normalized_power().unwrap();
        assert!(np > 200.0, "NP ({np}) should exceed mean power (200) due to variability");
    }

    // --- NP Gap Detection ---

    #[test]
    fn np_gap_resets_buffer() {
        let mut calc = MetricsCalculator::new(200);
        // 35 seconds at 200W → NP established
        feed_constant_power(&mut calc, 200, 35, 0);
        assert!(calc.normalized_power().is_some());
        // 5-second gap (jump from second 34 to second 40) → resets buffer
        // Then 35 more seconds at 200W
        feed_constant_power(&mut calc, 200, 35, 40);
        let np = calc.normalized_power().unwrap();
        // NP should be ~200, not corrupted by the gap
        assert_approx(np, 200.0, 0.1, "NP after gap reset");
    }

    #[test]
    fn np_one_second_gap_no_reset() {
        let mut calc = MetricsCalculator::new(200);
        // 35 seconds at 200W
        feed_constant_power(&mut calc, 200, 35, 0);
        let np_before = calc.normalized_power().unwrap();
        // 2-second gap (normal jitter: jump from second 34 to second 36)
        feed_constant_power(&mut calc, 200, 5, 36);
        let np_after = calc.normalized_power().unwrap();
        // Should still be valid, not reset
        assert_approx(np_after, 200.0, 0.1, "NP after small gap");
        assert_approx(np_before, np_after, 0.1, "NP unchanged by small gap");
    }

    // --- TSS ---

    #[test]
    fn tss_one_hour_at_ftp_equals_100() {
        let mut calc = MetricsCalculator::new(200);
        feed_constant_power(&mut calc, 200, 35, 0);
        // TSS = (duration * NP * IF) / (FTP * 3600) * 100
        // = (3600 * 200 * 1.0) / (200 * 3600) * 100 = 100.0
        let tss = calc.tss(3600).unwrap();
        assert_approx(tss, 100.0, 0.5, "1hr at FTP TSS");
    }

    #[test]
    fn tss_zero_duration_returns_zero() {
        let mut calc = MetricsCalculator::new(200);
        feed_constant_power(&mut calc, 200, 35, 0);
        let tss = calc.tss(0).unwrap();
        assert_approx(tss, 0.0, 0.01, "zero duration TSS");
    }

    #[test]
    fn tss_returns_none_without_np() {
        let calc = MetricsCalculator::new(200);
        assert!(calc.tss(3600).is_none());
    }

    // --- Intensity Factor ---

    #[test]
    fn intensity_factor_at_ftp_equals_one() {
        let mut calc = MetricsCalculator::new(200);
        feed_constant_power(&mut calc, 200, 35, 0);
        let if_ = calc.intensity_factor().unwrap();
        assert_approx(if_, 1.0, 0.01, "IF at FTP");
    }

    #[test]
    fn intensity_factor_returns_none_without_np() {
        let calc = MetricsCalculator::new(200);
        assert!(calc.intensity_factor().is_none());
    }

    // --- Rolling Average Power ---

    #[test]
    fn avg_power_empty_returns_none() {
        let calc = MetricsCalculator::new(200);
        assert!(calc.avg_power(usize::MAX).is_none());
        assert!(calc.avg_power(30).is_none());
    }

    #[test]
    fn avg_power_session_wide() {
        let mut calc = MetricsCalculator::new(200);
        calc.record_power(100, 0);
        calc.record_power(200, 1000);
        calc.record_power(300, 2000);
        let avg = calc.avg_power(usize::MAX).unwrap();
        assert_approx(avg, 200.0, 0.1, "session-wide avg");
    }

    #[test]
    fn avg_power_window_excludes_old_readings() {
        let mut calc = MetricsCalculator::new(200);
        calc.record_power(100, 0);       // 0s — outside 3s window
        calc.record_power(200, 5000);    // 5s — outside 3s window
        calc.record_power(300, 10000);   // 10s — inside window
        // cutoff = 10000 - 3000 = 7000; only 10000 >= 7000
        let avg = calc.avg_power(3).unwrap();
        assert_approx(avg, 300.0, 0.1, "windowed avg excludes old");
    }

    #[test]
    fn avg_power_window_boundary_is_inclusive() {
        let mut calc = MetricsCalculator::new(200);
        calc.record_power(100, 7000);   // 7s — at cutoff boundary
        calc.record_power(200, 10000);  // 10s
        // cutoff = 10000 - 3000 = 7000; both 10000 >= 7000 and 7000 >= 7000
        let avg = calc.avg_power(3).unwrap();
        assert_approx(avg, 150.0, 0.1, "boundary inclusive avg");
    }

    // --- FTP Guard ---

    #[test]
    fn ftp_zero_clamped_to_one() {
        let mut calc = MetricsCalculator::new(0);
        feed_constant_power(&mut calc, 200, 35, 0);
        let np = calc.normalized_power().unwrap();
        let if_ = calc.intensity_factor().unwrap();
        // ftp clamped to 1, so IF = NP / 1 = NP
        assert_approx(np, 200.0, 0.1, "NP with ftp=0");
        assert_approx(if_, 200.0, 0.1, "IF = NP/1");
    }

    // --- HR Zones ---

    #[test]
    fn hr_zone_at_boundary_goes_to_lower_zone() {
        let mut calc = MetricsCalculator::new(200);
        calc.record_hr(120);
        // 120 <= zones[0]=120 → zone 1 (lower zone, using <=)
        assert_eq!(calc.hr_zone(&[120, 140, 160, 180, 200]), Some(1));
    }

    #[test]
    fn hr_zone_above_all_returns_zone_5() {
        let mut calc = MetricsCalculator::new(200);
        calc.record_hr(210);
        // 210 > all zone boundaries → falls through to zone 5
        assert_eq!(calc.hr_zone(&[120, 140, 160, 180, 200]), Some(5));
    }

    #[test]
    fn hr_zone_below_first_boundary() {
        let mut calc = MetricsCalculator::new(200);
        calc.record_hr(80);
        // 80 <= zones[0]=120 → zone 1
        assert_eq!(calc.hr_zone(&[120, 140, 160, 180, 200]), Some(1));
    }

    #[test]
    fn hr_zone_no_hr_returns_none() {
        let calc = MetricsCalculator::new(200);
        assert_eq!(calc.hr_zone(&[120, 140, 160, 180, 200]), None);
    }

    // --- Power Zones ---

    const DEFAULT_POWER_ZONES: [u16; 6] = [55, 75, 90, 105, 120, 150];

    #[test]
    fn power_zone_below_first_boundary() {
        // 100W at FTP=200 → 50% → Z1 (≤55%)
        let mut calc = MetricsCalculator::new(200);
        calc.record_power(100, 1000);
        assert_eq!(calc.power_zone(200, &DEFAULT_POWER_ZONES), Some(1));
    }

    #[test]
    fn power_zone_at_boundary() {
        // 110W at FTP=200 → 55% → Z1 (≤55%)
        let mut calc = MetricsCalculator::new(200);
        calc.record_power(110, 1000);
        assert_eq!(calc.power_zone(200, &DEFAULT_POWER_ZONES), Some(1));
    }

    #[test]
    fn power_zone_above_all() {
        // 400W at FTP=200 → 200% → Z7 (>150%)
        let mut calc = MetricsCalculator::new(200);
        calc.record_power(400, 1000);
        assert_eq!(calc.power_zone(200, &DEFAULT_POWER_ZONES), Some(7));
    }

    #[test]
    fn power_zone_no_power_returns_none() {
        let calc = MetricsCalculator::new(200);
        assert_eq!(calc.power_zone(200, &DEFAULT_POWER_ZONES), None);
    }

    // --- HR Stats ---

    #[test]
    fn avg_hr_rounds_correctly() {
        let mut calc = MetricsCalculator::new(200);
        calc.record_hr(149);
        calc.record_hr(150);
        // avg = 149.5, rounds to 150
        assert_eq!(calc.avg_hr(), Some(150));
    }

    #[test]
    fn max_hr_returns_maximum() {
        let mut calc = MetricsCalculator::new(200);
        calc.record_hr(120);
        calc.record_hr(180);
        calc.record_hr(150);
        assert_eq!(calc.max_hr(), Some(180));
    }

    // --- Zero-Filtered Averages ---

    #[test]
    fn avg_cadence_excludes_zeros() {
        let mut calc = MetricsCalculator::new(200);
        calc.record_cadence(90.0);
        calc.record_cadence(0.0);
        calc.record_cadence(90.0);
        assert_approx(calc.avg_cadence().unwrap(), 90.0, 0.1, "avg cadence excluding zeros");
    }

    #[test]
    fn avg_speed_excludes_zeros() {
        let mut calc = MetricsCalculator::new(200);
        calc.record_speed(30.0);
        calc.record_speed(0.0);
        calc.record_speed(30.0);
        assert_approx(calc.avg_speed().unwrap(), 30.0, 0.1, "avg speed excluding zeros");
    }

    #[test]
    fn avg_cadence_all_zeros_returns_none() {
        let mut calc = MetricsCalculator::new(200);
        calc.record_cadence(0.0);
        calc.record_cadence(0.0);
        calc.record_cadence(0.0);
        assert!(calc.avg_cadence().is_none());
    }
}
