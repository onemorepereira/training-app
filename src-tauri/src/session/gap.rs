//! The single gap rule shared by live metrics, analysis and FIT export.
//!
//! Two consecutive samples of one channel more than `MAX_READING_GAP_MS` apart
//! are separated by a *gap* (pause, sensor dropout, reconnect). Across a gap:
//! - integrals (work, distance, zone time) count at most `MAX_READING_GAP_MS`;
//! - 1 Hz curves/series have no data (values are never carried across it).
//!
//! Shorter holes (≤ 5 s) are treated as continuous: 1 Hz consumers hold the
//! previous second's value.

/// Maximum time between two readings that still counts as continuous riding.
/// Also caps active-duration accumulation in `SessionManager`.
pub const MAX_READING_GAP_SECS: u64 = 5;
pub const MAX_READING_GAP_MS: u64 = MAX_READING_GAP_SECS * 1000;

/// True when two consecutive samples (ms timestamps) are separated by a gap.
pub fn is_gap(prev_ms: u64, next_ms: u64) -> bool {
    next_ms.saturating_sub(prev_ms) > MAX_READING_GAP_MS
}

/// Duration an interval between two consecutive samples contributes to an
/// integral: the real interval, capped at `MAX_READING_GAP_MS`. Backward
/// timestamps contribute 0.
pub fn capped_interval_ms(prev_ms: u64, next_ms: u64) -> u64 {
    next_ms.saturating_sub(prev_ms).min(MAX_READING_GAP_MS)
}

/// A run of contiguous epoch-seconds with no gap inside.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Epoch-second (ms / 1000) of `values[0]`.
    pub start_sec: u64,
    /// One value per second: mean of that second's samples, or the previous
    /// second's value when the second falls inside a hole ≤ 5 s.
    pub values: Vec<f64>,
}

/// Resample `(epoch_ms, value)` samples of one channel to 1 Hz, split at gaps.
/// Input need not be sorted. Memory is bounded by the data, not the wall-clock span.
pub fn resample_1hz_segments(samples: &[(u64, f64)]) -> Vec<Segment> {
    let mut sorted: Vec<(u64, f64)> = samples.to_vec();
    sorted.sort_by_key(|(ms, _)| *ms);

    let mut segments: Vec<Segment> = Vec::new();
    // Current second being accumulated: (sec, sum, count, last_ms)
    let mut cur: Option<(u64, f64, u32, u64)> = None;

    let flush = |segments: &mut Vec<Segment>, sec: u64, mean: f64| {
        let seg = segments.last_mut().expect("segment opened before flush");
        let next_sec = seg.start_sec + seg.values.len() as u64;
        // Hole ≤ 5 s inside a segment: hold the previous second's value.
        if let Some(&held) = seg.values.last() {
            for _ in next_sec..sec {
                seg.values.push(held);
            }
        }
        seg.values.push(mean);
    };

    for &(ms, v) in &sorted {
        let sec = ms / 1000;
        match cur {
            Some((s, sum, n, last_ms)) if s == sec => {
                cur = Some((s, sum + v, n + 1, last_ms.max(ms)));
            }
            Some((s, sum, n, last_ms)) => {
                flush(&mut segments, s, sum / n as f64);
                if is_gap(last_ms, ms) {
                    segments.push(Segment { start_sec: sec, values: Vec::new() });
                }
                cur = Some((sec, v, 1, ms));
            }
            None => {
                segments.push(Segment { start_sec: sec, values: Vec::new() });
                cur = Some((sec, v, 1, ms));
            }
        }
    }
    if let Some((s, sum, n, _)) = cur {
        flush(&mut segments, s, sum / n as f64);
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_gap_boundary_5000ms_is_not_gap() {
        assert!(!is_gap(0, 5000), "exactly 5 s apart is continuous");
        assert!(is_gap(0, 5001), "just over 5 s is a gap");
    }

    #[test]
    fn is_gap_backward_timestamp_is_not_gap() {
        assert!(!is_gap(10_000, 1_000));
    }

    #[test]
    fn capped_interval_returns_real_interval_below_cap() {
        assert_eq!(capped_interval_ms(1000, 3000), 2000);
    }

    #[test]
    fn capped_interval_caps_at_5000ms() {
        // 10-minute pause contributes only 5 s
        assert_eq!(capped_interval_ms(0, 600_000), 5000);
    }

    #[test]
    fn capped_interval_backward_timestamp_contributes_zero() {
        assert_eq!(capped_interval_ms(5000, 1000), 0);
    }

    #[test]
    fn resample_empty_returns_no_segments() {
        assert!(resample_1hz_segments(&[]).is_empty());
    }

    #[test]
    fn resample_averages_samples_within_a_second() {
        // Second 0: (100 + 300) / 2 = 200; second 1: 200
        let segs = resample_1hz_segments(&[(0, 100.0), (500, 300.0), (1000, 200.0)]);
        assert_eq!(segs, vec![Segment { start_sec: 0, values: vec![200.0, 200.0] }]);
    }

    #[test]
    fn resample_holds_value_across_short_hole() {
        // 4 s apart (≤ 5 s): seconds 1..3 hold 100
        let segs = resample_1hz_segments(&[(0, 100.0), (4000, 300.0)]);
        assert_eq!(
            segs,
            vec![Segment { start_sec: 0, values: vec![100.0, 100.0, 100.0, 100.0, 300.0] }]
        );
    }

    #[test]
    fn resample_splits_segments_at_gap() {
        // 1000 → 7000 ms = 6 s apart → gap: no held values in between
        let segs = resample_1hz_segments(&[(0, 100.0), (1000, 100.0), (7000, 300.0)]);
        assert_eq!(
            segs,
            vec![
                Segment { start_sec: 0, values: vec![100.0, 100.0] },
                Segment { start_sec: 7, values: vec![300.0] },
            ]
        );
    }

    #[test]
    fn resample_gap_measured_from_last_sample_of_previous_second() {
        // Last sample of second 0 at 900 ms; next at 5900 ms = exactly 5 s → no gap
        let segs = resample_1hz_segments(&[(0, 100.0), (900, 100.0), (5900, 200.0)]);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].values, vec![100.0, 100.0, 100.0, 100.0, 100.0, 200.0]);
    }

    #[test]
    fn resample_sorts_unordered_input() {
        let segs = resample_1hz_segments(&[(1000, 200.0), (0, 100.0)]);
        assert_eq!(segs, vec![Segment { start_sec: 0, values: vec![100.0, 200.0] }]);
    }
}
