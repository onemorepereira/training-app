#![cfg(not(feature = "production"))]

use crate::device::types::SensorReading;
use serde::{Deserialize, Serialize};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimStatus {
    Stopped,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimProfile {
    SteadyState,
    Intervals,
    Ramp,
    Stochastic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimStatusResponse {
    pub status: SimStatus,
    pub profile: SimProfile,
}

struct Segment {
    duration_secs: f64,
    power_start: f64,
    power_end: f64,
    noise_amplitude: f64,
}

pub struct Simulator {
    task_handle: Option<JoinHandle<()>>,
    status: SimStatus,
    profile: SimProfile,
}

// Minimal xorshift64 PRNG — no external deps needed.
struct Xorshift64(u64);

impl Xorshift64 {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 { 1 } else { seed })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Returns a value in [-1.0, 1.0].
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() as f64 / u64::MAX as f64) * 2.0 - 1.0
    }
}

fn epoch_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn segments_for_profile(profile: SimProfile) -> Vec<Segment> {
    match profile {
        SimProfile::SteadyState => vec![Segment {
            duration_secs: 600.0,
            power_start: 200.0,
            power_end: 200.0,
            noise_amplitude: 3.0,
        }],
        SimProfile::Intervals => {
            let mut segs = Vec::with_capacity(8);
            for _ in 0..4 {
                segs.push(Segment {
                    duration_secs: 300.0,
                    power_start: 140.0,
                    power_end: 140.0,
                    noise_amplitude: 5.0,
                });
                segs.push(Segment {
                    duration_secs: 180.0,
                    power_start: 280.0,
                    power_end: 280.0,
                    noise_amplitude: 8.0,
                });
            }
            segs
        }
        SimProfile::Ramp => vec![Segment {
            duration_secs: 900.0,
            power_start: 100.0,
            power_end: 350.0,
            noise_amplitude: 5.0,
        }],
        SimProfile::Stochastic => vec![
            // Block 1 (~10 min): warmup + endurance
            Segment {
                duration_secs: 180.0,
                power_start: 100.0,
                power_end: 150.0,
                noise_amplitude: 5.0,
            },
            Segment {
                duration_secs: 300.0,
                power_start: 180.0,
                power_end: 200.0,
                noise_amplitude: 10.0,
            },
            Segment {
                duration_secs: 120.0,
                power_start: 120.0,
                power_end: 120.0,
                noise_amplitude: 3.0,
            },
            // Block 2 (~10 min): intervals with sprints
            Segment {
                duration_secs: 240.0,
                power_start: 220.0,
                power_end: 240.0,
                noise_amplitude: 15.0,
            },
            Segment {
                duration_secs: 30.0,
                power_start: 400.0,
                power_end: 450.0,
                noise_amplitude: 20.0,
            },
            Segment {
                duration_secs: 120.0,
                power_start: 100.0,
                power_end: 130.0,
                noise_amplitude: 5.0,
            },
            Segment {
                duration_secs: 180.0,
                power_start: 250.0,
                power_end: 270.0,
                noise_amplitude: 15.0,
            },
            Segment {
                duration_secs: 30.0,
                power_start: 420.0,
                power_end: 480.0,
                noise_amplitude: 25.0,
            },
            // Block 3 (~10 min): tempo + cooldown
            Segment {
                duration_secs: 120.0,
                power_start: 130.0,
                power_end: 150.0,
                noise_amplitude: 5.0,
            },
            Segment {
                duration_secs: 360.0,
                power_start: 200.0,
                power_end: 220.0,
                noise_amplitude: 10.0,
            },
            Segment {
                duration_secs: 120.0,
                power_start: 150.0,
                power_end: 100.0,
                noise_amplitude: 5.0,
            },
        ],
    }
}

fn total_duration(segments: &[Segment]) -> f64 {
    segments.iter().map(|s| s.duration_secs).sum()
}

fn power_at_time(segments: &[Segment], elapsed_secs: f64, rng: &mut Xorshift64) -> f64 {
    let total = total_duration(segments);
    if total == 0.0 {
        return 0.0;
    }
    let t = elapsed_secs % total;

    let mut acc = 0.0;
    for seg in segments {
        if t < acc + seg.duration_secs {
            let frac = (t - acc) / seg.duration_secs;
            let base = seg.power_start + (seg.power_end - seg.power_start) * frac;
            let noise = rng.next_f64() * seg.noise_amplitude;
            return (base + noise).max(0.0);
        }
        acc += seg.duration_secs;
    }
    // Should not reach here due to modulo, but handle edge case
    segments.last().map_or(0.0, |s| s.power_end)
}

fn hr_update(hr: f64, power: f64, dt_secs: f64) -> f64 {
    let hr_ss = 60.0 + 0.4 * power;
    let tau = if hr_ss > hr { 30.0 } else { 45.0 };
    hr + (hr_ss - hr) * (1.0 - (-dt_secs / tau).exp())
}

fn cadence_from_power(power: f64) -> f32 {
    (85.0 + (power - 150.0) * 0.02).clamp(70.0, 110.0) as f32
}

fn speed_from_power(power: f64) -> f32 {
    (4.0 * power.max(0.0).cbrt()) as f32
}

impl Simulator {
    pub fn new() -> Self {
        Self {
            task_handle: None,
            status: SimStatus::Stopped,
            profile: SimProfile::SteadyState,
        }
    }

    pub fn start(&mut self, profile: SimProfile, sensor_tx: broadcast::Sender<SensorReading>) {
        self.stop();
        self.profile = profile;
        self.status = SimStatus::Running;

        let segments = segments_for_profile(profile);
        let handle = tokio::spawn(async move {
            let mut hr = 60.0_f64;
            let mut rng = Xorshift64::new(0xdeadbeef_cafe1234);
            let mut tick = 0u64;
            let start = tokio::time::Instant::now();
            let mut interval = tokio::time::interval(Duration::from_millis(250));

            loop {
                interval.tick().await;
                let elapsed = start.elapsed().as_secs_f64();
                let power = power_at_time(&segments, elapsed, &mut rng);

                let epoch_ms = epoch_now();
                let _ = sensor_tx.send(SensorReading::Power {
                    watts: power.round() as u16,
                    timestamp: Some(Instant::now()),
                    epoch_ms,
                    device_id: "sim:power".to_string(),
                    pedal_balance: None,
                });

                // HR, cadence, speed at 1 Hz (every 4th tick)
                if tick % 4 == 0 {
                    hr = hr_update(hr, power, 1.0);

                    let epoch_ms = epoch_now();
                    let _ = sensor_tx.send(SensorReading::HeartRate {
                        bpm: (hr.round() as u8).max(40),
                        timestamp: Some(Instant::now()),
                        epoch_ms,
                        device_id: "sim:hr".to_string(),
                    });

                    let _ = sensor_tx.send(SensorReading::Cadence {
                        rpm: cadence_from_power(power),
                        timestamp: Some(Instant::now()),
                        epoch_ms,
                        device_id: "sim:cadence".to_string(),
                    });

                    let _ = sensor_tx.send(SensorReading::Speed {
                        kmh: speed_from_power(power),
                        timestamp: Some(Instant::now()),
                        epoch_ms,
                        device_id: "sim:speed".to_string(),
                    });
                }

                tick += 1;
            }
        });

        self.task_handle = Some(handle);
    }

    pub fn stop(&mut self) {
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }
        self.status = SimStatus::Stopped;
    }

    pub fn status(&self) -> SimStatusResponse {
        SimStatusResponse {
            status: self.status,
            profile: self.profile,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_at_time_ramp_midpoint() {
        let segments = vec![Segment {
            duration_secs: 100.0,
            power_start: 100.0,
            power_end: 300.0,
            noise_amplitude: 0.0,
        }];
        let mut rng = Xorshift64::new(1);
        let power = power_at_time(&segments, 50.0, &mut rng);
        assert!(
            (power - 200.0).abs() < 0.1,
            "midpoint power should be 200W, got {}",
            power
        );
    }

    #[test]
    fn power_at_time_segment_boundary() {
        let segments = vec![
            Segment {
                duration_secs: 60.0,
                power_start: 100.0,
                power_end: 100.0,
                noise_amplitude: 0.0,
            },
            Segment {
                duration_secs: 60.0,
                power_start: 200.0,
                power_end: 200.0,
                noise_amplitude: 0.0,
            },
        ];
        let mut rng = Xorshift64::new(1);
        let before = power_at_time(&segments, 59.9, &mut rng);
        assert!(
            (before - 100.0).abs() < 0.1,
            "before boundary: expected ~100W, got {}",
            before
        );
        let after = power_at_time(&segments, 60.1, &mut rng);
        assert!(
            (after - 200.0).abs() < 0.1,
            "after boundary: expected ~200W, got {}",
            after
        );
    }

    #[test]
    fn hr_converges_to_steady_state() {
        let power = 200.0;
        let hr_target = 60.0 + 0.4 * power; // 140 bpm
        let mut hr = 60.0;
        // 5 * tau_rise (30s) = 150s gives residual < 1 bpm
        for _ in 0..150 {
            hr = hr_update(hr, power, 1.0);
        }
        assert!(
            (hr - hr_target).abs() < 1.0,
            "HR should converge to {}, got {}",
            hr_target,
            hr
        );
    }

    #[test]
    fn hr_rise_faster_than_fall() {
        let mut hr_rise = 60.0;
        for _ in 0..60 {
            hr_rise = hr_update(hr_rise, 200.0, 1.0);
        }
        let rise_delta = hr_rise - 60.0;

        let mut hr_fall = 140.0;
        for _ in 0..60 {
            hr_fall = hr_update(hr_fall, 0.0, 1.0);
        }
        let fall_delta = 140.0 - hr_fall;

        assert!(
            rise_delta > fall_delta,
            "rise ({:.1} bpm) should exceed fall ({:.1} bpm) in same interval",
            rise_delta,
            fall_delta
        );
    }

    #[test]
    fn cadence_in_reasonable_range() {
        for watts in [0.0, 50.0, 150.0, 200.0, 300.0, 500.0] {
            let rpm = cadence_from_power(watts);
            assert!(
                (70.0..=110.0).contains(&rpm),
                "cadence at {}W = {} rpm, outside 70-110 range",
                watts,
                rpm
            );
        }
    }

    #[test]
    fn speed_increases_with_power() {
        let speeds: Vec<f32> = [100.0, 150.0, 200.0, 250.0, 300.0]
            .iter()
            .map(|&w| speed_from_power(w))
            .collect();
        for pair in speeds.windows(2) {
            assert!(
                pair[1] > pair[0],
                "speed should increase with power: {} > {}",
                pair[1],
                pair[0]
            );
        }
    }
}
