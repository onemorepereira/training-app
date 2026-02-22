use std::sync::Arc;
use std::time::Instant;

use log::info;
use tokio::sync::{broadcast, watch, Mutex};
use tokio::task::JoinHandle;

use crate::device::manager::DeviceManager;
use crate::device::types::{CommandSource, SensorReading};
use crate::error::AppError;

use super::pid::{adaptive_gains, HrSmoother, PidController};
use super::types::{StopReason, ZoneControlStatus, ZoneMode, ZoneTarget};

/// Maximum watts per tick adjustment (rate limiter, separate from PID output_limit)
const HR_MAX_WATTS_PER_TICK: f64 = 10.0;
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

    pub async fn start(
        &mut self,
        target: ZoneTarget,
        device_manager: Arc<Mutex<DeviceManager>>,
        sensor_tx: broadcast::Sender<SensorReading>,
    ) -> Result<(), AppError> {
        self.start_with_config(target, device_manager, sensor_tx, None, None, None)
            .await
    }

    pub async fn start_with_config(
        &mut self,
        target: ZoneTarget,
        device_manager: Arc<Mutex<DeviceManager>>,
        sensor_tx: broadcast::Sender<SensorReading>,
        ftp: Option<u16>,
        max_hr: Option<u8>,
        initial_power_estimate: Option<u16>,
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
            state.started_at = Some(Instant::now());
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
        }

        // Command trainer to initial power
        {
            let mut dm = device_manager.lock().await;
            if let Some(trainer_id) = dm.connected_trainer_id() {
                dm.set_target_power(&trainer_id, initial_power as i16)
                    .await
                    .ok();
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

    // === Safety: cadence zero for >CADENCE_ZERO_SECS → command 0W ===
    if let Some(zero_since) = s.last_cadence_zero_since {
        if zero_since.elapsed().as_secs() >= CADENCE_ZERO_SECS {
            if s.commanded_power != 0 {
                s.commanded_power = 0;
                s.safety_note = Some("Cadence zero — power reduced".to_string());
                drop(s);
                if command_trainer(device_manager, 0, sensor_tx).await.is_err() {
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
                    s.commanded_power = SAFETY_POWER;
                    s.safety_note = Some("HR ceiling exceeded".to_string());
                    s.phase = "adjusting".to_string();
                    drop(s);
                    if command_trainer(device_manager, SAFETY_POWER, sensor_tx)
                        .await
                        .is_err()
                    {
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
            s.stop_reason = Some(StopReason::SensorLost);
            s.safety_note = Some("HR sensor lost".to_string());
            s.active = false;
            return true;
        } else if hr_lost_secs >= HR_SENSOR_WARN_SECS {
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
            process_power_tick(&mut s, target);
        }
        ZoneMode::HeartRate => {
            let new_power = process_hr_tick(&mut s, target, pid, hr_smoother);
            if let Some(watts) = new_power {
                s.commanded_power = watts;
                drop(s);
                if command_trainer(device_manager, watts, sensor_tx)
                    .await
                    .is_err()
                {
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

fn process_power_tick(s: &mut ControlLoopState, target: &ZoneTarget) {
    if let Some(power) = s.last_power {
        let in_zone = power >= target.lower_bound && power <= target.upper_bound;
        if in_zone {
            s.time_in_zone_ms += 1000; // 1s tick
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
) -> Option<u16> {
    let smoothed_hr = hr_smoother.smoothed()?;
    let target_hr = ((target.lower_bound + target.upper_bound) / 2) as f64;
    let error = target_hr - smoothed_hr as f64;

    // Track time in zone
    let in_zone =
        smoothed_hr as u16 >= target.lower_bound && smoothed_hr as u16 <= target.upper_bound;
    if in_zone {
        s.time_in_zone_ms += 5000; // 5s tick
        s.phase = "in_zone".to_string();
        s.safety_note = None;
    } else {
        s.phase = "adjusting".to_string();
    }

    // Adaptive gains based on distance from target
    let (kp, ki, kd) = adaptive_gains(error.abs());
    pid.set_gains(kp, ki, kd);

    let dt_secs = 5.0; // HR mode tick interval
    let watts_adjustment = pid.update(error, dt_secs);

    // Rate limit: max ±HR_MAX_WATTS_PER_TICK per tick
    let clamped_adjustment =
        watts_adjustment.clamp(-HR_MAX_WATTS_PER_TICK, HR_MAX_WATTS_PER_TICK);

    let new_power_f = s.commanded_power as f64 + clamped_adjustment;

    // Clamp to [MIN_POWER, FTP×1.5]
    let max_power = s.ftp.map(|f| (f as f64 * 1.5) as u16).unwrap_or(400);
    let new_power = (new_power_f as u16).clamp(MIN_POWER, max_power);

    if new_power != s.commanded_power {
        Some(new_power)
    } else {
        None
    }
}
