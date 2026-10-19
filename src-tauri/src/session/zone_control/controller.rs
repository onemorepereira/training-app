//! Zone control: drives a smart trainer toward a power or HR target.
//! See docs/design/zone-control-safety/design.md for the decisions referenced
//! below (ADR-n).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use log::{debug, error, info, warn};
use tokio::sync::{broadcast, mpsc, watch, Mutex};
use tokio::time::Instant;

use crate::config::MAX_TRAINER_WATTS;
use crate::device::types::{CommandSource, SensorReading};
use crate::error::AppError;

use super::pid::{HrPid, HrSmoother, PowerBand};
use super::trainer::{
    log_release, DeviceManagerBackend, SerializedSink, TrainerBackend, TrainerCmd,
    DEFAULT_RELEASE_BUDGET, TRAINER_COMMAND_TIMEOUT,
};
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
/// Trainer unreachable for this long (no safety override pending) → stop (ADR-4)
pub const TRAINER_LOST_STOP_SECS: u64 = 30;
/// Safety override undelivered for this many consecutive ticks → stop (ADR-15)
pub const SAFETY_UNDELIVERED_STOP_TICKS: u32 = 3;
/// … or for this much wall-clock time, whichever comes first (ADR-24)
pub const SAFETY_UNDELIVERED_STOP: Duration = Duration::from_secs(6);
/// Power mode: after a cadence-zero cut, restore the target only once cadence
/// ≥ this has been sustained for POWER_RESTORE_CADENCE_SECS (ADR-25)
pub const POWER_RESTORE_CADENCE_RPM: f32 = 40.0;
pub const POWER_RESTORE_CADENCE_SECS: u64 = 2;
/// HR-ceiling override clears below max_hr − margin for N consecutive samples (ADR-15)
pub const HR_CEILING_CLEAR_MARGIN_BPM: u8 = 3;
pub const HR_CEILING_CLEAR_SAMPLES: u32 = 2;
/// Re-assert the current target at least this often (covers missed reconnects)
pub const TRAINER_REFRESH_SECS: u64 = 10;
/// Total release budget on app close / exit (ADR-17, ADR-19)
pub const APP_CLOSE_RELEASE_BUDGET: Duration = Duration::from_secs(4);

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
    last_cadence_zero_since: Option<Instant>,
    /// Since when cadence has been ≥ POWER_RESTORE_CADENCE_RPM (None if below)
    cadence_ok_since: Option<Instant>,
    last_hr_seen: Option<Instant>,
    last_power_seen: Option<Instant>,
    /// FTP from user config, used for HR mode power banding
    ftp: Option<u16>,
    /// Max HR from user config, used for HR ceiling safety
    max_hr: Option<u8>,
    /// Instant of the last processed tick, for measuring actual elapsed time.
    /// Advances on every tick, paused or not (AC8).
    last_tick_at: Option<Instant>,
    /// Power zone percentages from user config (for HR mode power banding)
    power_zones: Option<[u16; 6]>,
    /// Whether finalize's 0 W release was confirmed (None until finalize ran)
    release_confirmed: Option<bool>,
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
            last_cadence_zero_since: None,
            cadence_ok_since: None,
            last_hr_seen: None,
            last_power_seen: None,
            ftp: None,
            max_hr: None,
            last_tick_at: None,
            power_zones: None,
            release_confirmed: None,
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

    fn status(&self) -> ZoneControlStatus {
        ZoneControlStatus {
            active: self.active,
            mode: self.target.as_ref().map(|t| t.mode),
            target_zone: self.target.as_ref().map(|t| t.zone),
            lower_bound: self.target.as_ref().map(|t| t.lower_bound),
            upper_bound: self.target.as_ref().map(|t| t.upper_bound),
            commanded_power: if self.active { Some(self.commanded_power) } else { None },
            time_in_zone_secs: self.time_in_zone_ms / 1000,
            elapsed_secs: self.elapsed_ms() / 1000,
            duration_secs: self.target.as_ref().and_then(|t| t.duration_secs),
            paused: self.paused,
            phase: self.phase.clone(),
            safety_note: self.safety_note.clone(),
            stop_reason: self.stop_reason.clone(),
        }
    }
}

#[derive(Debug)]
enum ControlMsg {
    /// Stop with reason; optional total release budget (app close)
    Stop(StopReason, Option<Duration>),
    Pause,
    Resume,
}

/// One zone-control run (ADR-12). Finalize only ever touches this run's state.
struct Run {
    state: Arc<Mutex<ControlLoopState>>,
    ctl_tx: mpsc::UnboundedSender<ControlMsg>,
    /// Some(reason) once finalize (including the trainer release) completed
    done: watch::Receiver<Option<StopReason>>,
}

pub struct ZoneController<B: TrainerBackend = DeviceManagerBackend> {
    sink: Arc<SerializedSink<B>>,
    /// Held for the whole of start() and stop() (ADR-12)
    lifecycle: Mutex<()>,
    /// Latest run; brief std lock, never held across .await
    current: std::sync::Mutex<Option<Arc<Run>>>,
    next_gen: AtomicU64,
    /// Set once app close begins; start() is refused from then on (ADR-26)
    closing: AtomicBool,
}

impl<B: TrainerBackend> ZoneController<B> {
    pub fn new(backend: B) -> Self {
        Self {
            sink: Arc::new(SerializedSink::new(backend)),
            lifecycle: Mutex::new(()),
            current: std::sync::Mutex::new(None),
            next_gen: AtomicU64::new(1),
            closing: AtomicBool::new(false),
        }
    }

    fn current_run(&self) -> Option<Arc<Run>> {
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn start_with_config(
        &self,
        target: ZoneTarget,
        sensor_tx: broadcast::Sender<SensorReading>,
        ftp: Option<u16>,
        max_hr: Option<u8>,
        initial_power_estimate: Option<u16>,
        power_zones: Option<[u16; 6]>,
    ) -> Result<(), AppError> {
        target.validate()?;
        let _life = self.lifecycle.lock().await;
        if self.closing.load(Ordering::SeqCst) {
            return Err(AppError::Session("App is closing".into()));
        }

        if !self.sink.is_connected().await {
            return Err(AppError::Session("No trainer connected".into()));
        }

        // Any previous run (live or self-finalizing) must be fully released first
        if let Some(prev) = self.current_run() {
            Self::stop_run(&prev, StopReason::UserStopped, None).await;
        }

        let gen = self.next_gen.fetch_add(1, Ordering::SeqCst);
        self.sink.set_active_gen(gen);

        let initial_power = initial_power_for(&target, ftp, power_zones, initial_power_estimate);
        let now = Instant::now();
        let mut st = ControlLoopState::new();
        st.active = true;
        st.target = Some(target.clone());
        st.commanded_power = initial_power;
        st.started_at = Some(now);
        st.last_tick_at = Some(now);
        st.phase = "ramping".to_string();
        st.last_hr_seen = Some(now);
        st.last_power_seen = Some(now);
        st.ftp = ftp;
        st.max_hr = max_hr;
        st.power_zones = power_zones;
        let state = Arc::new(Mutex::new(st));

        let (ctl_tx, ctl_rx) = mpsc::unbounded_channel();
        let (done_tx, done_rx) = watch::channel(None);
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(Run {
            state: state.clone(),
            ctl_tx,
            done: done_rx,
        }));

        info!(
            "Zone control started (run {}): {:?} zone {} ({}-{} {}), initial {}W",
            gen,
            target.mode,
            target.zone,
            target.lower_bound,
            target.upper_bound,
            if target.mode == ZoneMode::Power { "W" } else { "bpm" },
            initial_power
        );

        let sensor_rx = sensor_tx.subscribe();
        let control = ControlLoop::new(
            gen,
            state.clone(),
            target,
            self.sink.clone(),
            sensor_tx.clone(),
            initial_power,
        );
        let sink = self.sink.clone();
        // Supervisor (ADR-8): every way the control task ends goes through finalize.
        tokio::spawn(async move {
            let inner = tokio::spawn(control.run(ctl_rx, sensor_rx));
            let (reason, budget) = match inner.await {
                Ok(v) => v,
                Err(e) => {
                    error!("Zone control task ended abnormally (run {gen}): {e}");
                    (StopReason::ControllerFault, None)
                }
            };
            let fin = tokio::spawn(finalize(gen, state.clone(), sink, sensor_tx, reason, budget, done_tx));
            if let Err(e) = fin.await {
                error!("Zone control finalize failed (run {gen}): {e}");
                mark_stopped(&state, StopReason::ControllerFault).await;
            }
        });

        Ok(())
    }

    /// Ask a run to stop and wait until its finalize (trainer release) is done.
    async fn stop_run(run: &Run, reason: StopReason, budget: Option<Duration>) -> StopReason {
        let mut done = run.done.clone();
        if let Some(r) = done.borrow().clone() {
            return r;
        }
        // Ignore send errors: the run may already be ending on its own.
        let _ = run.ctl_tx.send(ControlMsg::Stop(reason, budget));
        let finished = done.wait_for(|d| d.is_some()).await.map(|v| v.clone());
        match finished {
            Ok(Some(r)) => r,
            _ => {
                // ADR-20: finalize dropped its sender (panicked) → controller fault
                error!("Zone control finalize did not complete; treating as controller fault");
                mark_stopped(&run.state, StopReason::ControllerFault).await;
                StopReason::ControllerFault
            }
        }
    }

    async fn stop_inner(&self, reason: StopReason, budget: Option<Duration>) -> Option<StopReason> {
        let _life = self.lifecycle.lock().await;
        let run = self.current_run()?;
        let r = Self::stop_run(&run, reason, budget).await;
        info!("Zone control stopped: {:?}", r);
        Some(r)
    }

    /// Stop zone control and return once the trainer has been released.
    /// Returns the reason the run ended (the first reason wins), or None if
    /// no run ever existed.
    pub async fn stop(&self, reason: StopReason) -> Option<StopReason> {
        self.stop_inner(reason, None).await
    }

    /// Like `stop`, but never takes longer than `budget` in total (ADR-19).
    pub async fn stop_with_budget(&self, reason: StopReason, budget: Duration) -> Option<StopReason> {
        match tokio::time::timeout(budget, self.stop_inner(reason.clone(), Some(budget))).await {
            Ok(r) => r,
            Err(_) => {
                error!("Zone control stop ({reason:?}) exceeded {budget:?}: trainer release unconfirmed");
                None
            }
        }
    }

    /// App close / exit (ADR-26): refuse new starts, then stop any run
    /// (including one that raced this call) within `budget`.
    pub async fn close(&self, budget: Duration) -> Option<StopReason> {
        self.closing.store(true, Ordering::SeqCst);
        self.stop_with_budget(StopReason::AppClosing, budget).await
    }

    fn is_run_active(&self) -> bool {
        self.current_run()
            .map(|run| run.done.borrow().is_none())
            .unwrap_or(false)
    }

    /// Manual trainer commands must not fight a running zone ride.
    pub fn ensure_manual_control_allowed(&self) -> Result<(), AppError> {
        if self.is_run_active() {
            Err(AppError::Session("Zone ride active — stop it first".into()))
        } else {
            Ok(())
        }
    }

    /// Whether the latest run's final 0 W release was confirmed.
    /// (Used by tests; the production path reports it in the log.)
    #[cfg_attr(not(test), allow(dead_code))]
    pub async fn last_release_confirmed(&self) -> Option<bool> {
        let run = self.current_run()?;
        let confirmed = run.state.lock().await.release_confirmed;
        confirmed
    }

    fn send_to_live_run(&self, msg: ControlMsg) {
        if let Some(run) = self.current_run() {
            if run.done.borrow().is_none() {
                let _ = run.ctl_tx.send(msg);
            }
        }
    }

    /// Pause control: the trainer is released until resume (ADR-13).
    /// Waits for any in-flight start/stop so a pause during start reaches the
    /// new run (ruling 6).
    pub async fn pause(&self) {
        let _life = self.lifecycle.lock().await;
        self.send_to_live_run(ControlMsg::Pause);
    }

    /// Resume control: the target is re-sent on the next tick.
    pub async fn resume(&self) {
        let _life = self.lifecycle.lock().await;
        self.send_to_live_run(ControlMsg::Resume);
    }

    pub async fn status(&self) -> ZoneControlStatus {
        match self.current_run() {
            Some(run) => run.state.lock().await.status(),
            None => ControlLoopState::new().status(),
        }
    }
}

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn log_trainer_command(sensor_tx: &broadcast::Sender<SensorReading>, watts: u16) {
    let _ = sensor_tx.send(SensorReading::TrainerCommand {
        target_watts: watts,
        epoch_ms: now_epoch_ms(),
        source: CommandSource::ZoneControl,
    });
}

async fn mark_stopped(state: &Arc<Mutex<ControlLoopState>>, reason: StopReason) {
    let mut s = state.lock().await;
    if let Some(p) = s.pause_started.take() {
        s.paused_accumulated_ms += p.elapsed().as_millis() as u64;
    }
    s.active = false;
    s.paused = false;
    s.phase = "stopped".to_string();
    s.stop_reason = Some(reason);
}

/// Release the trainer, then flip status to stopped (ADR-11, ADR-21).
async fn finalize<B: TrainerBackend>(
    gen: u64,
    state: Arc<Mutex<ControlLoopState>>,
    sink: Arc<SerializedSink<B>>,
    sensor_tx: broadcast::Sender<SensorReading>,
    reason: StopReason,
    budget: Option<Duration>,
    done_tx: watch::Sender<Option<StopReason>>,
) {
    let outcome = sink.release(gen, budget.unwrap_or(DEFAULT_RELEASE_BUDGET)).await;
    log_release(&format!("Zone control ended ({reason:?}, run {gen})"), &outcome);
    if outcome.delivered {
        log_trainer_command(&sensor_tx, 0);
    }
    state.lock().await.release_confirmed = Some(outcome.delivered);
    mark_stopped(&state, reason.clone()).await;
    let _ = done_tx.send(Some(reason));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delivered {
    Unknown,
    Watts(u16),
    Released,
}

#[derive(Debug, Clone, Copy)]
enum Step {
    /// Send the initial target
    Initial,
    Tick,
    /// Reconcile immediately (e.g. pause release)
    Reconcile,
}

struct ControlLoop<B: TrainerBackend> {
    gen: u64,
    state: Arc<Mutex<ControlLoopState>>,
    target: ZoneTarget,
    sink: Arc<SerializedSink<B>>,
    sensor_tx: broadcast::Sender<SensorReading>,
    pid: HrPid,
    smoother: HrSmoother,
    /// What the trainer was last confirmed to be doing
    delivered: Delivered,
    last_confirmed_watts: u16,
    last_sent_at: Option<Instant>,
    trainer_lost_since: Option<Instant>,
    undelivered_safety_ticks: u32,
    /// Start of the current undelivered-safety-override episode (ADR-24)
    safety_pending_since: Option<Instant>,
    /// Power mode cadence cut is latched until cadence recovers (ADR-25)
    power_cadence_cut: bool,
    hr_ceiling_override: bool,
    hr_clear_samples: u32,
    /// PID was frozen on the previous tick (re-seed on exit, ADR-14)
    was_frozen: bool,
}

impl<B: TrainerBackend> ControlLoop<B> {
    fn new(
        gen: u64,
        state: Arc<Mutex<ControlLoopState>>,
        target: ZoneTarget,
        sink: Arc<SerializedSink<B>>,
        sensor_tx: broadcast::Sender<SensorReading>,
        initial_power: u16,
    ) -> Self {
        Self {
            gen,
            state,
            target,
            sink,
            sensor_tx,
            pid: HrPid::new(initial_power),
            smoother: HrSmoother::new(5),
            delivered: Delivered::Unknown,
            last_confirmed_watts: 0,
            last_sent_at: None,
            trainer_lost_since: None,
            undelivered_safety_ticks: 0,
            safety_pending_since: None,
            power_cadence_cut: false,
            hr_ceiling_override: false,
            hr_clear_samples: 0,
            was_frozen: false,
        }
    }

    /// Runs until stopped; returns the stop reason and optional release budget.
    async fn run(
        mut self,
        mut ctl_rx: mpsc::UnboundedReceiver<ControlMsg>,
        mut sensor_rx: broadcast::Receiver<SensorReading>,
    ) -> (StopReason, Option<Duration>) {
        let period = match self.target.mode {
            ZoneMode::Power => Duration::from_secs(1),
            ZoneMode::HeartRate => Duration::from_secs(5),
        };
        let mut tick = tokio::time::interval_at(Instant::now() + period, period);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut next_step = Some(Step::Initial);
        let mut pending: Vec<ControlMsg> = Vec::new();
        loop {
            if let Some(step) = next_step.take() {
                // Trainer I/O can take up to the command timeout; keep listening
                // for Stop so shutdown is never delayed by an in-flight command.
                let outcome = {
                    let fut = self.step(step);
                    tokio::pin!(fut);
                    loop {
                        tokio::select! {
                            biased;
                            msg = ctl_rx.recv() => match msg {
                                Some(ControlMsg::Stop(r, b)) => break Err((r, b)),
                                Some(other) => pending.push(other),
                                None => break Err((StopReason::ControllerFault, None)),
                            },
                            res = &mut fut => break Ok(res),
                        }
                    }
                };
                match outcome {
                    Err(stop) => return stop,
                    Ok(Some(reason)) => return (reason, None),
                    Ok(None) => {}
                }
                for msg in std::mem::take(&mut pending) {
                    if let Some(step) = self.on_msg(msg).await {
                        next_step = Some(step);
                    }
                }
                continue;
            }

            tokio::select! {
                // Control messages (stop/pause) take priority over ticks and readings
                biased;
                msg = ctl_rx.recv() => match msg {
                    Some(ControlMsg::Stop(r, b)) => return (r, b),
                    Some(other) => next_step = self.on_msg(other).await,
                    None => return (StopReason::ControllerFault, None),
                },
                result = sensor_rx.recv() => match result {
                    Ok(reading) => self.on_reading(reading).await,
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => {
                        return (StopReason::ControllerFault, None)
                    }
                },
                _ = tick.tick() => next_step = Some(Step::Tick),
            }
        }
    }

    async fn step(&mut self, step: Step) -> Option<StopReason> {
        match step {
            Step::Initial => {
                let w = self.state.lock().await.commanded_power;
                self.reconcile(Delivered::Watts(w), TRAINER_COMMAND_TIMEOUT).await;
                None
            }
            Step::Reconcile => {
                let paused = self.state.lock().await.paused;
                if paused {
                    self.reconcile(Delivered::Released, TRAINER_COMMAND_TIMEOUT).await;
                }
                None
            }
            Step::Tick => self.on_tick().await,
        }
    }

    async fn on_msg(&mut self, msg: ControlMsg) -> Option<Step> {
        let now = Instant::now();
        let mut s = self.state.lock().await;
        match msg {
            ControlMsg::Pause => {
                if !s.paused {
                    s.paused = true;
                    s.pause_started = Some(now);
                    s.phase = "paused".to_string();
                    info!("Zone control paused (run {}): releasing trainer", self.gen);
                }
                Some(Step::Reconcile)
            }
            ControlMsg::Resume => {
                if s.paused {
                    if let Some(p) = s.pause_started.take() {
                        s.paused_accumulated_ms += now.duration_since(p).as_millis() as u64;
                    }
                    s.paused = false;
                    // Sensor timers restart so a quiet pause doesn't trip "sensor lost"
                    s.last_hr_seen = Some(now);
                    s.last_power_seen = Some(now);
                    info!("Zone control resumed (run {})", self.gen);
                }
                None
            }
            ControlMsg::Stop(..) => None, // handled by run()
        }
    }

    async fn on_reading(&mut self, reading: SensorReading) {
        let now = Instant::now();
        let mut s = self.state.lock().await;
        match reading {
            SensorReading::Power { watts, .. } => {
                s.last_power = Some(watts);
                s.last_power_seen = Some(now);
            }
            SensorReading::HeartRate { bpm, .. } => {
                s.last_hr_seen = Some(now);
                self.smoother.push(bpm);
                if self.target.mode == ZoneMode::HeartRate {
                    let max_hr = s.max_hr;
                    drop(s);
                    self.update_hr_ceiling(max_hr);
                }
            }
            SensorReading::Cadence { rpm, .. } => {
                if rpm < 1.0 {
                    if s.last_cadence_zero_since.is_none() {
                        s.last_cadence_zero_since = Some(now);
                    }
                } else {
                    s.last_cadence_zero_since = None;
                }
                if rpm >= POWER_RESTORE_CADENCE_RPM {
                    s.cadence_ok_since.get_or_insert(now);
                } else {
                    s.cadence_ok_since = None;
                }
            }
            _ => {}
        }
    }

    /// HR-ceiling override with hysteresis on smoothed HR (ADR-15).
    fn update_hr_ceiling(&mut self, max_hr: Option<u8>) {
        let (Some(max_hr), Some(hr)) = (max_hr, self.smoother.smoothed()) else {
            return;
        };
        if hr > max_hr {
            if !self.hr_ceiling_override {
                warn!("HR ceiling exceeded: {} bpm > {} max — reducing to {}W", hr, max_hr, SAFETY_POWER);
            }
            self.hr_ceiling_override = true;
            self.hr_clear_samples = 0;
        } else if self.hr_ceiling_override {
            if hr < max_hr.saturating_sub(HR_CEILING_CLEAR_MARGIN_BPM) {
                self.hr_clear_samples += 1;
                if self.hr_clear_samples >= HR_CEILING_CLEAR_SAMPLES {
                    info!("HR back below ceiling ({} bpm) — resuming control", hr);
                    self.hr_ceiling_override = false;
                    self.hr_clear_samples = 0;
                }
            } else {
                self.hr_clear_samples = 0;
            }
        }
    }

    fn delivered_watts(&self) -> u16 {
        match self.delivered {
            Delivered::Watts(w) => w,
            Delivered::Released => 0,
            Delivered::Unknown => self.last_confirmed_watts,
        }
    }

    /// Send `effective` if the trainer isn't confirmed to be doing it (ADR-4).
    async fn reconcile(&mut self, effective: Delivered, limit: Duration) {
        let now = Instant::now();
        let refresh_due = matches!(effective, Delivered::Watts(_))
            && self
                .last_sent_at
                .map(|t| now.duration_since(t) >= Duration::from_secs(TRAINER_REFRESH_SECS))
                .unwrap_or(true);
        if self.delivered == effective && !refresh_due {
            return;
        }
        let ok = match effective {
            Delivered::Watts(w) => {
                match self.sink.send(self.gen, TrainerCmd::Power(w), limit).await {
                    Ok(()) => {
                        log_trainer_command(&self.sensor_tx, w);
                        true
                    }
                    Err(e) => {
                        warn!("Zone control: trainer command {}W failed: {}", w, e);
                        false
                    }
                }
            }
            Delivered::Released => {
                let out = self.sink.release(self.gen, DEFAULT_RELEASE_BUDGET).await;
                log_release(&format!("Zone control paused (run {})", self.gen), &out);
                if out.delivered {
                    log_trainer_command(&self.sensor_tx, 0);
                }
                out.delivered
            }
            Delivered::Unknown => return,
        };
        if ok {
            self.delivered = effective;
            self.last_sent_at = Some(now);
            if let Delivered::Watts(w) = effective {
                self.last_confirmed_watts = w;
            }
            if self.trainer_lost_since.take().is_some() {
                info!("Zone control: trainer reachable again (run {})", self.gen);
            }
        } else {
            self.delivered = Delivered::Unknown;
            self.trainer_lost_since.get_or_insert(now);
        }
    }

    async fn on_tick(&mut self) -> Option<StopReason> {
        let now = Instant::now();
        if !self.sink.is_connected().await {
            if self.delivered != Delivered::Unknown {
                warn!("Zone control: trainer not connected (run {})", self.gen);
            }
            self.delivered = Delivered::Unknown;
            self.trainer_lost_since.get_or_insert(now);
        }

        let state = self.state.clone();
        let mut s = state.lock().await;
        let max_tick = match self.target.mode {
            ZoneMode::Power => 2_000,
            ZoneMode::HeartRate => 10_000,
        };
        let tick_ms = s
            .last_tick_at
            .map(|t| now.duration_since(t).as_millis() as u64)
            .unwrap_or(0)
            .min(max_tick);
        s.last_tick_at = Some(now);
        let paused = s.paused;
        let hr_mode = self.target.mode == ZoneMode::HeartRate;

        let mut hr_holding = false;
        if !paused {
            if let Some(duration) = self.target.duration_secs {
                if s.elapsed_ms() / 1000 >= duration {
                    info!("Zone control: duration complete");
                    return Some(StopReason::DurationComplete);
                }
            }
            if hr_mode {
                let hr_lost_secs = s
                    .last_hr_seen
                    .map(|t| now.duration_since(t).as_secs())
                    .unwrap_or(u64::MAX);
                if hr_lost_secs >= HR_SENSOR_STOP_SECS {
                    warn!("HR sensor lost for {}s — stopping zone control", hr_lost_secs);
                    s.safety_note = Some("HR sensor lost".to_string());
                    return Some(StopReason::SensorLost);
                }
                hr_holding = hr_lost_secs >= HR_SENSOR_WARN_SECS;
            }
        }

        if hr_mode {
            let max_hr = s.max_hr;
            self.update_hr_ceiling_on_tick(max_hr);
        }
        let cadence_zero_long = s
            .last_cadence_zero_since
            .map(|z| now.duration_since(z).as_secs() >= CADENCE_ZERO_SECS)
            .unwrap_or(false);
        let cadence_override = if hr_mode {
            cadence_zero_long
        } else {
            if cadence_zero_long {
                self.power_cadence_cut = true;
            } else if self.power_cadence_cut
                && s
                    .cadence_ok_since
                    .map(|t| now.duration_since(t).as_secs() >= POWER_RESTORE_CADENCE_SECS)
                    .unwrap_or(false)
            {
                info!("Cadence sustained ≥ {} rpm — restoring target", POWER_RESTORE_CADENCE_RPM);
                self.power_cadence_cut = false;
            }
            self.power_cadence_cut
        };
        let safety = if cadence_override {
            Some(0)
        } else if hr_mode && self.hr_ceiling_override {
            Some(SAFETY_POWER)
        } else {
            None
        };
        let trainer_lost = self.trainer_lost_since.is_some();
        let frozen = paused || safety.is_some() || trainer_lost || hr_holding;

        // Zone tracking (not while paused)
        let smoothed = self.smoother.smoothed();
        let effective = if paused {
            Delivered::Released
        } else {
            match self.target.mode {
                ZoneMode::Power => {
                    process_power_tick(&mut s, &self.target, tick_ms);
                    if safety.is_none() {
                        Delivered::Watts(self.target.midpoint().min(MAX_TRAINER_WATTS))
                    } else {
                        Delivered::Watts(0)
                    }
                }
                ZoneMode::HeartRate => {
                    if frozen {
                        if let Some(hr) = smoothed {
                            track_hr_zone(&mut s, &self.target, hr, tick_ms);
                        }
                    } else {
                        if self.was_frozen {
                            self.pid.reseed(self.delivered_watts());
                        }
                        process_hr_tick(&mut s, &self.target, &mut self.pid, &self.smoother, tick_ms);
                    }
                    match safety {
                        Some(w) => Delivered::Watts(w),
                        None => Delivered::Watts(self.pid.last_output()),
                    }
                }
            }
        };
        self.was_frozen = frozen;

        // Notes override zone-tracking defaults
        if cadence_override {
            s.safety_note = Some("Cadence zero — power reduced".to_string());
        } else if safety.is_some() {
            s.safety_note = Some("HR ceiling exceeded".to_string());
            s.phase = "adjusting".to_string();
        } else if hr_holding {
            s.safety_note = Some("HR sensor not responding — holding power".to_string());
        } else if self.target.mode == ZoneMode::Power {
            let power_lost_secs = s
                .last_power_seen
                .map(|t| now.duration_since(t).as_secs())
                .unwrap_or(u64::MAX);
            if power_lost_secs >= POWER_SENSOR_WARN_SECS {
                s.safety_note = Some("Power sensor not responding".to_string());
            }
        }
        s.commanded_power = match effective {
            Delivered::Watts(w) => w,
            _ => 0,
        };
        drop(s);

        // Undelivered safety override: bound the episode by wall clock too (ADR-24)
        let safety_pending = !paused && safety.is_some();
        let mut limit = TRAINER_COMMAND_TIMEOUT;
        if safety_pending && self.delivered != effective {
            let since = *self.safety_pending_since.get_or_insert(now);
            limit = limit.min((since + SAFETY_UNDELIVERED_STOP).saturating_duration_since(now));
        }
        self.reconcile(effective, limit).await;

        let mut s = self.state.lock().await;
        if paused {
            s.phase = "paused".to_string();
        } else if self.trainer_lost_since.is_some() {
            s.phase = "trainer_lost".to_string();
        }
        drop(s);

        if safety_pending && self.delivered != effective {
            self.undelivered_safety_ticks += 1;
            let pending_for = self
                .safety_pending_since
                .map(|t| Instant::now().duration_since(t))
                .unwrap_or_default();
            if self.undelivered_safety_ticks >= SAFETY_UNDELIVERED_STOP_TICKS
                || pending_for >= SAFETY_UNDELIVERED_STOP
            {
                error!(
                    "Safety power cut undelivered for {} ticks / {:?} — stopping zone control",
                    self.undelivered_safety_ticks, pending_for
                );
                return Some(StopReason::TrainerUnresponsive);
            }
        } else {
            self.undelivered_safety_ticks = 0;
            self.safety_pending_since = None;
        }
        if !safety_pending {
            if let Some(since) = self.trainer_lost_since {
                if now.duration_since(since) >= Duration::from_secs(TRAINER_LOST_STOP_SECS) {
                    warn!("Trainer unreachable for {}s — stopping zone control", TRAINER_LOST_STOP_SECS);
                    return Some(StopReason::TrainerDisconnected);
                }
            }
        }
        None
    }

    /// Tick-time ceiling entry check (samples drive exit hysteresis).
    fn update_hr_ceiling_on_tick(&mut self, max_hr: Option<u8>) {
        if let (Some(max_hr), Some(hr)) = (max_hr, self.smoother.smoothed()) {
            if hr > max_hr {
                self.update_hr_ceiling(Some(max_hr));
            }
        }
    }
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

fn track_hr_zone(s: &mut ControlLoopState, target: &ZoneTarget, smoothed_hr: u8, tick_ms: u64) {
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
    track_hr_zone(s, target, smoothed_hr, tick_ms);

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

    #[derive(Clone)]
    pub(crate) struct PlantParams {
        pub(crate) tau_s: f64,
        pub(crate) delay_s: f64,
        pub(crate) gain_bpm_per_w: f64,
        pub(crate) rest_bpm: f64,
    }

    struct SimResult {
        peak_hr: u8,
        /// (sim time secs, smoothed HR, commanded watts) at each controller tick
        ticks: Vec<(f64, u8, u16)>,
    }

    /// Simulates `minutes` of riding: HR follows a first-order lag of the
    /// commanded power delayed by `delay_s`. HR is sampled at 1 Hz into a
    /// real HrSmoother; the controller ticks every 5 s. No clock involved.
    /// Deterministic HR plant: first-order lag of power delayed by `delay_s`,
    /// sampled at 1 Hz with optional seeded ±3 bpm measurement noise.
    pub(crate) struct Plant {
        p: PlantParams,
        hr: f64,
        delay_line: std::collections::VecDeque<f64>,
        rng: Option<u64>,
    }

    impl Plant {
        /// `hr0` is the starting HR; `prior_power` is what was driving the plant
        /// before t=0 (steady start when it matches hr0, rising when higher).
        pub(crate) fn new(p: PlantParams, hr0: f64, prior_power: f64, noise_seed: Option<u64>) -> Self {
            let steps = (p.delay_s / 0.1).round() as usize;
            Self {
                delay_line: std::iter::repeat(prior_power).take(steps + 1).collect(),
                p,
                hr: hr0,
                rng: noise_seed,
            }
        }

        fn noise(&mut self) -> f64 {
            // 64-bit LCG (Knuth MMIX); uniform integer in [-3, 3]
            let Some(state) = self.rng.as_mut() else { return 0.0 };
            *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((*state >> 33) % 7) as f64 - 3.0
        }

        /// Advance one second under `power`; returns the 1 Hz HR sample.
        pub(crate) fn step_second(&mut self, power: u16) -> u8 {
            for _ in 0..10 {
                let delayed = self.delay_line.pop_front().unwrap();
                self.delay_line.push_back(power as f64);
                self.hr += (self.p.rest_bpm + self.p.gain_bpm_per_w * delayed - self.hr) / self.p.tau_s * 0.1;
            }
            let n = self.noise();
            (self.hr + n).round().clamp(0.0, 255.0) as u8
        }
    }

    pub(crate) fn steady_power(p: &PlantParams, hr: f64) -> f64 {
        (hr - p.rest_bpm) / p.gain_bpm_per_w
    }

    fn sim_target() -> ZoneTarget {
        ZoneTarget {
            mode: ZoneMode::HeartRate,
            zone: 0,
            lower_bound: 130,
            upper_bound: 140,
            duration_secs: None,
        }
    }

    /// Drives the real `process_hr_tick` every 5 s. No clock involved.
    fn simulate_hr_plant(plant: &mut Plant, initial_power: u16, minutes: u32) -> SimResult {
        let target = sim_target();
        let mut s = ControlLoopState::new();
        s.active = true;
        s.commanded_power = initial_power;
        s.ftp = Some(250); // band [50, 375]
        s.power_zones = None;
        let mut pid = new_hr_pid(initial_power);
        let mut smoother = HrSmoother::new(5);
        let mut peak_hr = 0u8;
        let mut ticks = Vec::new();
        for sec in 1..=(minutes as u64 * 60) {
            smoother.push(plant.step_second(s.commanded_power));
            if sec % 5 == 0 {
                if let Some(w) = process_hr_tick(&mut s, &target, &mut pid, &smoother, 5000) {
                    s.commanded_power = w;
                }
                let sm = smoother.smoothed().unwrap();
                peak_hr = peak_hr.max(sm);
                ticks.push((sec as f64, sm, s.commanded_power));
            }
        }
        SimResult { peak_hr, ticks }
    }

    fn assert_sim_passes(r: &SimResult, minutes: u32, label: &str) {
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

    fn assert_settles_without_overshoot(p: &PlantParams, label: &str) {
        // Steady start 20 bpm below the 135 target
        let p0 = steady_power(p, 115.0);
        let mut plant = Plant::new(p.clone(), 115.0, p0, None);
        let r = simulate_hr_plant(&mut plant, p0.round() as u16, 20);
        assert_sim_passes(&r, 20, label);
    }

    fn sim_grid() -> Vec<PlantParams> {
        let mut v = Vec::new();
        for tau_s in [30.0, 60.0, 90.0] {
            for delay_s in [5.0, 15.0, 25.0] {
                for gain in [0.25, 0.35] {
                    v.push(PlantParams { tau_s, delay_s, gain_bpm_per_w: gain, rest_bpm: 60.0 });
                }
            }
        }
        v
    }

    #[test]
    fn hr_plant_noisy_rising_start_settles_across_grid() {
        // HR 30 bpm below target and already rising (prior power drives toward
        // 125 bpm), with seeded ±3 bpm measurement noise. Same pass criteria.
        for (i, p) in sim_grid().into_iter().enumerate() {
            let prior = steady_power(&p, 125.0);
            let mut plant = Plant::new(p.clone(), 105.0, prior, Some(0x5eed_0000 + i as u64));
            let r = simulate_hr_plant(&mut plant, prior.round() as u16, 20);
            assert_sim_passes(
                &r,
                20,
                &format!("noisy rising tau={} delay={} gain={}", p.tau_s, p.delay_s, p.gain_bpm_per_w),
            );
        }
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

/// Task-level tests: real control task, FakeTrainer, paused tokio clock.
#[cfg(test)]
pub(crate) mod task_tests {
    use super::super::trainer::fake::{Behavior, FakeTrainer};
    use super::super::trainer::TrainerCmd;
    use super::*;
    use tokio::time::sleep;

    pub(crate) fn hr(bpm: u8) -> SensorReading {
        SensorReading::HeartRate { bpm, timestamp: None, epoch_ms: 0, device_id: String::new() }
    }

    pub(crate) fn cadence(rpm: f32) -> SensorReading {
        SensorReading::Cadence { rpm, timestamp: None, epoch_ms: 0, device_id: String::new() }
    }

    pub(crate) fn power(watts: u16) -> SensorReading {
        SensorReading::Power {
            watts,
            timestamp: None,
            epoch_ms: 0,
            device_id: String::new(),
            pedal_balance: None,
        }
    }

    pub(crate) fn power_target(lower: u16, upper: u16, duration_secs: Option<u64>) -> ZoneTarget {
        ZoneTarget { mode: ZoneMode::Power, zone: 0, lower_bound: lower, upper_bound: upper, duration_secs }
    }

    fn hr_target(lower: u16, upper: u16) -> ZoneTarget {
        ZoneTarget { mode: ZoneMode::HeartRate, zone: 0, lower_bound: lower, upper_bound: upper, duration_secs: None }
    }

    pub(crate) fn setup() -> (Arc<ZoneController<FakeTrainer>>, FakeTrainer, broadcast::Sender<SensorReading>) {
        let fake = FakeTrainer::new();
        let zc = Arc::new(ZoneController::new(fake.clone()));
        let (tx, _rx) = broadcast::channel(1024);
        (zc, fake, tx)
    }

    pub(crate) async fn start(
        zc: &ZoneController<FakeTrainer>,
        tx: &broadcast::Sender<SensorReading>,
        target: ZoneTarget,
        max_hr: Option<u8>,
    ) {
        zc.start_with_config(target, tx.clone(), Some(250), max_hr, None, None)
            .await
            .unwrap();
    }

    /// Feed `readings(t)` once per second for `secs` simulated seconds.
    pub(crate) async fn ride(
        tx: &broadcast::Sender<SensorReading>,
        secs: u64,
        readings: impl Fn(u64) -> Vec<SensorReading>,
    ) {
        for t in 0..secs {
            for r in readings(t) {
                let _ = tx.send(r);
            }
            sleep(Duration::from_secs(1)).await;
        }
    }

    fn steady(_: u64) -> Vec<SensorReading> {
        vec![cadence(90.0), power(200)]
    }

    fn hr_steady(bpm: u8) -> impl Fn(u64) -> Vec<SensorReading> {
        move |_| vec![cadence(90.0), hr(bpm)]
    }

    fn is_release_tail(cmds: &[TrainerCmd]) -> bool {
        cmds.ends_with(&[TrainerCmd::Power(0), TrainerCmd::Resistance(0)])
    }

    // --- AC6: stop releases the trainer ---

    #[tokio::test(start_paused = true)]
    async fn stop_releases_trainer_with_zero_watts_then_resistance() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 2, steady).await;
        assert_eq!(zc.stop(StopReason::UserStopped).await, Some(StopReason::UserStopped));
        let cmds = fake.commands();
        assert_eq!(cmds[0], TrainerCmd::Power(200));
        assert!(is_release_tail(&cmds), "{cmds:?}");
        let st = zc.status().await;
        assert!(!st.active);
        assert_eq!(st.phase, "stopped");
        assert_eq!(st.stop_reason, Some(StopReason::UserStopped));
    }

    #[tokio::test(start_paused = true)]
    async fn stop_when_inactive_is_noop() {
        let (zc, fake, _tx) = setup();
        assert_eq!(zc.stop(StopReason::UserStopped).await, None);
        assert!(fake.commands().is_empty());
        assert_eq!(zc.status().await.phase, "idle");
    }

    #[tokio::test(start_paused = true)]
    async fn second_stop_after_finish_returns_first_reason_without_commands() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        zc.stop(StopReason::SessionStopped).await;
        let n = fake.commands().len();
        assert_eq!(zc.stop(StopReason::AppClosing).await, Some(StopReason::SessionStopped));
        assert_eq!(fake.commands().len(), n);
    }

    // --- AC4: abnormal end ---

    #[tokio::test(start_paused = true)]
    async fn control_task_panic_reports_controller_fault_and_releases_trainer() {
        let (zc, fake, tx) = setup();
        fake.queue(Behavior::Panic); // the initial target command panics
        start(&zc, &tx, power_target(190, 210, None), None).await;
        sleep(Duration::from_millis(100)).await;
        let st = zc.status().await;
        assert!(!st.active);
        assert_eq!(st.stop_reason, Some(StopReason::ControllerFault));
        assert_eq!(fake.commands(), vec![TrainerCmd::Power(0), TrainerCmd::Resistance(0)]);
    }

    #[tokio::test(start_paused = true)]
    async fn self_stop_on_duration_complete_releases_trainer() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, Some(5)), None).await;
        ride(&tx, 8, steady).await;
        let st = zc.status().await;
        assert_eq!(st.stop_reason, Some(StopReason::DurationComplete));
        assert!(!st.active);
        assert!(is_release_tail(&fake.commands()));
    }

    #[tokio::test(start_paused = true)]
    async fn sensor_lost_stop_releases_trainer() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, hr_target(145, 155), None).await;
        ride(&tx, 40, |_| vec![cadence(90.0)]).await; // no HR readings at all
        let st = zc.status().await;
        assert_eq!(st.stop_reason, Some(StopReason::SensorLost));
        assert!(is_release_tail(&fake.commands()));
    }

    #[tokio::test(start_paused = true)]
    async fn finalize_panic_reports_controller_fault_to_stop() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 2, steady).await;
        fake.set_power_behavior(Behavior::Panic); // release's 0 W panics inside finalize
        assert_eq!(zc.stop(StopReason::UserStopped).await, Some(StopReason::ControllerFault));
        let st = zc.status().await;
        assert!(!st.active);
        assert_eq!(st.stop_reason, Some(StopReason::ControllerFault));
    }

    // --- AC5: re-send after cadence cut and reconnect ---

    fn first_power_at_or_after(fake: &FakeTrainer, w: u16, t: Instant) -> Option<Instant> {
        fake.log()
            .into_iter()
            .find(|(c, at)| *c == TrainerCmd::Power(w) && *at >= t)
            .map(|(_, at)| at)
    }

    #[tokio::test(start_paused = true)]
    async fn power_mode_restores_target_within_2_ticks_after_cadence_sustained_2s() {
        // Amended AC5: restore within 2 ticks once cadence ≥ 40 rpm has been sustained 2 s.
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 3, steady).await;
        ride(&tx, 6, |_| vec![cadence(0.0), power(0)]).await;
        assert!(fake.commands().contains(&TrainerCmd::Power(0)), "cadence cut not sent");
        let back = Instant::now();
        ride(&tx, 6, steady).await;
        let at = first_power_at_or_after(&fake, 200, back).expect("target never restored");
        assert!(at - back >= Duration::from_secs(2), "restored after only {:?}", at - back);
        assert!(at - back <= Duration::from_secs(4), "restored after {:?}", at - back);
    }

    #[tokio::test(start_paused = true)]
    async fn power_mode_keeps_cut_while_cadence_below_40rpm() {
        // Low cadence under ERG is the "spiral of death": stay at 0 W.
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 3, steady).await;
        ride(&tx, 6, |_| vec![cadence(0.0)]).await;
        let cut = Instant::now();
        ride(&tx, 10, |_| vec![cadence(30.0)]).await;
        assert_eq!(first_power_at_or_after(&fake, 200, cut), None, "restored at 30 rpm");
        // A 1 s burst at 90 rpm is not sustained
        ride(&tx, 1, |_| vec![cadence(90.0)]).await;
        ride(&tx, 3, |_| vec![cadence(30.0)]).await;
        assert_eq!(first_power_at_or_after(&fake, 200, cut), None, "restored after 1 s burst");
    }

    #[tokio::test(start_paused = true)]
    async fn hung_safety_cut_stops_within_6s_wall_clock() {
        // Each send blocks to its 3 s timeout, so 3 ticks would take ~9-10 s;
        // the 6 s wall-clock bound must stop the ride first.
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 3, steady).await;
        // Every 0 W send blocks exactly to the 3 s command timeout
        fake.set_power_behavior(Behavior::Hang(TRAINER_COMMAND_TIMEOUT));
        ride(&tx, 20, |_| vec![cadence(0.0)]).await;
        assert_eq!(zc.status().await.stop_reason, Some(StopReason::TrainerUnresponsive));
        let zero_attempts: Vec<Instant> = fake
            .attempts()
            .into_iter()
            .filter(|(c, _)| *c == TrainerCmd::Power(0))
            .map(|(_, at)| at)
            .collect();
        let first_cut = zero_attempts[0];
        let release = *zero_attempts.last().unwrap(); // finalize's 0 W
        assert!(
            release - first_cut <= Duration::from_secs(6),
            "stopped {:?} after the first undelivered cut",
            release - first_cut
        );
    }

    #[tokio::test(start_paused = true)]
    async fn target_resent_within_2_ticks_after_trainer_reconnect() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 3, steady).await;
        fake.set_connected(false);
        ride(&tx, 5, steady).await;
        assert_eq!(zc.status().await.phase, "trainer_lost");
        fake.set_connected(true);
        let back = Instant::now();
        ride(&tx, 4, steady).await;
        let at = first_power_at_or_after(&fake, 200, back).expect("target never re-sent");
        assert!(at - back <= Duration::from_secs(2), "re-sent after {:?}", at - back);
        assert!(zc.status().await.active);
    }

    #[tokio::test(start_paused = true)]
    async fn trainer_lost_without_override_waits_30s() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 2, steady).await;
        fake.set_connected(false);
        ride(&tx, 25, steady).await;
        assert!(zc.status().await.active, "stopped before the 30 s grace");
        ride(&tx, 10, steady).await;
        let st = zc.status().await;
        assert!(!st.active);
        assert_eq!(st.stop_reason, Some(StopReason::TrainerDisconnected));
    }

    // --- ADR-15: bounded safety-cut grace ---

    #[tokio::test(start_paused = true)]
    async fn undelivered_cadence_cut_for_3_ticks_stops_trainer_unresponsive() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 3, steady).await;
        fake.set_power_behavior(Behavior::Err);
        ride(&tx, 10, |_| vec![cadence(0.0)]).await;
        let st = zc.status().await;
        assert!(!st.active);
        assert_eq!(st.stop_reason, Some(StopReason::TrainerUnresponsive));
    }

    #[tokio::test(start_paused = true)]
    async fn undelivered_hr_ceiling_cut_for_3_ticks_stops_trainer_unresponsive() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, hr_target(145, 155), Some(170)).await;
        ride(&tx, 10, hr_steady(150)).await;
        fake.set_power_behavior(Behavior::Err);
        ride(&tx, 25, hr_steady(185)).await;
        let st = zc.status().await;
        assert!(!st.active);
        assert_eq!(st.stop_reason, Some(StopReason::TrainerUnresponsive));
    }

    // --- AC11 / ADR-15: HR ceiling on smoothed HR with hysteresis ---

    fn test_loop(fake: &FakeTrainer, window: usize) -> ControlLoop<FakeTrainer> {
        let mut st = ControlLoopState::new();
        st.active = true;
        st.target = Some(hr_target(145, 155));
        let (tx, _) = broadcast::channel(16);
        let mut lp = ControlLoop::new(
            1,
            Arc::new(Mutex::new(st)),
            hr_target(145, 155),
            Arc::new(SerializedSink::new(fake.clone())),
            tx,
            137,
        );
        lp.smoother = HrSmoother::new(window);
        lp
    }

    fn sample(lp: &mut ControlLoop<FakeTrainer>, bpm: u8) {
        lp.smoother.push(bpm);
        lp.update_hr_ceiling(Some(170));
    }

    #[test]
    fn hr_ceiling_clears_only_after_two_samples_below_ceiling_minus_3() {
        let mut lp = test_loop(&FakeTrainer::new(), 1);
        sample(&mut lp, 175);
        assert!(lp.hr_ceiling_override);
        sample(&mut lp, 166);
        assert!(lp.hr_ceiling_override, "one sample below 167 must not clear");
        sample(&mut lp, 166);
        assert!(!lp.hr_ceiling_override);
    }

    #[test]
    fn single_sample_below_clear_threshold_keeps_override() {
        let mut lp = test_loop(&FakeTrainer::new(), 1);
        sample(&mut lp, 175);
        sample(&mut lp, 166);
        sample(&mut lp, 167); // not < 170−3 → counter resets
        sample(&mut lp, 166);
        assert!(lp.hr_ceiling_override);
        sample(&mut lp, 160);
        assert!(!lp.hr_ceiling_override);
    }

    #[test]
    fn single_hr_spike_above_max_does_not_cut_power() {
        // Median-of-5 smoothing: one 200 bpm spike among 150s never exceeds 170.
        let mut lp = test_loop(&FakeTrainer::new(), 5);
        for bpm in [150, 150, 200, 150, 150, 150] {
            sample(&mut lp, bpm);
            assert!(!lp.hr_ceiling_override, "spike tripped the ceiling");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn sustained_hr_above_max_cuts_to_safety_power() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, hr_target(145, 155), Some(170)).await;
        ride(&tx, 10, hr_steady(150)).await;
        ride(&tx, 10, hr_steady(180)).await;
        assert!(fake.commands().contains(&TrainerCmd::Power(SAFETY_POWER)));
        assert_eq!(zc.status().await.safety_note.as_deref(), Some("HR ceiling exceeded"));
    }

    // --- ADR-14: freeze and re-seed ---

    fn power_cmds_after(fake: &FakeTrainer, idx: usize) -> Vec<u16> {
        fake.commands()[idx..]
            .iter()
            .filter_map(|c| match c {
                TrainerCmd::Power(w) => Some(*w),
                _ => None,
            })
            .collect()
    }

    fn last_index_of(fake: &FakeTrainer, cmd: TrainerCmd) -> usize {
        fake.commands().iter().rposition(|c| *c == cmd).expect("command not found")
    }

    #[tokio::test(start_paused = true)]
    async fn hr_ceiling_override_30s_first_post_override_command_at_most_delivered_plus_max_up() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, hr_target(145, 155), Some(170)).await;
        ride(&tx, 20, hr_steady(140)).await;
        ride(&tx, 30, hr_steady(180)).await;
        ride(&tx, 20, hr_steady(140)).await;
        let idx = last_index_of(&fake, TrainerCmd::Power(SAFETY_POWER));
        let after = power_cmds_after(&fake, idx + 1);
        assert!(!after.is_empty(), "control never resumed after override");
        assert!(after[0] <= SAFETY_POWER + 20, "jumped to {} W after override", after[0]);
        assert!(after[0] > SAFETY_POWER);
    }

    #[tokio::test(start_paused = true)]
    async fn hr_mode_after_cadence_zero_ramps_up_from_zero() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, hr_target(145, 155), None).await;
        ride(&tx, 15, hr_steady(140)).await;
        ride(&tx, 10, |_| vec![cadence(0.0), hr(140)]).await;
        ride(&tx, 21, hr_steady(140)).await;
        let idx = last_index_of(&fake, TrainerCmd::Power(0));
        let after = power_cmds_after(&fake, idx + 1);
        assert!(after.len() >= 3, "{after:?}");
        assert!(after[0] <= 20, "first post-cut command {} W", after[0]);
        let mut prev = 0;
        for w in after {
            assert!(w <= prev + 20, "step {prev} → {w} exceeds ramp limit");
            prev = w;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn pid_state_unchanged_during_override() {
        let fake = FakeTrainer::new();
        let mut lp = test_loop(&fake, 5);
        {
            let mut s = lp.state.lock().await;
            s.last_tick_at = Some(Instant::now());
            s.last_hr_seen = Some(Instant::now());
            s.started_at = Some(Instant::now());
        }
        for _ in 0..5 {
            lp.smoother.push(140);
        }
        for _ in 0..3 {
            sleep(Duration::from_secs(5)).await;
            lp.state.lock().await.last_hr_seen = Some(Instant::now());
            assert_eq!(lp.on_tick().await, None);
        }
        let (i0, u0) = (lp.pid.integral(), lp.pid.last_output());
        assert!(i0 > 0.0);
        lp.state.lock().await.last_cadence_zero_since = Some(Instant::now());
        for _ in 0..6 {
            sleep(Duration::from_secs(5)).await;
            lp.state.lock().await.last_hr_seen = Some(Instant::now());
            assert_eq!(lp.on_tick().await, None);
        }
        assert_eq!(lp.pid.integral(), i0);
        assert_eq!(lp.pid.last_output(), u0);
    }

    // --- AC6 / ADR-13 / ADR-22: pause ---

    #[tokio::test(start_paused = true)]
    async fn pause_releases_trainer_and_resume_resends_target() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 3, steady).await;
        zc.pause().await;
        sleep(Duration::from_millis(100)).await;
        assert!(is_release_tail(&fake.commands()), "{:?}", fake.commands());
        let st = zc.status().await;
        assert!(st.active && st.paused);
        assert_eq!(st.phase, "paused");
        zc.resume().await;
        let back = Instant::now();
        ride(&tx, 3, steady).await;
        let at = first_power_at_or_after(&fake, 200, back).expect("target not re-sent");
        assert!(at - back <= Duration::from_secs(2));
    }

    #[tokio::test(start_paused = true)]
    async fn failed_pause_release_is_retried_next_tick() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 2, steady).await;
        fake.set_power_behavior(Behavior::Err);
        zc.pause().await;
        ride(&tx, 3, steady).await;
        assert!(!fake.commands().contains(&TrainerCmd::Power(0)));
        fake.set_power_behavior(Behavior::Ok);
        ride(&tx, 2, steady).await;
        assert!(fake.commands().contains(&TrainerCmd::Power(0)), "release never retried");
        assert!(zc.status().await.paused);
    }

    #[tokio::test(start_paused = true)]
    async fn paused_trainer_lost_30s_stops_trainer_disconnected() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 2, steady).await;
        fake.set_power_behavior(Behavior::Err);
        zc.pause().await;
        ride(&tx, 35, steady).await;
        assert_eq!(zc.status().await.stop_reason, Some(StopReason::TrainerDisconnected));
    }

    #[tokio::test(start_paused = true)]
    async fn paused_with_cadence_zero_sends_release_and_never_trainer_unresponsive() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 2, steady).await;
        fake.set_power_behavior(Behavior::Err);
        zc.pause().await;
        ride(&tx, 10, |_| vec![cadence(0.0)]).await;
        assert!(zc.status().await.active, "paused ride stopped by safety counter");
        ride(&tx, 25, |_| vec![cadence(0.0)]).await;
        assert_eq!(zc.status().await.stop_reason, Some(StopReason::TrainerDisconnected));
    }

    // --- AC8: pause accounting ---

    #[tokio::test(start_paused = true)]
    async fn pause_10_minutes_adds_zero_time_in_zone() {
        let (zc, _fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 5, steady).await;
        zc.pause().await;
        sleep(Duration::from_millis(100)).await; // pause is applied by the control task
        let before = zc.status().await.time_in_zone_secs;
        ride(&tx, 600, steady).await;
        assert_eq!(zc.status().await.time_in_zone_secs, before);
    }

    #[tokio::test(start_paused = true)]
    async fn first_tick_after_resume_uses_normal_dt() {
        let (zc, _fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 5, steady).await;
        zc.pause().await;
        ride(&tx, 600, steady).await;
        let before = zc.status().await.time_in_zone_secs;
        zc.resume().await;
        ride(&tx, 2, steady).await;
        let after = zc.status().await.time_in_zone_secs;
        assert!(after - before <= 2, "first post-resume tick counted {} s", after - before);
    }

    // --- AC10 / ADR-21: lock scope and status during finalize ---

    #[tokio::test(start_paused = true)]
    async fn status_responsive_while_stop_awaits_task() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 2, steady).await;
        fake.set_power_behavior(Behavior::Hang(Duration::from_secs(2)));
        let zc2 = zc.clone();
        let stopper = tokio::spawn(async move { zc2.stop(StopReason::UserStopped).await });
        sleep(Duration::from_millis(500)).await;
        let st = tokio::time::timeout(Duration::from_millis(10), zc.status())
            .await
            .expect("status blocked by stop");
        assert!(st.active, "status must stay active until release completes");
        assert_eq!(stopper.await.unwrap(), Some(StopReason::UserStopped));
        assert!(!zc.status().await.active);
    }

    // --- ADR-12: generation-scoped runs ---

    #[tokio::test(start_paused = true)]
    async fn two_concurrent_stops_both_return_after_release() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 2, steady).await;
        fake.set_power_behavior(Behavior::Hang(Duration::from_secs(2)));
        let t0 = Instant::now();
        let (a, b) = (zc.clone(), zc.clone());
        let s1 = tokio::spawn(async move { (a.stop(StopReason::UserStopped).await, Instant::now()) });
        let s2 = tokio::spawn(async move { (b.stop(StopReason::SessionStopped).await, Instant::now()) });
        let (r1, t1) = s1.await.unwrap();
        let (r2, t2) = s2.await.unwrap();
        let released_at = fake
            .log()
            .into_iter()
            .find(|(c, _)| *c == TrainerCmd::Power(0))
            .map(|(_, at)| at)
            .expect("release not sent");
        assert!(released_at >= t0 + Duration::from_secs(2));
        assert!(t1 >= released_at && t2 >= released_at);
        assert_eq!(r1, r2, "both callers report the same (first) reason");
    }

    #[tokio::test(start_paused = true)]
    async fn start_during_in_flight_stop_waits_for_release() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 2, steady).await;
        fake.queue(Behavior::Hang(Duration::from_secs(2))); // A's release 0 W
        let a = zc.clone();
        let stopper = tokio::spawn(async move { a.stop(StopReason::UserStopped).await });
        sleep(Duration::from_millis(10)).await;
        start(&zc, &tx, power_target(300, 320, None), None).await;
        stopper.await.unwrap();
        sleep(Duration::from_millis(100)).await; // let B's task send its initial target
        let cmds = fake.commands();
        let rel = cmds.iter().position(|c| *c == TrainerCmd::Power(0)).unwrap();
        let b_init = cmds.iter().position(|c| *c == TrainerCmd::Power(310)).unwrap();
        assert!(rel < b_init, "{cmds:?}");
        let st = zc.status().await;
        assert!(st.active);
        assert_eq!(st.lower_bound, Some(300));
    }

    #[tokio::test(start_paused = true)]
    async fn stop_then_start_old_generation_never_commands_after_new_initial() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 2, steady).await;
        zc.stop(StopReason::UserStopped).await;
        start(&zc, &tx, power_target(300, 320, None), None).await;
        ride(&tx, 15, steady).await;
        let cmds = fake.commands();
        let b_init = cmds.iter().position(|c| *c == TrainerCmd::Power(310)).unwrap();
        assert!(!cmds[b_init..].contains(&TrainerCmd::Power(200)), "{cmds:?}");
        assert!(!cmds[b_init..].contains(&TrainerCmd::Power(0)), "old release after new run: {cmds:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn self_finalize_racing_start_does_not_touch_new_run_status() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, Some(2)), None).await;
        fake.set_power_behavior(Behavior::Hang(Duration::from_secs(3)));
        // A completes at ~2 s and is releasing (hang) when B starts
        ride(&tx, 3, steady).await;
        fake.set_power_behavior(Behavior::Ok);
        start(&zc, &tx, power_target(300, 320, None), None).await;
        ride(&tx, 5, steady).await;
        let st = zc.status().await;
        assert!(st.active);
        assert_eq!(st.stop_reason, None);
        assert_eq!(st.lower_bound, Some(300));
    }

    // --- AC1 through the full control-task tick path ---

    #[tokio::test(start_paused = true)]
    async fn hr_plant_noisy_rising_start_settles_through_on_tick() {
        use super::tests::{steady_power, Plant, PlantParams};
        let p = PlantParams { tau_s: 60.0, delay_s: 15.0, gain_bpm_per_w: 0.35, rest_bpm: 60.0 };
        let prior = steady_power(&p, 125.0);
        let initial = prior.round() as u16;
        let mut plant = Plant::new(p, 105.0, prior, Some(0xC0FFEE));

        let fake = FakeTrainer::new();
        let target = ZoneTarget {
            mode: ZoneMode::HeartRate,
            zone: 0,
            lower_bound: 130,
            upper_bound: 140,
            duration_secs: None,
        };
        let now = Instant::now();
        let mut st = ControlLoopState::new();
        st.active = true;
        st.target = Some(target.clone());
        st.commanded_power = initial;
        st.started_at = Some(now);
        st.last_tick_at = Some(now);
        st.last_hr_seen = Some(now);
        st.ftp = Some(250);
        let (tx, _) = broadcast::channel(16);
        let mut lp = ControlLoop::new(
            1,
            Arc::new(Mutex::new(st)),
            target,
            Arc::new(SerializedSink::new(fake.clone())),
            tx,
            initial,
        );
        assert_eq!(lp.step(Step::Initial).await, None);

        let minutes = 20u64;
        let mut ticks: Vec<(u64, u8, u16)> = Vec::new();
        let mut peak = 0u8;
        for sec in 1..=minutes * 60 {
            let delivered = match lp.delivered {
                Delivered::Watts(w) => w,
                _ => 0,
            };
            let sample = plant.step_second(delivered);
            lp.on_reading(hr(sample)).await;
            lp.on_reading(cadence(90.0)).await;
            tokio::time::advance(Duration::from_secs(1)).await;
            if sec % 5 == 0 {
                assert_eq!(lp.on_tick().await, None, "control stopped at t={sec}s");
                let sm = lp.smoother.smoothed().unwrap();
                peak = peak.max(sm);
                ticks.push((sec, sm, lp.state.lock().await.commanded_power));
            }
        }
        assert!(peak <= 140, "overshoot: peak smoothed HR {peak}");
        for &(t, h, _) in ticks.iter().filter(|x| x.0 > minutes * 60 - 600) {
            assert!((130..=140).contains(&h), "HR {h} outside band at t={t}s");
        }
        let last: Vec<u16> = ticks.iter().filter(|x| x.0 > minutes * 60 - 300).map(|x| x.2).collect();
        let swing = last.iter().max().unwrap() - last.iter().min().unwrap();
        assert!(swing <= 10, "power swing {swing} W in last 5 min");
        // Every tick's command really reached the (fake) trainer
        assert_eq!(fake.commands().last(), Some(&TrainerCmd::Power(*last.last().unwrap())));
    }

    // --- ADR-26: closing flag ---

    #[tokio::test(start_paused = true)]
    async fn start_racing_app_close_never_leaves_a_run_sending() {
        for start_first in [true, false] {
            let (zc, fake, tx) = setup();
            start(&zc, &tx, power_target(190, 210, None), None).await;
            ride(&tx, 2, steady).await;
            let (z1, z2, tx1) = (zc.clone(), zc.clone(), tx.clone());
            let starter = async move {
                z1.start_with_config(power_target(300, 320, None), tx1, Some(250), None, None, None)
                    .await
            };
            let closer = async move { z2.close(APP_CLOSE_RELEASE_BUDGET).await };
            let (started, closed) = if start_first {
                let s = tokio::spawn(starter);
                tokio::task::yield_now().await;
                let c = tokio::spawn(closer);
                (s.await.unwrap(), c.await.unwrap())
            } else {
                let c = tokio::spawn(closer);
                tokio::task::yield_now().await;
                let s = tokio::spawn(starter);
                (s.await.unwrap(), c.await.unwrap())
            };
            assert!(closed.is_some());
            if !start_first {
                assert!(started.unwrap_err().to_string().contains("App is closing"));
            }
            let n = fake.attempts().len();
            ride(&tx, 15, steady).await;
            assert_eq!(fake.attempts().len(), n, "a run kept commanding after close (start_first={start_first})");
            assert!(!zc.status().await.active);
            let err = zc
                .start_with_config(power_target(300, 320, None), tx.clone(), Some(250), None, None, None)
                .await
                .unwrap_err();
            assert!(err.to_string().contains("App is closing"), "{err}");
        }
    }

    // --- ruling 6: manual commands and pause during start ---

    #[tokio::test(start_paused = true)]
    async fn manual_trainer_commands_refused_while_zone_run_active() {
        let (zc, _fake, tx) = setup();
        assert!(zc.ensure_manual_control_allowed().is_ok());
        start(&zc, &tx, power_target(190, 210, None), None).await;
        let err = zc.ensure_manual_control_allowed().unwrap_err().to_string();
        assert!(err.contains("Zone ride active — stop it first"), "{err}");
        zc.pause().await;
        assert!(zc.ensure_manual_control_allowed().is_err(), "paused run still owns the trainer");
        zc.stop(StopReason::UserStopped).await;
        assert!(zc.ensure_manual_control_allowed().is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn pause_during_start_applies_to_new_run() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 2, steady).await;
        fake.queue(Behavior::Hang(Duration::from_secs(2))); // run A's release, inside B's start
        let (z1, tx1) = (zc.clone(), tx.clone());
        let starter = tokio::spawn(async move { start(&z1, &tx1, power_target(300, 320, None), None).await });
        sleep(Duration::from_millis(10)).await;
        zc.pause().await;
        starter.await.unwrap();
        sleep(Duration::from_millis(100)).await;
        let st = zc.status().await;
        assert_eq!(st.lower_bound, Some(300));
        assert!(st.active && st.paused, "pause lost during start: {st:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn stop_after_finish_returns_the_original_reason() {
        // Documented contract: a stop after the run ended reports why it ended.
        let (zc, _fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, Some(2)), None).await;
        ride(&tx, 4, steady).await;
        assert_eq!(zc.stop(StopReason::UserStopped).await, Some(StopReason::DurationComplete));
    }

    // --- ADR-19: hard budget ---

    #[tokio::test(start_paused = true)]
    async fn stop_with_budget_returns_at_budget_when_release_hangs() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 2, steady).await;
        fake.set_power_behavior(Behavior::Hang(Duration::from_secs(60)));
        fake.set_resistance_behavior(Behavior::Hang(Duration::from_secs(60)));
        let t0 = Instant::now();
        zc.stop_with_budget(StopReason::AppClosing, APP_CLOSE_RELEASE_BUDGET).await;
        assert!(t0.elapsed() <= APP_CLOSE_RELEASE_BUDGET, "took {:?}", t0.elapsed());
    }
}
