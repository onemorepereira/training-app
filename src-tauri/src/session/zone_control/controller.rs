use std::sync::Arc;
use std::time::Instant;

use log::{debug, info, warn};
use tokio::sync::{broadcast, watch, Mutex};
use tokio::task::JoinHandle;

use crate::device::manager::DeviceManager;
use crate::device::types::{CommandSource, SensorReading};
use crate::error::AppError;

use super::pid::{adaptive_gains, HrSmoother, PidController};
use super::types::{StopReason, ZoneControlStatus, ZoneMode, ZoneTarget};

/// Maximum watts per tick when ramping UP (rate limiter, separate from PID output_limit)
const HR_MAX_WATTS_UP_PER_TICK: f64 = 10.0;
/// Maximum watts per tick when ramping DOWN (faster — reducing power is always safe)
const HR_MAX_WATTS_DOWN_PER_TICK: f64 = 30.0;
/// Integral decay factor when HR is above zone but already falling
const INTEGRAL_DECAY_ON_FALLING_HR: f64 = 0.7;
/// Minimum commanded power (watts)
const MIN_POWER: u16 = 50;
/// Safety: reduce to this power when HR ceiling exceeded
const SAFETY_POWER: u16 = 50;
/// HR sensor lost thresholds (seconds)
const HR_SENSOR_WARN_SECS: u64 = 15;
const HR_SENSOR_STOP_SECS: u64 = 30;
/// Power sensor lost threshold (seconds)
const POWER_SENSOR_WARN_SECS: u64 = 15;
/// Cadence zero threshold (seconds)
const CADENCE_ZERO_SECS: u64 = 3;

struct ControlLoopState {
    active: bool,
    target: Option<ZoneTarget>,
    paused: bool,
    commanded_power: u16,
    time_in_zone_ms: u64,
    started_at: Option<Instant>,
    paused_accumulated_ms: u64,
    pause_started: Option<Instant>,
    phase: String,
    safety_note: Option<String>,
    stop_reason: Option<StopReason>,
    last_power: Option<u16>,
    last_hr: Option<u8>,
    last_cadence: Option<f32>,
    last_cadence_zero_since: Option<Instant>,
    last_hr_seen: Option<Instant>,
    last_power_seen: Option<Instant>,
    /// FTP from user config, used for HR mode power clamping
    ftp: Option<u16>,
    /// Max HR from user config, used for HR ceiling safety
    max_hr: Option<u8>,
    /// Instant of the last processed tick, for measuring actual elapsed time
    last_tick_at: Option<Instant>,
    /// Whether HR was above zone on previous tick (for integral reset on re-entry)
    was_above_zone: bool,
    /// Power zone percentages from user config (for HR mode power banding)
    power_zones: Option<[u16; 6]>,
}

impl ControlLoopState {
    fn new() -> Self {
        Self {
            active: false,
            target: None,
            paused: false,
            commanded_power: 0,
            time_in_zone_ms: 0,
            started_at: None,
            paused_accumulated_ms: 0,
            pause_started: None,
            phase: "idle".to_string(),
            safety_note: None,
            stop_reason: None,
            last_power: None,
            last_hr: None,
            last_cadence: None,
            last_cadence_zero_since: None,
            last_hr_seen: None,
            last_power_seen: None,
            ftp: None,
            max_hr: None,
            last_tick_at: None,
            was_above_zone: false,
            power_zones: None,
        }
    }

    fn elapsed_ms(&self) -> u64 {
        let Some(started) = self.started_at else {
            return 0;
        };
        let total = started.elapsed().as_millis() as u64;
        let paused = self.paused_accumulated_ms
            + self
                .pause_started
                .map(|p| p.elapsed().as_millis() as u64)
                .unwrap_or(0);
        total.saturating_sub(paused)
    }
}

pub struct ZoneController {
    state: Arc<Mutex<ControlLoopState>>,
    shutdown_tx: Option<watch::Sender<bool>>,
    task_handle: Option<JoinHandle<()>>,
}

impl ZoneController {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(ControlLoopState::new())),
            shutdown_tx: None,
            task_handle: None,
        }
    }

    pub async fn start_with_config(
        &mut self,
        target: ZoneTarget,
        device_manager: Arc<Mutex<DeviceManager>>,
        sensor_tx: broadcast::Sender<SensorReading>,
        ftp: Option<u16>,
        max_hr: Option<u8>,
        initial_power_estimate: Option<u16>,
        power_zones: Option<[u16; 6]>,
    ) -> Result<(), AppError> {
        // Validate
        if target.lower_bound >= target.upper_bound {
            return Err(AppError::Session(
                "Zone lower bound must be less than upper bound".into(),
            ));
        }

        // Verify trainer connected
        {
            let dm = device_manager.lock().await;
            if dm.connected_trainer_id().is_none() {
                return Err(AppError::Session("No trainer connected".into()));
            }
        }

        // Stop any existing control loop
        self.stop_internal().await;

        let midpoint = (target.lower_bound + target.upper_bound) / 2;
        let initial_power = match target.mode {
            ZoneMode::Power => midpoint,
            ZoneMode::HeartRate => {
                if let Some(estimate) = initial_power_estimate {
                    // Historical model estimate, clamped to safe range
                    let max = ftp.map(|f| (f as f64 * 1.2) as u16).unwrap_or(300);
                    estimate.clamp(MIN_POWER, max)
                } else {
                    // Conservative start: 55% FTP if available, else 100W
                    ftp.map(|f| (f as f64 * 0.55) as u16).unwrap_or(100)
                }
            }
        };

        {
            let mut state = self.state.lock().await;
            state.active = true;
            state.target = Some(target.clone());
            state.paused = false;
            state.commanded_power = initial_power;
            state.time_in_zone_ms = 0;
            let now = Instant::now();
            state.started_at = Some(now);
            state.last_tick_at = Some(now);
            state.paused_accumulated_ms = 0;
            state.pause_started = None;
            state.phase = "ramping".to_string();
            state.safety_note = None;
            state.stop_reason = None;
            state.last_power = None;
            state.last_hr = None;
            state.last_cadence = None;
            state.last_cadence_zero_since = None;
            state.last_hr_seen = Some(Instant::now());
            state.last_power_seen = Some(Instant::now());
            state.ftp = ftp;
            state.max_hr = max_hr;
            state.was_above_zone = false;
            state.power_zones = power_zones;
        }

        // Command trainer to initial power
        {
            let mut dm = device_manager.lock().await;
            if let Some(trainer_id) = dm.connected_trainer_id() {
                if let Err(e) = dm.set_target_power(&trainer_id, initial_power as i16).await {
                    warn!("Initial trainer power command failed: {}", e);
                }
            }
        }

        // Log initial command
        let _ = sensor_tx.send(SensorReading::TrainerCommand {
            target_watts: initial_power,
            epoch_ms: now_epoch_ms(),
            source: CommandSource::ZoneControl,
        });

        info!(
            "Zone control started: {:?} zone {} ({}-{} {}), initial {}W",
            target.mode,
            target.zone,
            target.lower_bound,
            target.upper_bound,
            if target.mode == ZoneMode::Power {
                "W"
            } else {
                "bpm"
            },
            initial_power
        );

        // Spawn control loop
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        let state = self.state.clone();
        let sensor_rx = sensor_tx.subscribe();

        let handle = tokio::spawn(control_loop(
            state,
            target,
            device_manager,
            sensor_tx,
            sensor_rx,
            shutdown_rx,
        ));
        self.task_handle = Some(handle);

        Ok(())
    }

    pub async fn stop(&mut self) -> Option<StopReason> {
        self.stop_internal().await;
        let mut state = self.state.lock().await;
        let reason = state
            .stop_reason
            .take()
            .unwrap_or(StopReason::UserStopped);
        state.active = false;
        state.phase = "idle".to_string();
        info!("Zone control stopped: {:?}", reason);
        Some(reason)
    }

    async fn stop_internal(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }
    }

    pub async fn pause(&self) {
        let mut state = self.state.lock().await;
        if state.active && !state.paused {
            state.paused = true;
            state.pause_started = Some(Instant::now());
            info!("Zone control paused");
        }
    }

    pub async fn resume(&self) {
        let mut state = self.state.lock().await;
        if state.active && state.paused {
            if let Some(pause_start) = state.pause_started.take() {
                state.paused_accumulated_ms += pause_start.elapsed().as_millis() as u64;
            }
            state.paused = false;
            info!("Zone control resumed");
        }
    }

    pub async fn status(&self) -> ZoneControlStatus {
        let state = self.state.lock().await;
        ZoneControlStatus {
            active: state.active,
            mode: state.target.as_ref().map(|t| t.mode),
            target_zone: state.target.as_ref().map(|t| t.zone),
            lower_bound: state.target.as_ref().map(|t| t.lower_bound),
            upper_bound: state.target.as_ref().map(|t| t.upper_bound),
            commanded_power: if state.active {
                Some(state.commanded_power)
            } else {
                None
            },
            time_in_zone_secs: state.time_in_zone_ms / 1000,
            elapsed_secs: state.elapsed_ms() / 1000,
            duration_secs: state.target.as_ref().and_then(|t| t.duration_secs),
            paused: state.paused,
            phase: state.phase.clone(),
            safety_note: state.safety_note.clone(),
        }
    }
}

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

async fn command_trainer(
    device_manager: &Arc<Mutex<DeviceManager>>,
    watts: u16,
    sensor_tx: &broadcast::Sender<SensorReading>,
) -> Result<(), AppError> {
    let mut dm = device_manager.lock().await;
    let trainer_id = dm
        .connected_trainer_id()
        .ok_or_else(|| AppError::Session("Trainer disconnected".into()))?;
    dm.set_target_power(&trainer_id, watts as i16).await?;
    drop(dm);

    let _ = sensor_tx.send(SensorReading::TrainerCommand {
        target_watts: watts,
        epoch_ms: now_epoch_ms(),
        source: CommandSource::ZoneControl,
    });
    Ok(())
}

async fn control_loop(
    state: Arc<Mutex<ControlLoopState>>,
    target: ZoneTarget,
    device_manager: Arc<Mutex<DeviceManager>>,
    sensor_tx: broadcast::Sender<SensorReading>,
    mut sensor_rx: broadcast::Receiver<SensorReading>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let tick_interval = match target.mode {
        ZoneMode::Power => tokio::time::Duration::from_secs(1),
        ZoneMode::HeartRate => tokio::time::Duration::from_secs(5),
    };
    let mut tick = tokio::time::interval(tick_interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Consume the immediate first tick — tokio::time::interval fires instantly
    // on first call, but we need sensor data to arrive before processing.
    tick.tick().await;

    // HR mode PID and smoother (only used for HeartRate mode)
    let mut pid = PidController::new(2.0, 0.1, 0.5);
    let mut hr_smoother = HrSmoother::new(5);

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                break;
            }
            result = sensor_rx.recv() => {
                match result {
                    Ok(reading) => {
                        let mut s = state.lock().await;
                        match &reading {
                            SensorReading::Power { watts, .. } => {
                                s.last_power = Some(*watts);
                                s.last_power_seen = Some(Instant::now());
                            }
                            SensorReading::HeartRate { bpm, .. } => {
                                s.last_hr = Some(*bpm);
                                s.last_hr_seen = Some(Instant::now());
                                hr_smoother.push(*bpm);
                            }
                            SensorReading::Cadence { rpm, .. } => {
                                let now = Instant::now();
                                if *rpm < 1.0 {
                                    if s.last_cadence_zero_since.is_none() {
                                        s.last_cadence_zero_since = Some(now);
                                    }
                                } else {
                                    s.last_cadence_zero_since = None;
                                }
                                s.last_cadence = Some(*rpm);
                            }
                            _ => {}
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = tick.tick() => {
                let should_stop = process_tick(
                    &state,
                    &target,
                    &device_manager,
                    &sensor_tx,
                    &mut pid,
                    &hr_smoother,
                ).await;
                if should_stop {
                    break;
                }
            }
        }
    }
}

async fn process_tick(
    state: &Arc<Mutex<ControlLoopState>>,
    target: &ZoneTarget,
    device_manager: &Arc<Mutex<DeviceManager>>,
    sensor_tx: &broadcast::Sender<SensorReading>,
    pid: &mut PidController,
    hr_smoother: &HrSmoother,
) -> bool {
    let mut s = state.lock().await;

    if !s.active || s.paused {
        return false;
    }

    // Measure actual elapsed time since last tick
    let now = Instant::now();
    let tick_ms = s
        .last_tick_at
        .map(|t| now.duration_since(t).as_millis() as u64)
        .unwrap_or(0);
    s.last_tick_at = Some(now);

    // === Safety: cadence zero for >CADENCE_ZERO_SECS → command 0W ===
    if let Some(zero_since) = s.last_cadence_zero_since {
        if zero_since.elapsed().as_secs() >= CADENCE_ZERO_SECS {
            if s.commanded_power != 0 {
                warn!("Cadence zero for >{}s — reducing power to 0W", CADENCE_ZERO_SECS);
                s.commanded_power = 0;
                s.safety_note = Some("Cadence zero — power reduced".to_string());
                drop(s);
                if command_trainer(device_manager, 0, sensor_tx).await.is_err() {
                    warn!("Trainer disconnected during cadence-zero safety command");
                    let mut s = state.lock().await;
                    s.stop_reason = Some(StopReason::TrainerDisconnected);
                    s.active = false;
                    return true;
                }
                return false;
            }
            return false;
        }
    }

    // === Safety: HR ceiling (HR mode) ===
    if target.mode == ZoneMode::HeartRate {
        if let Some(max_hr) = s.max_hr {
            if let Some(hr) = s.last_hr {
                if hr > max_hr {
                    warn!(
                        "HR ceiling exceeded: {} bpm > {} max — reducing to {}W",
                        hr, max_hr, SAFETY_POWER
                    );
                    s.commanded_power = SAFETY_POWER;
                    s.safety_note = Some("HR ceiling exceeded".to_string());
                    s.phase = "adjusting".to_string();
                    drop(s);
                    if command_trainer(device_manager, SAFETY_POWER, sensor_tx)
                        .await
                        .is_err()
                    {
                        warn!("Trainer disconnected during HR ceiling safety command");
                        let mut s = state.lock().await;
                        s.stop_reason = Some(StopReason::TrainerDisconnected);
                        s.active = false;
                        return true;
                    }
                    return false;
                }
            }
        }

        // === Safety: HR sensor lost (HR mode) ===
        let hr_lost_secs = s
            .last_hr_seen
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(u64::MAX);
        if hr_lost_secs >= HR_SENSOR_STOP_SECS {
            warn!("HR sensor lost for {}s — stopping zone control", hr_lost_secs);
            s.stop_reason = Some(StopReason::SensorLost);
            s.safety_note = Some("HR sensor lost".to_string());
            s.active = false;
            return true;
        } else if hr_lost_secs >= HR_SENSOR_WARN_SECS {
            warn!("HR sensor not responding for {}s — holding power", hr_lost_secs);
            s.safety_note = Some("HR sensor not responding — holding power".to_string());
            // Hold current power, don't adjust
            return false;
        }
    }

    // === Safety: Power sensor lost (power mode) ===
    if target.mode == ZoneMode::Power {
        let power_lost_secs = s
            .last_power_seen
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(u64::MAX);
        if power_lost_secs >= POWER_SENSOR_WARN_SECS {
            warn!("Power sensor not responding for {}s", power_lost_secs);
            s.safety_note = Some("Power sensor not responding".to_string());
            // Continue — trainer ERG still works
        }
    }

    // === Check duration expiry ===
    if let Some(duration) = target.duration_secs {
        if s.elapsed_ms() / 1000 >= duration {
            s.stop_reason = Some(StopReason::DurationComplete);
            s.active = false;
            info!("Zone control: duration complete");
            return true;
        }
    }

    // === Mode-specific tick ===
    match target.mode {
        ZoneMode::Power => {
            process_power_tick(&mut s, target, tick_ms);
        }
        ZoneMode::HeartRate => {
            let new_power = process_hr_tick(&mut s, target, pid, hr_smoother, tick_ms);
            if let Some(watts) = new_power {
                s.commanded_power = watts;
                drop(s);
                if command_trainer(device_manager, watts, sensor_tx)
                    .await
                    .is_err()
                {
                    warn!("Trainer disconnected during HR mode power command");
                    let mut s = state.lock().await;
                    s.stop_reason = Some(StopReason::TrainerDisconnected);
                    s.active = false;
                    return true;
                }
            }
        }
    }

    false
}

fn process_power_tick(s: &mut ControlLoopState, target: &ZoneTarget, tick_ms: u64) {
    if let Some(power) = s.last_power {
        let in_zone = power >= target.lower_bound && power <= target.upper_bound;
        if in_zone {
            s.time_in_zone_ms += tick_ms;
            s.phase = "in_zone".to_string();
            s.safety_note = None;
        } else {
            s.phase = "adjusting".to_string();
        }
    } else {
        s.phase = "ramping".to_string();
    }
}

/// HR mode tick: uses PID controller with adaptive gains to adjust power.
/// Returns Some(new_watts) if power should be changed, None to hold.
fn process_hr_tick(
    s: &mut ControlLoopState,
    target: &ZoneTarget,
    pid: &mut PidController,
    hr_smoother: &HrSmoother,
    tick_ms: u64,
) -> Option<u16> {
    let smoothed_hr = hr_smoother.smoothed()?;
    let target_hr = ((target.lower_bound + target.upper_bound) / 2) as f64;
    let error = target_hr - smoothed_hr as f64;

    // Integral reset on zone re-entry: clear integral when HR drops from above-zone back into zone
    let above_zone = (smoothed_hr as u16) > target.upper_bound;
    if s.was_above_zone && !above_zone {
        pid.reset_integral();
    }
    s.was_above_zone = above_zone;

    // Track time in zone
    let prev_phase = s.phase.clone();
    let in_zone =
        smoothed_hr as u16 >= target.lower_bound && smoothed_hr as u16 <= target.upper_bound;
    if in_zone {
        s.time_in_zone_ms += tick_ms;
        s.phase = "in_zone".to_string();
        s.safety_note = None;
    } else {
        s.phase = "adjusting".to_string();
    }

    if s.phase != prev_phase {
        debug!("Phase transition: {} -> {}", prev_phase, s.phase);
    }

    // Adaptive gains based on distance from target
    let (kp, ki, kd) = adaptive_gains(error.abs());
    pid.set_gains(kp, ki, kd);

    let dt_secs = tick_ms as f64 / 1000.0;
    let watts_adjustment = pid.update(error, dt_secs);

    // Derive power band from HR zone number and power zone config
    let (power_floor, power_ceiling) = match (s.ftp, s.power_zones) {
        (Some(ftp), Some(pz)) => {
            let zone = target.zone;
            // Floor: one power zone below (zone-2 index, or MIN_POWER for zone 1)
            let floor = if zone >= 2 {
                (ftp as f64 * pz[(zone - 2) as usize] as f64 / 100.0) as u16
            } else {
                MIN_POWER
            };
            // Ceiling: one power zone above (zone index, capped at array length)
            let ceil_idx = (zone as usize).min(5);
            let ceiling = (ftp as f64 * pz[ceil_idx] as f64 / 100.0) as u16;
            (floor.max(MIN_POWER), ceiling)
        }
        _ => (MIN_POWER, s.ftp.map(|f| (f as f64 * 1.5) as u16).unwrap_or(400)),
    };

    // Rate limit: asymmetric — ramp down faster than up, with faster recovery when below band
    let band_midpoint = (power_floor + power_ceiling) / 2;
    let max_up = if error > 0.0 && s.commanded_power < band_midpoint.saturating_sub(20) {
        HR_MAX_WATTS_UP_PER_TICK * 2.0 // 20W/tick during recovery
    } else {
        HR_MAX_WATTS_UP_PER_TICK // 10W/tick normal
    };
    let clamped_adjustment = if watts_adjustment < 0.0 {
        watts_adjustment.max(-HR_MAX_WATTS_DOWN_PER_TICK)
    } else {
        watts_adjustment.min(max_up)
    };

    // Decay integral when HR is above zone but already falling
    if error < 0.0 {
        if let Some(prev_hr) = s.last_hr {
            if (prev_hr as f64) > smoothed_hr as f64 {
                pid.decay_integral(INTEGRAL_DECAY_ON_FALLING_HR);
            }
        }
    }

    let new_power_f = s.commanded_power as f64 + clamped_adjustment;

    // Clamp to power band [power_floor, power_ceiling]
    let new_power = (new_power_f as u16).clamp(power_floor, power_ceiling);

    if new_power != s.commanded_power {
        debug!(
            "HR PID: smoothed_hr={}, error={:.1}, adjustment={:.1}W, power {}W -> {}W",
            smoothed_hr, error, clamped_adjustment, s.commanded_power, new_power
        );
        Some(new_power)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::pid::{HrSmoother, PidController};
    use super::super::types::{ZoneMode, ZoneTarget};

    fn assert_approx(actual: f64, expected: f64, epsilon: f64, msg: &str) {
        assert!(
            (actual - expected).abs() <= epsilon,
            "{msg}: expected {expected} ± {epsilon}, got {actual}"
        );
    }

    /// Build a ControlLoopState with commanded_power and last_hr pre-set.
    fn make_state(commanded_power: u16, last_hr: Option<u8>) -> ControlLoopState {
        let mut s = ControlLoopState::new();
        s.active = true;
        s.commanded_power = commanded_power;
        s.last_hr = last_hr;
        s.ftp = Some(300);
        s
    }

    /// HR zone target: 130-140 bpm, midpoint 135.
    fn hr_target() -> ZoneTarget {
        ZoneTarget {
            mode: ZoneMode::HeartRate,
            zone: 3,
            lower_bound: 130,
            upper_bound: 140,
            duration_secs: None,
        }
    }

    /// Push `bpm` into smoother 5 times so smoothed() == bpm.
    fn fill_smoother(smoother: &mut HrSmoother, bpm: u8) {
        for _ in 0..5 {
            smoother.push(bpm);
        }
    }

    #[test]
    fn ramp_down_faster_than_ramp_up() {
        // HR=155 (20 above midpoint 135) → error=-20, PID wants large negative.
        // Down limit is 30W, so single tick should drop > 10W.
        let target = hr_target();
        let mut pid = PidController::new(2.0, 0.1, 0.5);
        let mut smoother = HrSmoother::new(5);
        fill_smoother(&mut smoother, 155);
        let mut s = make_state(200, None);

        let new = process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000);
        let drop = 200_i32 - new.unwrap() as i32;
        assert!(drop > 10, "ramp-down should exceed old 10W limit, got {drop}W drop");
    }

    #[test]
    fn ramp_up_still_limited_to_10w() {
        // HR=120 (15 below midpoint 135) → error=+15, PID wants large positive.
        // At 250W (near fallback midpoint 250), normal mode: up limit is 10W/tick.
        let target = hr_target();
        let mut pid = PidController::new(2.0, 0.1, 0.5);
        let mut smoother = HrSmoother::new(5);
        fill_smoother(&mut smoother, 120);
        let mut s = make_state(250, None);

        let new = process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000);
        let gain = new.unwrap() as i32 - 250;
        assert!(gain <= 10, "ramp-up should stay <= 10W, got {gain}W gain");
    }

    #[test]
    fn integral_decays_when_hr_above_zone_and_falling() {
        // HR above zone (error < 0), last_hr higher than smoothed → decay.
        let target = hr_target();
        let mut pid = PidController::new(2.0, 0.1, 0.5);
        let mut smoother = HrSmoother::new(5);

        // First tick: HR=150, build some negative integral
        fill_smoother(&mut smoother, 150);
        let mut s = make_state(200, None);
        process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000);
        let integral_after_first = pid.integral();

        // Second tick: HR=148 (falling), last_hr=150 (from raw reading)
        fill_smoother(&mut smoother, 148);
        s.last_hr = Some(150);
        process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000);
        let integral_after_second = pid.integral();

        // Integral should be less negative than just accumulating (decay pulled it toward zero)
        // Without decay: integral would be integral_after_first + (-13 * 5) = more negative
        // With decay: integral_after_first * 0.7 + (-13 * 5) = less negative
        assert!(
            integral_after_second.abs() < (integral_after_first + (-13.0 * 5.0)).abs(),
            "integral should decay when HR above zone and falling: first={integral_after_first}, second={integral_after_second}"
        );
    }

    #[test]
    fn no_integral_decay_when_hr_below_zone() {
        // HR=120 → error=+15 (positive), no decay should happen.
        let target = hr_target();
        let mut pid = PidController::new(2.0, 0.1, 0.5);
        let mut smoother = HrSmoother::new(5);

        // First tick to establish integral
        fill_smoother(&mut smoother, 120);
        let mut s = make_state(100, None);
        process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000);
        let integral_after_first = pid.integral();

        // Second tick: HR still below zone, last_hr=125 (falling but error > 0)
        fill_smoother(&mut smoother, 120);
        s.last_hr = Some(125);
        process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000);
        let integral_after_second = pid.integral();

        // Integral should keep growing (no decay), approximately first + 15*5
        let expected_no_decay = integral_after_first + 15.0 * 5.0;
        assert_approx(
            integral_after_second,
            expected_no_decay,
            0.5,
            "no decay when HR below zone",
        );
    }

    #[test]
    fn no_integral_decay_when_hr_above_zone_but_rising() {
        // HR above zone (error < 0), but HR is rising (last_hr < smoothed) → no decay.
        let target = hr_target();
        let mut pid = PidController::new(2.0, 0.1, 0.5);
        let mut smoother = HrSmoother::new(5);

        // First tick at HR=150
        fill_smoother(&mut smoother, 150);
        let mut s = make_state(200, None);
        process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000);
        let integral_after_first = pid.integral();

        // Second tick: HR=152 (rising), last_hr=148 (lower than smoothed)
        fill_smoother(&mut smoother, 152);
        s.last_hr = Some(148);
        process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000);
        let integral_after_second = pid.integral();

        // Integral should just accumulate without decay: first + (-17 * 5)
        let expected_no_decay = integral_after_first + (-17.0 * 5.0);
        assert_approx(
            integral_after_second,
            expected_no_decay,
            0.5,
            "no decay when HR rising",
        );
    }

    #[test]
    fn ramp_down_multi_tick_reaches_target_faster() {
        // Simulate 6 ticks at HR=155, starting at 200W.
        // With 30W/tick down limit, power should reach well below 140W.
        // Old symmetric 10W/tick would only get to 200 - 60 = 140W floor.
        let target = hr_target();
        let mut pid = PidController::new(2.0, 0.1, 0.5);
        let mut smoother = HrSmoother::new(5);
        fill_smoother(&mut smoother, 155);
        let mut s = make_state(200, None);

        for _ in 0..6 {
            if let Some(new) = process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000) {
                s.commanded_power = new;
            }
        }

        assert!(
            s.commanded_power < 140,
            "after 6 ticks, power should be < 140W (was {}W)",
            s.commanded_power
        );
    }

    // --- HR zone 2 helpers for power band / integral reset tests ---

    /// HR zone 2 target: 139-151 bpm (matching plan context).
    fn hr_zone2_target() -> ZoneTarget {
        ZoneTarget {
            mode: ZoneMode::HeartRate,
            zone: 2,
            lower_bound: 139,
            upper_bound: 151,
            duration_secs: None,
        }
    }

    /// Build a state configured for HR zone 2 with FTP=200 and standard power zones.
    /// Power zones [55,75,90,105,120,150]% → Z1≤110, Z2≤150, Z3≤180, Z4≤210, Z5≤240, Z6≤300.
    /// For HR zone 2: floor=200×55%=110W (pz[0]), ceiling=200×90%=180W (pz[2]).
    fn make_zone2_state(commanded_power: u16, last_hr: Option<u8>) -> ControlLoopState {
        let mut s = ControlLoopState::new();
        s.active = true;
        s.commanded_power = commanded_power;
        s.last_hr = last_hr;
        s.ftp = Some(200);
        s.power_zones = Some([55, 75, 90, 105, 120, 150]);
        s
    }

    #[test]
    fn integral_resets_on_above_to_in_zone_transition() {
        // HR 155→145: above zone then back in zone → integral should reset to 0.
        let target = hr_zone2_target();
        let mut pid = PidController::new(2.0, 0.1, 0.5);
        let mut smoother = HrSmoother::new(5);

        // First tick: HR=155 (above zone, upper_bound=151)
        fill_smoother(&mut smoother, 155);
        let mut s = make_zone2_state(150, None);
        process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000);
        // Integral should be non-zero (negative from error < 0)
        assert!(pid.integral() != 0.0, "integral should be non-zero after first tick");

        // Second tick: HR=145 (in zone, not above). Transition from above→in triggers reset.
        fill_smoother(&mut smoother, 145);
        process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000);
        assert_approx(pid.integral(), 0.0, 0.5, "integral should reset on above→in-zone transition");
    }

    #[test]
    fn integral_not_reset_on_below_to_in_zone_transition() {
        // HR 130→145: below zone then into zone → integral should NOT reset.
        let target = hr_zone2_target();
        let mut pid = PidController::new(2.0, 0.1, 0.5);
        let mut smoother = HrSmoother::new(5);

        // First tick: HR=130 (below zone, lower_bound=139)
        fill_smoother(&mut smoother, 130);
        let mut s = make_zone2_state(150, None);
        process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000);
        let integral_after_first = pid.integral();
        assert!(integral_after_first > 0.0, "integral should be positive when HR below target");

        // Second tick: HR=145 (in zone). Coming from below, not above → no reset.
        fill_smoother(&mut smoother, 145);
        process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000);
        // Integral should have continued accumulating (not reset to 0)
        assert!(pid.integral() != 0.0, "integral should not reset on below→in-zone transition");
    }

    #[test]
    fn power_band_clamps_floor() {
        // FTP=200, zone 2, power_zones=[55,75,90,105,120,150]
        // Floor = 200 * 55% = 110W. PID wants to push below → clamped at 110.
        let target = hr_zone2_target();
        let mut pid = PidController::new(2.0, 0.1, 0.5);
        let mut smoother = HrSmoother::new(5);

        // HR=160 (well above zone) → large negative error → PID wants to decrease power
        fill_smoother(&mut smoother, 160);
        let mut s = make_zone2_state(115, None);

        // Run several ticks to push power down
        for _ in 0..10 {
            if let Some(new) = process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000) {
                s.commanded_power = new;
            }
        }

        assert!(
            s.commanded_power >= 110,
            "power should not go below floor of 110W (was {}W)",
            s.commanded_power
        );
    }

    #[test]
    fn power_band_clamps_ceiling() {
        // FTP=200, zone 2, power_zones=[55,75,90,105,120,150]
        // Ceiling = 200 * 90% = 180W. PID wants to push above → clamped at 180.
        let target = hr_zone2_target();
        let mut pid = PidController::new(2.0, 0.1, 0.5);
        let mut smoother = HrSmoother::new(5);

        // HR=120 (well below zone) → large positive error → PID wants to increase power
        fill_smoother(&mut smoother, 120);
        let mut s = make_zone2_state(175, None);

        // Run several ticks to push power up
        for _ in 0..10 {
            if let Some(new) = process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000) {
                s.commanded_power = new;
            }
        }

        assert!(
            s.commanded_power <= 180,
            "power should not go above ceiling of 180W (was {}W)",
            s.commanded_power
        );
    }

    #[test]
    fn faster_ramp_up_when_below_band_midpoint() {
        // FTP=200, zone 2: floor=110, ceiling=180, midpoint=145.
        // At 110W with error>0 (HR below target), 110 < 145-20=125 → recovery mode (20W/tick).
        let target = hr_zone2_target();
        let mut pid = PidController::new(2.0, 0.1, 0.5);
        let mut smoother = HrSmoother::new(5);

        // HR=130 (below zone) → positive error
        fill_smoother(&mut smoother, 130);
        let mut s = make_zone2_state(110, None);

        let new = process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000);
        let gain = new.unwrap() as i32 - 110;
        assert!(
            gain > 10,
            "recovery ramp-up should exceed 10W/tick, got {gain}W gain"
        );
    }

    #[test]
    fn normal_ramp_up_when_near_band_midpoint() {
        // FTP=200, zone 2: floor=110, ceiling=180, midpoint=145.
        // At 140W with error>0, 140 >= 125 → normal mode (10W/tick).
        let target = hr_zone2_target();
        let mut pid = PidController::new(2.0, 0.1, 0.5);
        let mut smoother = HrSmoother::new(5);

        // HR=130 (below zone) → positive error
        fill_smoother(&mut smoother, 130);
        let mut s = make_zone2_state(140, None);

        let new = process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000);
        let gain = new.unwrap() as i32 - 140;
        assert!(
            gain <= 10,
            "normal ramp-up should stay <= 10W/tick, got {gain}W gain"
        );
    }
}
