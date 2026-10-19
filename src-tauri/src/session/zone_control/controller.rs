use std::sync::Arc;
use std::time::Instant;

use log::{debug, info, warn};
use tokio::sync::{broadcast, watch, Mutex};
use tokio::task::JoinHandle;

use crate::config::MAX_TRAINER_WATTS;
use crate::device::manager::DeviceManager;
use crate::device::types::{CommandSource, SensorReading};
use crate::error::AppError;

use super::pid::{HrPid, HrSmoother, PowerBand};
use super::types::{StopReason, ZoneControlStatus, ZoneMode, ZoneTarget};

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
        target.validate()?;

        // Verify trainer connected
        {
            let dm = device_manager.lock().await;
            if dm.connected_trainer_id().is_none() {
                return Err(AppError::Session("No trainer connected".into()));
            }
        }

        // Stop any existing control loop
        self.stop_internal().await;

        let initial_power = initial_power_for(&target, ftp, power_zones, initial_power_estimate);

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
    let mut pid = HrPid::new(state.lock().await.commanded_power);
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
    pid: &mut HrPid,
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

/// Initial commanded power. Power mode: target midpoint. HR mode: the
/// historical estimate (capped at 1.2×FTP / 300 W) or 55 % FTP (100 W without
/// FTP), always clamped into the HR power band so no clamp can panic.
fn initial_power_for(
    target: &ZoneTarget,
    ftp: Option<u16>,
    power_zones: Option<[u16; 6]>,
    estimate: Option<u16>,
) -> u16 {
    match target.mode {
        ZoneMode::Power => target.midpoint().min(MAX_TRAINER_WATTS),
        ZoneMode::HeartRate => {
            let band = hr_power_band(ftp, power_zones, target.zone);
            let raw = match estimate {
                Some(e) => e.min(ftp.map(|f| pct_of_ftp(f, 120)).unwrap_or(300)),
                None => ftp.map(|f| pct_of_ftp(f, 55)).unwrap_or(100),
            };
            raw.clamp(band.floor, band.ceiling)
        }
    }
}

fn pct_of_ftp(ftp: u16, pct: u16) -> u16 {
    (ftp as u32 * pct as u32 / 100).min(u16::MAX as u32) as u16
}

/// Power band an HR target may command (design ADR-9).
/// Zone 0 (custom), zones beyond 5, or missing zone config → [MIN_POWER, 1.5×FTP]
/// (400 W without FTP). Zone n → one power zone below to one above.
/// Always returns floor ≤ ceiling ≤ MAX_TRAINER_WATTS, for any input.
pub(crate) fn hr_power_band(ftp: Option<u16>, power_zones: Option<[u16; 6]>, zone: u8) -> PowerBand {
    let (floor, ceiling) = match (ftp, power_zones) {
        (Some(ftp), Some(pz)) if (1..=5).contains(&zone) => {
            let floor = if zone >= 2 { pct_of_ftp(ftp, pz[(zone - 2) as usize]) } else { MIN_POWER };
            let ceiling = pct_of_ftp(ftp, pz[(zone as usize).min(5)]);
            (floor, ceiling)
        }
        _ => (MIN_POWER, ftp.map(|f| pct_of_ftp(f, 150)).unwrap_or(400)),
    };
    let floor = floor.max(MIN_POWER).min(MAX_TRAINER_WATTS);
    let ceiling = ceiling.min(MAX_TRAINER_WATTS).max(floor);
    PowerBand { floor, ceiling }
}

/// HR mode tick: positional PID (design ADR-1) adjusts power toward the HR target.
/// Returns Some(new_watts) if power should be changed, None to hold.
fn process_hr_tick(
    s: &mut ControlLoopState,
    target: &ZoneTarget,
    pid: &mut HrPid,
    hr_smoother: &HrSmoother,
    tick_ms: u64,
) -> Option<u16> {
    let smoothed_hr = hr_smoother.smoothed()?;
    let target_hr = (target.lower_bound as f64 + target.upper_bound as f64) / 2.0;

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

    let band = hr_power_band(s.ftp, s.power_zones, target.zone);
    let dt_secs = tick_ms as f64 / 1000.0;
    let new_power = pid.update(target_hr, smoothed_hr as f64, dt_secs, band);

    if new_power != s.commanded_power {
        debug!(
            "HR PID: smoothed_hr={}, target={:.1}, integral={:.1}W, power {}W -> {}W",
            smoothed_hr, target_hr, pid.integral(), s.commanded_power, new_power
        );
        Some(new_power)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::pid::{HrPid, HrSmoother};
    use super::super::types::{ZoneMode, ZoneTarget};

    /// Push `bpm` into smoother 5 times so smoothed() == bpm.
    fn fill_smoother(smoother: &mut HrSmoother, bpm: u8) {
        for _ in 0..5 {
            smoother.push(bpm);
        }
    }

    /// HR zone 2 target: 139-151 bpm.
    fn hr_zone2_target() -> ZoneTarget {
        ZoneTarget {
            mode: ZoneMode::HeartRate,
            zone: 2,
            lower_bound: 139,
            upper_bound: 151,
            duration_secs: None,
        }
    }

    /// State for HR zone 2 with FTP=200 and standard power zones.
    /// Power zones [55,75,90,105,120,150]% → HR zone 2 band: floor 110 W (pz[0]), ceiling 180 W (pz[2]).
    fn make_zone2_state(commanded_power: u16) -> ControlLoopState {
        let mut s = ControlLoopState::new();
        s.active = true;
        s.commanded_power = commanded_power;
        s.ftp = Some(200);
        s.power_zones = Some([55, 75, 90, 105, 120, 150]);
        s
    }

    fn run_ticks(s: &mut ControlLoopState, target: &ZoneTarget, pid: &mut HrPid, smoother: &HrSmoother, n: usize) {
        for _ in 0..n {
            if let Some(new) = process_hr_tick(s, target, pid, smoother, 5000) {
                s.commanded_power = new;
            }
        }
    }

    #[test]
    fn power_band_clamps_floor() {
        // HR far above zone pushes power down; zone-2 floor is 110 W.
        let target = hr_zone2_target();
        let mut smoother = HrSmoother::new(5);
        fill_smoother(&mut smoother, 160);
        let mut s = make_zone2_state(115);
        let mut pid = HrPid::new(115);
        run_ticks(&mut s, &target, &mut pid, &smoother, 10);
        assert_eq!(s.commanded_power, 110);
    }

    #[test]
    fn power_band_clamps_ceiling() {
        // HR far below zone pushes power up; zone-2 ceiling is 180 W.
        let target = hr_zone2_target();
        let mut smoother = HrSmoother::new(5);
        fill_smoother(&mut smoother, 120);
        let mut s = make_zone2_state(175);
        let mut pid = HrPid::new(175);
        run_ticks(&mut s, &target, &mut pid, &smoother, 10);
        assert_eq!(s.commanded_power, 180);
    }

    fn new_hr_pid(p0: u16) -> HrPid {
        HrPid::new(p0)
    }

    // --- AC2 / AC3: power band ---

    const STD_PZ: [u16; 6] = [55, 75, 90, 105, 120, 150];

    #[test]
    fn custom_hr_zone_band_is_min_to_150pct_ftp() {
        assert_eq!(
            hr_power_band(Some(200), Some(STD_PZ), 0),
            PowerBand { floor: 50, ceiling: 300 }
        );
    }

    #[test]
    fn custom_hr_zone_without_ftp_caps_at_400() {
        assert_eq!(hr_power_band(None, Some(STD_PZ), 0), PowerBand { floor: 50, ceiling: 400 });
    }

    #[test]
    fn hr_zone_3_band_spans_neighbouring_power_zones() {
        // FTP 200: floor pz[1]=75% → 150, ceiling pz[3]=105% → 210
        assert_eq!(
            hr_power_band(Some(200), Some(STD_PZ), 3),
            PowerBand { floor: 150, ceiling: 210 }
        );
    }

    #[test]
    fn custom_hr_150_160_commands_above_55pct_ftp() {
        // FTP 200 → 55% = 110 W. HR well below a custom 150-160 target must push above it.
        let target = ZoneTarget {
            mode: ZoneMode::HeartRate,
            zone: 0,
            lower_bound: 150,
            upper_bound: 160,
            duration_secs: None,
        };
        let mut s = make_zone2_state(110);
        let mut pid = HrPid::new(110);
        let mut smoother = HrSmoother::new(5);
        fill_smoother(&mut smoother, 120);
        run_ticks(&mut s, &target, &mut pid, &smoother, 10);
        assert!(s.commanded_power > 110, "custom zone stuck at {} W", s.commanded_power);
    }

    #[test]
    fn hr_band_floor_never_exceeds_ceiling_for_any_ftp() {
        let zone_sets = [Some(STD_PZ), Some([1, 2, 3, 4, 5, 6]), Some([900, 950, 960, 970, 980, 1000]), None];
        for ftp in 1..=2000u16 {
            for zone in 0..=7u8 {
                for pz in zone_sets {
                    let b = hr_power_band(Some(ftp), pz, zone);
                    assert!(b.floor <= b.ceiling, "ftp={ftp} zone={zone} {b:?}");
                    assert!(b.ceiling <= MAX_TRAINER_WATTS, "ftp={ftp} zone={zone} {b:?}");
                }
            }
        }
    }

    #[test]
    fn initial_power_with_tiny_ftp_does_not_panic() {
        // Old code: estimate.clamp(50, 1.2×FTP=1) panicked (min > max).
        let target = ZoneTarget {
            mode: ZoneMode::HeartRate,
            zone: 3,
            lower_bound: 130,
            upper_bound: 140,
            duration_secs: None,
        };
        assert_eq!(initial_power_for(&target, Some(1), Some(STD_PZ), Some(120)), 50);
        assert_eq!(initial_power_for(&target, Some(1), Some(STD_PZ), None), 50);
    }

    #[test]
    fn initial_power_estimate_clamped_into_hr_band() {
        // FTP 200 zone 3 band [150, 210]; estimate 300 → capped at 1.2×FTP=240 → band 210
        let target = ZoneTarget {
            mode: ZoneMode::HeartRate,
            zone: 3,
            lower_bound: 130,
            upper_bound: 140,
            duration_secs: None,
        };
        assert_eq!(initial_power_for(&target, Some(200), Some(STD_PZ), Some(300)), 210);
    }

    // --- AC1: deterministic first-order-lag + delay HR plant simulation ---

    struct PlantParams {
        tau_s: f64,
        delay_s: f64,
        gain_bpm_per_w: f64,
        rest_bpm: f64,
    }

    struct SimResult {
        peak_hr: u8,
        /// (sim time secs, smoothed HR, commanded watts) at each controller tick
        ticks: Vec<(f64, u8, u16)>,
    }

    /// Simulates `minutes` of riding: HR follows a first-order lag of the
    /// commanded power delayed by `delay_s`. HR is sampled at 1 Hz into a
    /// real HrSmoother; the controller ticks every 5 s. No clock involved.
    fn simulate_hr_plant(p: &PlantParams, start_hr: f64, minutes: u32) -> SimResult {
        let target = ZoneTarget {
            mode: ZoneMode::HeartRate,
            zone: 0,
            lower_bound: 130,
            upper_bound: 140,
            duration_secs: None,
        };
        // Start in steady state at start_hr: P0 = (HR - rest) / gain
        let p0 = ((start_hr - p.rest_bpm) / p.gain_bpm_per_w).round() as u16;
        let mut s = ControlLoopState::new();
        s.active = true;
        s.commanded_power = p0;
        s.ftp = Some(250); // band [50, 375]
        s.power_zones = None;
        let mut pid = new_hr_pid(p0);
        let mut smoother = HrSmoother::new(5);

        let dt = 0.1;
        let delay_steps = (p.delay_s / dt).round() as usize;
        let mut delay_line: std::collections::VecDeque<f64> =
            std::iter::repeat(p0 as f64).take(delay_steps + 1).collect();
        let mut hr = start_hr;
        let mut peak_hr = 0u8;
        let mut ticks = Vec::new();
        let total_steps = (minutes as usize) * 60 * 10;
        for step in 1..=total_steps {
            let delayed = delay_line.pop_front().unwrap();
            delay_line.push_back(s.commanded_power as f64);
            hr += (p.rest_bpm + p.gain_bpm_per_w * delayed - hr) / p.tau_s * dt;
            if step % 10 == 0 {
                smoother.push(hr.round() as u8);
            }
            if step % 50 == 0 {
                if let Some(w) = process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000) {
                    s.commanded_power = w;
                }
                let sm = smoother.smoothed().unwrap();
                peak_hr = peak_hr.max(sm);
                ticks.push((step as f64 * dt, sm, s.commanded_power));
            }
        }
        SimResult { peak_hr, ticks }
    }

    fn assert_settles_without_overshoot(p: &PlantParams, label: &str) {
        let minutes = 20;
        let r = simulate_hr_plant(p, 115.0, minutes);
        let end = (minutes * 60) as f64;
        // Overshoot bound: one zone half-width (5 bpm) above the 135 midpoint
        assert!(
            r.peak_hr <= 140,
            "{label}: overshoot — peak smoothed HR {} > 140",
            r.peak_hr
        );
        // Settled: every tick in the last 10 minutes inside the band
        for &(t, hr, _) in r.ticks.iter().filter(|(t, _, _)| *t > end - 600.0) {
            assert!(
                (130..=140).contains(&hr),
                "{label}: HR {hr} outside 130-140 at t={t}s"
            );
        }
        // No sustained oscillation: power swing over last 5 minutes <= 10 W
        let last: Vec<u16> = r
            .ticks
            .iter()
            .filter(|(t, _, _)| *t > end - 300.0)
            .map(|x| x.2)
            .collect();
        let swing = last.iter().max().unwrap() - last.iter().min().unwrap();
        assert!(swing <= 10, "{label}: power swing {swing} W in last 5 min");
    }

    #[test]
    fn hr_plant_20bpm_below_settles_without_overshoot_or_oscillation() {
        let p = PlantParams { tau_s: 60.0, delay_s: 15.0, gain_bpm_per_w: 0.35, rest_bpm: 60.0 };
        assert_settles_without_overshoot(&p, "reference plant");
    }

    #[test]
    fn hr_plant_grid_settles_across_lag_and_delay() {
        for tau_s in [30.0, 60.0, 90.0] {
            for delay_s in [5.0, 15.0, 25.0] {
                for gain in [0.25, 0.35] {
                    let p = PlantParams { tau_s, delay_s, gain_bpm_per_w: gain, rest_bpm: 60.0 };
                    assert_settles_without_overshoot(
                        &p,
                        &format!("tau={tau_s} delay={delay_s} gain={gain}"),
                    );
                }
            }
        }
    }
}
