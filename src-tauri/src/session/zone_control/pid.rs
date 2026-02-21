pub struct PidController {
    kp: f64,
    ki: f64,
    kd: f64,
    integral: f64,
    prev_error: Option<f64>,
    integral_limit: f64,
    output_limit: f64,
}

impl PidController {
    pub fn new(kp: f64, ki: f64, kd: f64) -> Self {
        Self::with_limits(kp, ki, kd, 200.0, 30.0)
    }

    pub fn with_limits(kp: f64, ki: f64, kd: f64, integral_limit: f64, output_limit: f64) -> Self {
        Self {
            kp,
            ki,
            kd,
            integral: 0.0,
            prev_error: None,
            integral_limit,
            output_limit,
        }
    }

    pub fn update(&mut self, error: f64, dt_secs: f64) -> f64 {
        // Proportional
        let p = self.kp * error;

        // Integral with anti-windup
        self.integral += error * dt_secs;
        self.integral = self.integral.clamp(-self.integral_limit, self.integral_limit);
        let i = self.ki * self.integral;

        // Derivative
        let d = match self.prev_error {
            Some(prev) => self.kd * (error - prev) / dt_secs,
            None => 0.0,
        };
        self.prev_error = Some(error);

        let output = p + i + d;
        output.clamp(-self.output_limit, self.output_limit)
    }

    pub fn reset(&mut self) {
        self.integral = 0.0;
        self.prev_error = None;
    }

    pub fn set_gains(&mut self, kp: f64, ki: f64, kd: f64) {
        self.kp = kp;
        self.ki = ki;
        self.kd = kd;
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

/// Returns (kp, ki, kd) tuned for distance from target.
pub fn adaptive_gains(error_abs: f64) -> (f64, f64, f64) {
    if error_abs > 15.0 {
        // Far from target — aggressive ramp
        (3.0, 0.15, 0.8)
    } else if error_abs > 5.0 {
        // Getting close — moderate
        (2.0, 0.10, 0.5)
    } else {
        // In/near zone — gentle maintenance
        (1.0, 0.05, 0.3)
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

    #[test]
    fn proportional_only_positive_error() {
        // P-only: error=10, kp=2.0 → output=20.0
        let mut pid = PidController::with_limits(2.0, 0.0, 0.0, 200.0, 100.0);
        let out = pid.update(10.0, 1.0);
        assert_approx(out, 20.0, 0.01, "P-only positive error");
    }

    #[test]
    fn proportional_only_negative_error() {
        // P-only: error=-5, kp=2.0 → output=-10.0
        let mut pid = PidController::with_limits(2.0, 0.0, 0.0, 200.0, 100.0);
        let out = pid.update(-5.0, 1.0);
        assert_approx(out, -10.0, 0.01, "P-only negative error");
    }

    #[test]
    fn integral_accumulates_over_ticks() {
        // I-only: error=5, ki=0.1, dt=5 → integral=25, output=2.5
        let mut pid = PidController::with_limits(0.0, 0.1, 0.0, 200.0, 100.0);
        let out1 = pid.update(5.0, 5.0);
        assert_approx(out1, 2.5, 0.01, "I-only first tick");

        // Second tick: integral=50, output=5.0
        let out2 = pid.update(5.0, 5.0);
        assert_approx(out2, 5.0, 0.01, "I-only second tick");
    }

    #[test]
    fn anti_windup_clamps_integral() {
        // ki=1.0, error=1000, dt=1 → integral would be 1000 but clamped to 200
        let mut pid = PidController::with_limits(0.0, 1.0, 0.0, 200.0, 1000.0);
        let out = pid.update(1000.0, 1.0);
        // integral clamped to 200, output = 1.0 * 200 = 200
        assert_approx(out, 200.0, 0.01, "anti-windup clamps integral");
    }

    #[test]
    fn derivative_responds_to_error_change() {
        // D-only: first tick error=10, second tick error=5, kd=1.0, dt=5
        // derivative = (5 - 10) / 5 = -1.0, output = 1.0 * -1.0 = -1.0
        let mut pid = PidController::with_limits(0.0, 0.0, 1.0, 200.0, 100.0);
        let out1 = pid.update(10.0, 5.0);
        assert_approx(out1, 0.0, 0.01, "D-only first tick (no prev)");

        let out2 = pid.update(5.0, 5.0);
        assert_approx(out2, -1.0, 0.01, "D-only second tick");
    }

    #[test]
    fn output_clamp_limits_extreme_adjustments() {
        // Large error with output_limit=30 → clamped to 30
        let mut pid = PidController::with_limits(10.0, 0.0, 0.0, 200.0, 30.0);
        let out = pid.update(100.0, 1.0);
        assert_approx(out, 30.0, 0.01, "output clamped to 30");

        // Negative extreme
        let out_neg = pid.update(-100.0, 1.0);
        assert_approx(out_neg, -30.0, 0.01, "output clamped to -30");
    }

    #[test]
    fn full_pid_first_tick() {
        // kp=2, ki=0.1, kd=0.5, error=10, dt=5
        // P = 2 * 10 = 20
        // I = 0.1 * (10 * 5) = 0.1 * 50 = 5
        // D = 0 (first tick, no prev_error)
        // total = 25, output_limit=30 → output=25
        let mut pid = PidController::with_limits(2.0, 0.1, 0.5, 200.0, 30.0);
        let out = pid.update(10.0, 5.0);
        assert_approx(out, 25.0, 0.01, "full PID first tick");
    }

    #[test]
    fn reset_clears_state() {
        let mut pid = PidController::with_limits(0.0, 1.0, 1.0, 200.0, 100.0);
        pid.update(10.0, 1.0);
        pid.update(20.0, 1.0);

        pid.reset();

        // After reset, integral should be 0 and no prev_error
        // I-only: error=5, dt=1 → integral=5, output=5
        let out = pid.update(5.0, 1.0);
        assert_approx(out, 5.0, 0.01, "after reset, integral starts fresh");
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
        // Window: [100, 110, 120], median = 110
        assert_eq!(s.smoothed(), Some(110));

        s.push(200);
        // Window: [110, 120, 200], median = 120
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
        // error=20 → aggressive gains
        let (kp, ki, kd) = adaptive_gains(20.0);
        assert_approx(kp, 3.0, 0.01, "far kp");
        assert_approx(ki, 0.15, 0.01, "far ki");
        assert_approx(kd, 0.8, 0.01, "far kd");
    }

    #[test]
    fn adaptive_gains_moderate_distance() {
        // error=10 → moderate gains
        let (kp, ki, kd) = adaptive_gains(10.0);
        assert_approx(kp, 2.0, 0.01, "moderate kp");
        assert_approx(ki, 0.10, 0.01, "moderate ki");
        assert_approx(kd, 0.5, 0.01, "moderate kd");
    }

    #[test]
    fn adaptive_gains_near_target() {
        // error=3 → gentle gains
        let (kp, ki, kd) = adaptive_gains(3.0);
        assert_approx(kp, 1.0, 0.01, "gentle kp");
        assert_approx(ki, 0.05, 0.01, "gentle ki");
        assert_approx(kd, 0.3, 0.01, "gentle kd");
    }

    #[test]
    fn adaptive_gains_zero_error() {
        let (kp, ki, kd) = adaptive_gains(0.0);
        assert_approx(kp, 1.0, 0.01, "zero kp");
        assert_approx(ki, 0.05, 0.01, "zero ki");
        assert_approx(kd, 0.3, 0.01, "zero kd");
    }

    #[test]
    fn adaptive_gains_boundary_15() {
        // error=15 exactly → moderate (> check, not >=)
        let (kp, ki, kd) = adaptive_gains(15.0);
        assert_approx(kp, 2.0, 0.01, "boundary 15 kp");
        assert_approx(ki, 0.10, 0.01, "boundary 15 ki");
        assert_approx(kd, 0.5, 0.01, "boundary 15 kd");
    }

    #[test]
    fn adaptive_gains_boundary_5() {
        // error=5 exactly → gentle (> check, not >=)
        let (kp, ki, kd) = adaptive_gains(5.0);
        assert_approx(kp, 1.0, 0.01, "boundary 5 kp");
        assert_approx(ki, 0.05, 0.01, "boundary 5 ki");
        assert_approx(kd, 0.3, 0.01, "boundary 5 kd");
    }
}
