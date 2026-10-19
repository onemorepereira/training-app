//! Trainer I/O for zone control (design ADR-11, ADR-12, ADR-17).
//!
//! All controller-issued commands go through `SerializedSink`, which:
//! - runs each command in a spawned task holding a command mutex, so a command
//!   that timed out (including an FE-C `spawn_blocking` call) still finishes
//!   before the next controller command starts;
//! - bounds every command with a timeout as seen by the caller;
//! - drops commands from a stale run generation (generation fence).

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use log::{error, info, warn};
use tokio::sync::Mutex;

use crate::config;
use crate::device::manager::DeviceManager;
use crate::error::AppError;

/// Per-command timeout as seen by the controller (ruling: 3 s).
pub const TRAINER_COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
/// Default total budget for a release outside app close: 0 W + resistance step.
pub const DEFAULT_RELEASE_BUDGET: Duration = Duration::from_secs(6);
/// Below this remaining budget the best-effort resistance step is skipped.
const MIN_RESISTANCE_STEP_BUDGET: Duration = Duration::from_millis(250);
/// Resistance level used as the safe state (0 % = minimum brake).
pub const SAFE_RESISTANCE_LEVEL: u8 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainerCmd {
    Power(u16),
    Resistance(u8),
}

/// Raw trainer calls, without timeout or serialization.
pub trait TrainerBackend: Send + Sync + 'static {
    fn is_connected(&self) -> impl Future<Output = bool> + Send;
    fn send(&self, cmd: TrainerCmd) -> impl Future<Output = Result<(), AppError>> + Send;
}

/// Production backend: the shared DeviceManager.
pub struct DeviceManagerBackend {
    dm: Arc<Mutex<DeviceManager>>,
}

impl DeviceManagerBackend {
    pub fn new(dm: Arc<Mutex<DeviceManager>>) -> Self {
        Self { dm }
    }
}

impl TrainerBackend for DeviceManagerBackend {
    async fn is_connected(&self) -> bool {
        self.dm.lock().await.connected_trainer_id().is_some()
    }

    async fn send(&self, cmd: TrainerCmd) -> Result<(), AppError> {
        let mut dm = self.dm.lock().await;
        let id = dm
            .connected_trainer_id()
            .ok_or_else(|| AppError::Session("Trainer disconnected".into()))?;
        match cmd {
            TrainerCmd::Power(w) => dm.set_target_power(&id, config::clamp_trainer_watts(w as i32)).await,
            TrainerCmd::Resistance(level) => dm.set_resistance(&id, level).await,
        }
    }
}

#[derive(Debug)]
pub struct ReleaseOutcome {
    /// True iff the 0 W command succeeded (ruling 1).
    pub delivered: bool,
    pub power: Result<(), String>,
    /// None when skipped for lack of budget.
    pub resistance: Option<Result<(), String>>,
}

pub struct SerializedSink<B: TrainerBackend> {
    backend: Arc<B>,
    cmd_lock: Arc<Mutex<()>>,
    active_gen: Arc<AtomicU64>,
}

impl<B: TrainerBackend> SerializedSink<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend: Arc::new(backend),
            cmd_lock: Arc::new(Mutex::new(())),
            active_gen: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Mark `gen` as the current run; commands from older generations are dropped.
    pub fn set_active_gen(&self, gen: u64) {
        self.active_gen.store(gen, Ordering::SeqCst);
    }

    /// Whether a trainer is connected; false if the check itself takes longer
    /// than the command timeout (e.g. DeviceManager lock held by a scan).
    pub async fn is_connected(&self) -> bool {
        tokio::time::timeout(TRAINER_COMMAND_TIMEOUT, self.backend.is_connected())
            .await
            .unwrap_or(false)
    }

    /// Send one command for run `gen`, bounded by `limit`. A panic inside the
    /// backend is re-raised in the caller so the control task's supervisor sees it.
    pub async fn send(&self, gen: u64, cmd: TrainerCmd, limit: Duration) -> Result<(), AppError> {
        let backend = self.backend.clone();
        let lock = self.cmd_lock.clone();
        let active = self.active_gen.clone();
        let task = tokio::spawn(async move {
            let _guard = lock.lock_owned().await;
            if gen < active.load(Ordering::SeqCst) {
                return Err(AppError::Session(format!(
                    "Dropped stale trainer command {cmd:?} from run {gen}"
                )));
            }
            backend.send(cmd).await
        });
        match tokio::time::timeout(limit, task).await {
            Ok(Ok(result)) => result,
            Ok(Err(join_err)) => match join_err.try_into_panic() {
                Ok(payload) => std::panic::resume_unwind(payload),
                Err(e) => Err(AppError::Session(format!("Trainer command task failed: {e}"))),
            },
            Err(_) => Err(AppError::Session(format!(
                "Trainer command {cmd:?} timed out after {limit:?}"
            ))),
        }
    }

    /// Put the trainer in the safe state (ADR-11): 0 W first, then resistance 0 %
    /// best effort, all within `budget`. Delivered iff the 0 W command succeeded.
    pub async fn release(&self, gen: u64, budget: Duration) -> ReleaseOutcome {
        let deadline = tokio::time::Instant::now() + budget;
        let power = self
            .send(gen, TrainerCmd::Power(0), budget.min(TRAINER_COMMAND_TIMEOUT))
            .await
            .map_err(|e| e.to_string());
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let resistance = if remaining >= MIN_RESISTANCE_STEP_BUDGET {
            Some(
                self.send(
                    gen,
                    TrainerCmd::Resistance(SAFE_RESISTANCE_LEVEL),
                    remaining.min(TRAINER_COMMAND_TIMEOUT),
                )
                .await
                .map_err(|e| e.to_string()),
            )
        } else {
            None
        };
        ReleaseOutcome { delivered: power.is_ok(), power, resistance }
    }
}

/// Log a release outcome with the reason control ended (ADR-11).
pub fn log_release(context: &str, outcome: &ReleaseOutcome) {
    let resistance = match &outcome.resistance {
        Some(Ok(())) => "ok".to_string(),
        Some(Err(e)) => format!("failed: {e}"),
        None => "skipped (no budget)".to_string(),
    };
    if outcome.delivered {
        info!("{context}: trainer released (0 W ok, resistance 0 %: {resistance})");
    } else {
        let power_err = outcome.power.as_ref().err().cloned().unwrap_or_default();
        error!("{context}: trainer release NOT confirmed (0 W failed: {power_err}; resistance 0 %: {resistance})");
    }
    if outcome.delivered && matches!(outcome.resistance, Some(Err(_))) {
        warn!("{context}: trainer may still be in ERG mode at 0 W");
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex as StdMutex;

    #[derive(Debug, Clone, Copy)]
    pub enum Behavior {
        Ok,
        Err,
        Hang(Duration),
        Panic,
    }

    struct Inner {
        connected: bool,
        default_power: Behavior,
        default_resistance: Behavior,
        /// One-shot behaviors consumed by the next command (any kind)
        queued: VecDeque<Behavior>,
        log: Vec<(TrainerCmd, tokio::time::Instant)>,
    }

    /// Scriptable in-memory trainer; records every command it *executes*.
    #[derive(Clone)]
    pub struct FakeTrainer {
        inner: Arc<StdMutex<Inner>>,
    }

    impl FakeTrainer {
        pub fn new() -> Self {
            Self {
                inner: Arc::new(StdMutex::new(Inner {
                    connected: true,
                    default_power: Behavior::Ok,
                    default_resistance: Behavior::Ok,
                    queued: VecDeque::new(),
                    log: Vec::new(),
                })),
            }
        }

        pub fn set_connected(&self, c: bool) {
            self.inner.lock().unwrap().connected = c;
        }

        pub fn set_power_behavior(&self, b: Behavior) {
            self.inner.lock().unwrap().default_power = b;
        }

        pub fn set_resistance_behavior(&self, b: Behavior) {
            self.inner.lock().unwrap().default_resistance = b;
        }

        pub fn queue(&self, b: Behavior) {
            self.inner.lock().unwrap().queued.push_back(b);
        }

        pub fn commands(&self) -> Vec<TrainerCmd> {
            self.inner.lock().unwrap().log.iter().map(|(c, _)| *c).collect()
        }

        pub fn log(&self) -> Vec<(TrainerCmd, tokio::time::Instant)> {
            self.inner.lock().unwrap().log.clone()
        }

        pub fn clear(&self) {
            self.inner.lock().unwrap().log.clear();
        }
    }

    impl TrainerBackend for FakeTrainer {
        async fn is_connected(&self) -> bool {
            self.inner.lock().unwrap().connected
        }

        async fn send(&self, cmd: TrainerCmd) -> Result<(), AppError> {
            let behavior = {
                let mut inner = self.inner.lock().unwrap();
                if !inner.connected {
                    return Err(AppError::Session("Trainer disconnected".into()));
                }
                let b = inner.queued.pop_front().unwrap_or(match cmd {
                    TrainerCmd::Power(_) => inner.default_power,
                    TrainerCmd::Resistance(_) => inner.default_resistance,
                });
                b
            };
            match behavior {
                Behavior::Ok => {}
                Behavior::Err => return Err(AppError::Session("fake trainer rejected".into())),
                Behavior::Hang(d) => tokio::time::sleep(d).await,
                Behavior::Panic => panic!("fake trainer panic on {cmd:?}"),
            }
            self.inner
                .lock()
                .unwrap()
                .log
                .push((cmd, tokio::time::Instant::now()));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{Behavior, FakeTrainer};
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn release_sends_zero_watts_before_resistance_zero() {
        let fake = FakeTrainer::new();
        let sink = SerializedSink::new(fake.clone());
        let out = sink.release(1, DEFAULT_RELEASE_BUDGET).await;
        assert!(out.delivered);
        assert_eq!(fake.commands(), vec![TrainerCmd::Power(0), TrainerCmd::Resistance(0)]);
    }

    #[tokio::test(start_paused = true)]
    async fn release_delivered_iff_zero_watts_ok() {
        // Resistance rejected (FTMS 0x02) but 0 W ok → delivered
        let fake = FakeTrainer::new();
        fake.set_resistance_behavior(Behavior::Err);
        let sink = SerializedSink::new(fake.clone());
        let out = sink.release(1, DEFAULT_RELEASE_BUDGET).await;
        assert!(out.delivered);
        assert!(matches!(out.resistance, Some(Err(_))));
    }

    #[tokio::test(start_paused = true)]
    async fn release_not_confirmed_when_zero_watts_fails_even_if_resistance_ok() {
        let fake = FakeTrainer::new();
        fake.set_power_behavior(Behavior::Err);
        let sink = SerializedSink::new(fake.clone());
        let out = sink.release(1, DEFAULT_RELEASE_BUDGET).await;
        assert!(!out.delivered);
        assert!(matches!(out.resistance, Some(Ok(()))));
        assert_eq!(fake.commands(), vec![TrainerCmd::Resistance(0)]);
    }

    #[tokio::test(start_paused = true)]
    async fn hung_trainer_command_times_out_within_3s_and_counts_as_failure() {
        let fake = FakeTrainer::new();
        fake.set_power_behavior(Behavior::Hang(Duration::from_secs(60)));
        let sink = SerializedSink::new(fake.clone());
        let start = tokio::time::Instant::now();
        let r = sink.send(1, TrainerCmd::Power(200), TRAINER_COMMAND_TIMEOUT).await;
        assert!(r.is_err());
        assert_eq!(start.elapsed(), Duration::from_secs(3));
    }

    #[tokio::test(start_paused = true)]
    async fn next_command_waits_for_timed_out_previous_command() {
        // First command hangs 5 s (times out at 3 s); the second must not
        // reach the trainer before the first finishes at t=5 s.
        let fake = FakeTrainer::new();
        fake.queue(Behavior::Hang(Duration::from_secs(5)));
        let sink = SerializedSink::new(fake.clone());
        let t0 = tokio::time::Instant::now();
        assert!(sink.send(1, TrainerCmd::Power(200), TRAINER_COMMAND_TIMEOUT).await.is_err());
        assert!(sink.send(1, TrainerCmd::Power(210), TRAINER_COMMAND_TIMEOUT).await.is_ok());
        let log = fake.log();
        assert_eq!(log[0].0, TrainerCmd::Power(200));
        assert_eq!(log[0].1 - t0, Duration::from_secs(5));
        assert_eq!(log[1].0, TrainerCmd::Power(210));
        assert!(log[1].1 >= log[0].1);
    }

    #[tokio::test(start_paused = true)]
    async fn stale_generation_command_dropped_by_sink() {
        let fake = FakeTrainer::new();
        let sink = SerializedSink::new(fake.clone());
        sink.set_active_gen(2);
        assert!(sink.send(1, TrainerCmd::Power(300), TRAINER_COMMAND_TIMEOUT).await.is_err());
        assert!(sink.send(2, TrainerCmd::Power(150), TRAINER_COMMAND_TIMEOUT).await.is_ok());
        assert_eq!(fake.commands(), vec![TrainerCmd::Power(150)]);
    }

    #[tokio::test(start_paused = true)]
    async fn release_respects_budget_when_trainer_hangs() {
        let fake = FakeTrainer::new();
        fake.set_power_behavior(Behavior::Hang(Duration::from_secs(60)));
        let sink = SerializedSink::new(fake.clone());
        let t0 = tokio::time::Instant::now();
        let out = sink.release(1, Duration::from_secs(4)).await;
        assert!(!out.delivered);
        assert!(t0.elapsed() <= Duration::from_secs(4));
    }
}
