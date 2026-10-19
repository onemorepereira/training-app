//! Backend lifecycle hooks for zone control (design ADR-6, ADR-18).
//!
//! Tauri handlers call these in one line each, so the behaviour is testable
//! without a Tauri runtime. All are no-ops when no zone ride is running.

use log::info;

use super::controller::{ZoneController, APP_CLOSE_RELEASE_BUDGET};
use super::trainer::TrainerBackend;
use super::types::StopReason;

/// `stop_session`: stop zone control and release the trainer before the
/// session is saved.
pub async fn on_session_stop<B: TrainerBackend>(zc: &ZoneController<B>) -> Option<StopReason> {
    let reason = zc.stop(StopReason::SessionStopped).await;
    if let Some(r) = &reason {
        info!("Session stop: zone control ended ({r:?})");
    }
    reason
}

/// `pause_session`: pause zone control (trainer released until resume).
pub async fn on_session_pause<B: TrainerBackend>(zc: &ZoneController<B>) {
    zc.pause().await;
}

/// `resume_session`: resume zone control (target re-sent on next tick).
pub async fn on_session_resume<B: TrainerBackend>(zc: &ZoneController<B>) {
    zc.resume().await;
}

/// Window close / app exit: refuse new zone rides and stop any running one
/// within a hard 4 s budget.
pub async fn on_app_close<B: TrainerBackend>(zc: &ZoneController<B>) -> Option<StopReason> {
    zc.close(APP_CLOSE_RELEASE_BUDGET).await
}

#[cfg(test)]
mod tests {
    use super::super::controller::task_tests::{power_target, ride, setup, start};
    use super::super::trainer::fake::Behavior;
    use super::super::trainer::{CommandOutcome, TrainerCmd};
    use super::*;
    use std::time::Duration;
    use tokio::time::{sleep, Instant};

    fn quiet(_: u64) -> Vec<crate::device::types::SensorReading> {
        vec![]
    }

    #[tokio::test(start_paused = true)]
    async fn on_session_stop_releases_trainer_before_returning() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 2, quiet).await;
        fake.set_power_behavior(Behavior::Hang(Duration::from_secs(1)));
        assert_eq!(on_session_stop(&zc).await, Some(StopReason::SessionStopped));
        // Returned only after the (slow) 0 W release reached the trainer
        assert!(fake.commands().ends_with(&[TrainerCmd::Power(0), TrainerCmd::Resistance(0)]));
        assert_eq!(zc.status().await.stop_reason, Some(StopReason::SessionStopped));
        assert_eq!(zc.last_release_confirmed().await, Some(true));
    }

    #[tokio::test(start_paused = true)]
    async fn on_session_stop_without_zone_ride_is_noop() {
        let (zc, fake, _tx) = setup();
        assert_eq!(on_session_stop(&zc).await, None);
        assert!(fake.commands().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn on_session_pause_releases_and_resume_resends() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 2, quiet).await;
        on_session_pause(&zc).await;
        sleep(Duration::from_millis(100)).await;
        assert!(fake.commands().ends_with(&[TrainerCmd::Power(0), TrainerCmd::Resistance(0)]));
        fake.clear();
        on_session_resume(&zc).await;
        ride(&tx, 2, quiet).await;
        assert_eq!(fake.commands().first(), Some(&TrainerCmd::Power(200)));
    }

    #[tokio::test(start_paused = true)]
    async fn on_app_close_respects_4s_budget_with_hung_trainer() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 2, quiet).await;
        fake.set_power_behavior(Behavior::Hang(Duration::from_secs(60)));
        fake.set_resistance_behavior(Behavior::Hang(Duration::from_secs(60)));
        let t0 = Instant::now();
        on_app_close(&zc).await;
        assert!(t0.elapsed() <= Duration::from_secs(4), "close took {:?}", t0.elapsed());
    }

    #[tokio::test(start_paused = true)]
    async fn on_app_close_reports_unconfirmed_release_when_zero_watt_fails() {
        let (zc, fake, tx) = setup();
        start(&zc, &tx, power_target(190, 210, None), None).await;
        ride(&tx, 2, quiet).await;
        fake.set_power_behavior(Behavior::Err);
        assert_eq!(on_app_close(&zc).await, Some(StopReason::AppClosing));
        // The 0 W release was attempted, failed, and was reported unconfirmed;
        // only the best-effort resistance step reached the trainer.
        assert!(fake.attempts().iter().any(|(c, _)| *c == TrainerCmd::Power(0)));
        let outcomes = fake.outcomes();
        assert!(outcomes.contains(&(TrainerCmd::Power(0), CommandOutcome::Err)), "{outcomes:?}");
        assert_eq!(outcomes.last(), Some(&(TrainerCmd::Resistance(0), CommandOutcome::Ok)));
        assert_eq!(zc.last_release_confirmed().await, Some(false));
        let cmds = fake.commands();
        assert_eq!(cmds.iter().filter(|c| **c == TrainerCmd::Power(0)).count(), 0);
    }
}
