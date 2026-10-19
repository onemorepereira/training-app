//! HR-mode power controller (see docs/design/zone-control-safety/design.md ADR-1, ADR-14).
//!
//! Positional PID around a base power:
//!   v_k = u_base + Kp·e_k + I_k + D_k        (I kept in watts → bumpless gain changes)
//!   b_k = clamp(v_k, floor, ceiling)          (power band)
//!   u_k = clamp(b_k, u_{k−1} − down, u_{k−1} + up_k)   (rate limit)
//! Anti-windup by conditional integration: the integral only moves while the
//! output is unsaturated, or when the error drives the demand back toward the
//! feasible range. The derivative acts on the measurement (no setpoint kick).

/// Maximum watts per tick when ramping UP (normal)
pub const HR_MAX_WATTS_UP_PER_TICK: f64 = 10.0;
/// Maximum watts per tick when ramping UP while well below the band midpoint
pub const HR_MAX_WATTS_UP_RECOVERY_PER_TICK: f64 = 20.0;
/// Maximum watts per tick when ramping DOWN (faster — reducing power is always safe)
pub const HR_MAX_WATTS_DOWN_PER_TICK: f64 = 30.0;
/// Integral decay factor when HR is above target but already falling
pub const INTEGRAL_DECAY_ON_FALLING_HR: f64 = 0.7;
/// Errors within ±this many bpm are treated as zero for P and I, so ±3 bpm
/// sensor noise (±1–2 bpm after the median filter) doesn't hunt the power
/// (design ADR-27; tuned against the noisy plant grid).
pub const HR_ERROR_DEADBAND_BPM: f64 = 1.0;

/// Inclusive power band the controller may command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerBand {
    pub floor: u16,
    pub ceiling: u16,
}

impl PowerBand {
    fn midpoint(&self) -> f64 {
        (self.floor as f64 + self.ceiling as f64) / 2.0
    }
}

pub struct HrPid {
    base: f64,
    /// Integral contribution in watts
    integral_w: f64,
    prev_measurement: Option<f64>,
    /// Last commanded output u_{k−1} (watts, already rounded)
    last_output: f64,
}

impl HrPid {
    pub fn new(base_watts: u16) -> Self {
        Self {
            base: base_watts as f64,
            integral_w: 0.0,
            prev_measurement: None,
            last_output: base_watts as f64,
        }
    }

    /// One controller step. Returns the new commanded power.
    /// A non-positive or non-finite `dt_secs` leaves all state untouched and
    /// returns the previous output (0 W adjustment, never NaN).
    pub fn update(&mut self, target_hr: f64, measured_hr: f64, dt_secs: f64, band: PowerBand) -> u16 {
        if !(dt_secs > 0.0) || !dt_secs.is_finite() || !measured_hr.is_finite() {
            return self.last_output as u16;
        }
        let raw_error = target_hr - measured_hr;
        let error = if raw_error.abs() <= HR_ERROR_DEADBAND_BPM { 0.0 } else { raw_error };
        let (kp, ki, kd) = adaptive_gains(error.abs());

        // Decay integral when HR is above target and falling tick-over-tick
        if error < 0.0 {
            if let Some(prev) = self.prev_measurement {
                if measured_hr < prev {
                    self.integral_w *= INTEGRAL_DECAY_ON_FALLING_HR;
                }
            }
        }

        let p = kp * error;
        let d = match self.prev_measurement {
            Some(prev) => -kd * (measured_hr - prev) / dt_secs,
            None => 0.0,
        };
        let integral_candidate = self.integral_w + ki * error * dt_secs;
        let demand = self.base + p + integral_candidate + d;

        let floor = band.floor as f64;
        let ceiling = band.ceiling.max(band.floor) as f64;
        let banded = demand.clamp(floor, ceiling);
        let max_up = if error > 0.0 && self.last_output < band.midpoint() - 20.0 {
            HR_MAX_WATTS_UP_RECOVERY_PER_TICK
        } else {
            HR_MAX_WATTS_UP_PER_TICK
        };
        let output = banded.clamp(
            self.last_output - HR_MAX_WATTS_DOWN_PER_TICK,
            self.last_output + max_up,
        );

        let saturated = (output - demand).abs() > 1e-9;
        let leaving_saturation =
            (demand > output && error < 0.0) || (demand < output && error > 0.0);
        if !saturated || leaving_saturation {
            self.integral_w = integral_candidate;
        }
        self.integral_w = self
            .integral_w
            .clamp((floor - self.base).min(0.0), (ceiling - self.base).max(0.0));

        self.prev_measurement = Some(measured_hr);
        self.last_output = output.round().max(0.0);
        self.last_output as u16
    }

    /// Re-seed after a period where the output was not delivered (ADR-14):
    /// the next step ramps from `delivered_watts`; derivative history is dropped.
    pub fn reseed(&mut self, delivered_watts: u16) {
        self.last_output = delivered_watts as f64;
        self.prev_measurement = None;
    }

    pub fn integral(&self) -> f64 {
        self.integral_w
    }

    pub fn last_output(&self) -> u16 {
        self.last_output as u16
    }
}

use std::collections::VecDeque;

pub struct HrSmoother {
    buffer: VecDeque<u8>,
    window_size: usize,
}

impl HrSmoother {
    pub fn new(window_size: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(window_size),
            window_size,
        }
    }

    pub fn push(&mut self, bpm: u8) {
        if self.buffer.len() >= self.window_size {
            self.buffer.pop_front();
        }
        self.buffer.push_back(bpm);
    }

    pub fn smoothed(&self) -> Option<u8> {
        if self.buffer.is_empty() {
            return None;
        }
        let mut sorted: Vec<u8> = self.buffer.iter().copied().collect();
        sorted.sort_unstable();
        Some(sorted[sorted.len() / 2])
    }
}

/// Returns (Kp W/bpm, Ki W/(bpm·s), Kd W·s/bpm) for the positional law,
/// scheduled on distance from target. Tuned against the plant-simulation
/// harness (design ADR-1): old gains scaled by (0.75, 0.25, 1.0).
pub fn adaptive_gains(error_abs: f64) -> (f64, f64, f64) {
    if error_abs > 15.0 {
        // Far from target
        (2.25, 0.0375, 0.8)
    } else if error_abs > 5.0 {
        // Getting close
        (1.5, 0.025, 0.5)
    } else {
        // In/near zone — gentle maintenance
        (0.75, 0.0125, 0.3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_approx(actual: f64, expected: f64, epsilon: f64, msg: &str) {
        assert!(
            (actual - expected).abs() <= epsilon,
            "{msg}: expected {expected} ± {epsilon}, got {actual}"
        );
    }

    const WIDE: PowerBand = PowerBand { floor: 50, ceiling: 375 };

    #[test]
    fn hr_pid_first_tick_reference_value() {
        // base 150, target 135, HR 125 → e=10 → moderate gains (1.5, 0.025, 0.5)
        // P = 15, I = 0.025·10·5 = 1.25, D = 0 (first tick) → v = 166.25
        // band [50,375] ok; up limit: 150 < 212.5-20 → 20 W → 166.25 unsaturated
        let mut pid = HrPid::new(150);
        let out = pid.update(135.0, 125.0, 5.0, WIDE);
        assert_eq!(out, 166);
        assert_approx(pid.integral(), 1.25, 0.01, "integral after first tick");
    }

    #[test]
    fn hr_pid_zero_error_stable_hr_holds_power() {
        // Build some integral, then hold HR at target: output must stop changing.
        let mut pid = HrPid::new(150);
        for _ in 0..3 {
            pid.update(135.0, 125.0, 5.0, WIDE);
        }
        let mut outputs = Vec::new();
        for _ in 0..30 {
            outputs.push(pid.update(135.0, 135.0, 5.0, WIDE));
        }
        let tail = &outputs[20..];
        assert!(
            tail.iter().all(|&w| w == tail[0]),
            "power must be constant with zero error, got {tail:?}"
        );
    }

    #[test]
    fn hr_pid_ignores_error_within_1bpm_deadband() {
        // Settle with some integral, then HR 1 bpm off target either way:
        // P and I see zero error, so power and integral stay put.
        let mut pid = HrPid::new(150);
        for _ in 0..3 {
            pid.update(135.0, 125.0, 5.0, WIDE);
        }
        for _ in 0..20 {
            pid.update(135.0, 135.0, 5.0, WIDE);
        }
        let (w, i) = (pid.last_output(), pid.integral());
        assert_eq!(pid.update(135.0, 136.0, 5.0, WIDE), w); // D = −0.3·1/5 = −0.06 → rounds away
        assert_eq!(pid.update(135.0, 135.0, 5.0, WIDE), w);
        assert_eq!(pid.update(135.0, 134.0, 5.0, WIDE), w);
        assert_approx(pid.integral(), i, 1e-9, "integral unchanged inside deadband");
        // 2 bpm off is outside the deadband: I moves by 0.0125·2·5 = 0.125
        pid.update(135.0, 133.0, 5.0, WIDE);
        assert_approx(pid.integral(), i + 0.125, 1e-9, "integral moves outside deadband");
    }

    #[test]
    fn hr_pid_integral_frozen_while_at_band_ceiling() {
        // Ceiling 160 just above base 150; HR far below target → demand >> ceiling.
        let band = PowerBand { floor: 50, ceiling: 160 };
        let mut pid = HrPid::new(150);
        let mut out = 0;
        for _ in 0..10 {
            out = pid.update(135.0, 100.0, 5.0, band);
        }
        assert_eq!(out, 160);
        assert_approx(pid.integral(), 0.0, 0.01, "no windup at ceiling");
    }

    #[test]
    fn hr_pid_integral_frozen_while_rate_limited() {
        // e=35 → P=78.75: demand far above 100+20 → rate limited, integral must not move
        let mut pid = HrPid::new(100);
        let out = pid.update(135.0, 100.0, 5.0, WIDE);
        assert_eq!(out, 120);
        assert_approx(pid.integral(), 0.0, 0.01, "no windup while rate limited");
    }

    #[test]
    fn hr_pid_integral_unwinds_when_error_reverses_at_ceiling() {
        // Near-target error (e=5, gentle gains) accumulates I until the ceiling binds.
        let band = PowerBand { floor: 50, ceiling: 160 };
        let mut pid = HrPid::new(150);
        for _ in 0..60 {
            pid.update(135.0, 130.0, 5.0, band);
        }
        assert_eq!(pid.last_output(), 160);
        let before = pid.integral();
        // HR now 2 above target (rising, so no decay): I += 0.0125·(−2)·5 = −0.125
        pid.update(135.0, 137.0, 5.0, band);
        assert_approx(pid.integral(), before - 0.125, 0.001, "integral unwinds");
    }

    #[test]
    fn hr_pid_zero_dt_returns_unchanged_power() {
        let mut pid = HrPid::new(150);
        pid.update(135.0, 125.0, 5.0, WIDE); // 166
        let integral = pid.integral();
        let out = pid.update(135.0, 100.0, 0.0, WIDE);
        assert_eq!(out, 166);
        assert_approx(pid.integral(), integral, 1e-9, "state untouched by dt=0");
    }

    #[test]
    fn hr_pid_negative_dt_returns_unchanged_power() {
        let mut pid = HrPid::new(150);
        let out = pid.update(135.0, 100.0, -5.0, WIDE);
        assert_eq!(out, 150);
        let nan = pid.update(135.0, 100.0, f64::NAN, WIDE);
        assert_eq!(nan, 150);
    }

    #[test]
    fn ramp_down_faster_than_ramp_up() {
        // HR 20 above target from 200 W: down limit 30 W/tick
        let mut pid = HrPid::new(200);
        let out = pid.update(135.0, 155.0, 5.0, WIDE);
        assert_eq!(out, 170);
    }

    #[test]
    fn recovery_ramp_up_is_20w_below_band_midpoint() {
        // band [110,180] midpoint 145; at 110 W (< 125) with HR below target → +20
        let band = PowerBand { floor: 110, ceiling: 180 };
        let mut pid = HrPid::new(110);
        assert_eq!(pid.update(145.0, 120.0, 5.0, band), 130);
    }

    #[test]
    fn normal_ramp_up_is_10w_near_band_midpoint() {
        let band = PowerBand { floor: 110, ceiling: 180 };
        let mut pid = HrPid::new(140);
        assert_eq!(pid.update(145.0, 120.0, 5.0, band), 150);
    }

    #[test]
    fn output_below_band_ramps_into_band_under_rate_limit() {
        // Re-seeded at 0 W (e.g. after cadence cut): band floor 110 must be
        // approached at the up rate, not jumped to.
        let band = PowerBand { floor: 110, ceiling: 180 };
        let mut pid = HrPid::new(140);
        pid.reseed(0);
        assert_eq!(pid.update(145.0, 120.0, 5.0, band), 20);
        assert_eq!(pid.update(145.0, 120.0, 5.0, band), 40);
    }

    #[test]
    fn reseed_drops_derivative_history() {
        // Tick 1 at HR 150: I = −1.875. Reseed to 150 W, then HR 133 (e=2, gentle):
        // P = 1.5, I = −1.875 + 0.125 = −1.75, D = 0 → v = 149.75 → 150 W.
        // Had the stale measurement been kept, D = −0.3·(133−150)/5 = +1.02 → 151 W.
        let mut pid = HrPid::new(150);
        pid.update(135.0, 150.0, 5.0, WIDE);
        pid.reseed(150);
        assert_eq!(pid.update(135.0, 133.0, 5.0, WIDE), 150);
    }

    #[test]
    fn integral_decays_when_smoothed_hr_falls_between_ticks() {
        // Two real ticks above target: 150 then 148 (falling).
        // Tick 1: e=−15 → moderate, I = 0.025·(−15)·5 = −1.875;
        //   v = 200 − 22.5 − 1.875 = 175.6 → within the −30 limit → unsaturated, accepted
        // Tick 2: decay → −1.3125, then + 0.025·(−13)·5 = −1.625 → −2.9375
        let mut pid = HrPid::new(200);
        pid.update(135.0, 150.0, 5.0, WIDE);
        assert_approx(pid.integral(), -1.875, 0.001, "after tick 1");
        pid.update(135.0, 148.0, 5.0, WIDE);
        assert_approx(pid.integral(), -2.9375, 0.001, "decayed on falling HR");
    }

    #[test]
    fn no_integral_decay_when_smoothed_hr_rises_above_zone() {
        // 150 then 152 (rising): no decay. Tick 2 e=−17 → far gains (Ki 0.0375):
        // −1.875 + 0.0375·(−17)·5 = −5.0625 (decay would have given −4.475)
        let mut pid = HrPid::new(200);
        pid.update(135.0, 150.0, 5.0, WIDE);
        pid.update(135.0, 152.0, 5.0, WIDE);
        assert_approx(pid.integral(), -5.0625, 0.001, "no decay when rising");
    }

    #[test]
    fn no_integral_decay_when_hr_below_target_and_falling() {
        // 125 then 120 (below target, falling) → no decay.
        // tick1 e=10 moderate → 1.25; tick2 e=15 (not > 15) moderate → +1.875 = 3.125
        let mut pid = HrPid::new(150);
        pid.update(135.0, 125.0, 5.0, WIDE);
        pid.update(135.0, 120.0, 5.0, WIDE);
        assert_approx(pid.integral(), 3.125, 0.001, "no decay below target");
    }

    // --- HrSmoother tests ---

    #[test]
    fn smoother_single_reading_returns_that_reading() {
        let mut s = HrSmoother::new(5);
        s.push(140);
        assert_eq!(s.smoothed(), Some(140));
    }

    #[test]
    fn smoother_median_rejects_spike() {
        // [100, 200, 150] sorted = [100, 150, 200], median = 150
        let mut s = HrSmoother::new(5);
        s.push(100);
        s.push(200);
        s.push(150);
        assert_eq!(s.smoothed(), Some(150));
    }

    #[test]
    fn smoother_window_overflow_drops_oldest() {
        let mut s = HrSmoother::new(3);
        s.push(100);
        s.push(110);
        s.push(120);
        assert_eq!(s.smoothed(), Some(110));
        s.push(200);
        assert_eq!(s.smoothed(), Some(120));
    }

    #[test]
    fn smoother_empty_returns_none() {
        let s = HrSmoother::new(5);
        assert_eq!(s.smoothed(), None);
    }

    // --- adaptive_gains tests ---

    #[test]
    fn adaptive_gains_far_from_target() {
        let (kp, ki, kd) = adaptive_gains(20.0);
        assert_approx(kp, 2.25, 0.001, "far kp");
        assert_approx(ki, 0.0375, 0.0001, "far ki");
        assert_approx(kd, 0.8, 0.001, "far kd");
    }

    #[test]
    fn adaptive_gains_boundary_15_is_moderate() {
        let (kp, ki, kd) = adaptive_gains(15.0);
        assert_approx(kp, 1.5, 0.001, "boundary 15 kp");
        assert_approx(ki, 0.025, 0.0001, "boundary 15 ki");
        assert_approx(kd, 0.5, 0.001, "boundary 15 kd");
    }

    #[test]
    fn adaptive_gains_boundary_5_is_gentle() {
        let (kp, ki, kd) = adaptive_gains(5.0);
        assert_approx(kp, 0.75, 0.001, "boundary 5 kp");
        assert_approx(ki, 0.0125, 0.0001, "boundary 5 ki");
        assert_approx(kd, 0.3, 0.001, "boundary 5 kd");
    }
}
