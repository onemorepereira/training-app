# Zone control & trainer safety — Spec

**Track:** Standard   **Product stage:** Introduction/early Growth
**Status:** Built   **Last updated:** 2026-10-18
**Findings:** B1–B11

## Problem
Zone rides drive a smart trainer automatically. Today the HR controller overshoots and oscillates, custom HR zones can never be reached, bad input crashes the controller silently while the trainer stays at its last wattage, a cadence stop leaves power-mode rides stuck at 0 W, and nothing in the backend releases the trainer when the session stops or the app closes. For a device that physically resists the rider, these are safety and trust failures.

## Users
Riders using Zone Ride (power or HR target) on an FTMS or FE-C trainer.

## Goals
- The controller converges on HR targets without sustained oscillation and respects its power band.
- No input can crash the controller; invalid input is rejected with a clear error.
- The trainer is always left in a known safe state when control ends for any reason.

## Non-goals
- Retuning gains beyond what's needed to make the corrected control law behave (no new control modes).
- UI changes (Area D owns the frontend; this area only changes backend behaviour and command contracts).

## Acceptance criteria
- [ ] AC1 (B1): The HR PID uses a control law where, with error held at 0 and stable, commanded power stops changing (no residual ramp); the integrator does not accumulate while the output is saturated at the band floor/ceiling or rate limit. A simulated first-order-lag HR plant (test harness, deterministic, no real clock) starting 20 bpm below target settles within the target band with overshoot ≤ 1 zone-band half-width and no sustained oscillation over 20 simulated minutes.
- [ ] AC2 (B2): An HR target with zone 0 (custom bounds) uses power band [MIN_POWER, 1.5 × FTP] (or 400 W without FTP); a custom 150–160 bpm target can command above 55 % FTP.
- [ ] AC3 (B3): `start_zone_control` rejects with a descriptive `AppError` — not panics — when: zone > 7 (power) / > 5 (HR) except 0 = custom; lower ≥ upper; power bound > 2000 W; HR bound > 250 bpm. `save_user_config` rejects FTP outside 50–1000 W, and invalid HR/power zone arrays (non-increasing). No `clamp` in the controller can panic for any FTP ≥ 1 (band always satisfies floor ≤ ceiling). Midpoint arithmetic cannot overflow.
- [ ] AC4 (B3 robustness): If the control task ends abnormally (panic or error), status reports inactive with a stop reason, and a safe command (see AC6) is sent.
- [ ] AC5 (B4, B7): In power mode, after a cadence-zero safety cut, the target power is re-sent within 2 ticks once cadence ≥ 40 rpm has been sustained 2 s (amended 2026-10-18, verify review: avoids the ERG low-cadence "spiral of death"). After a trainer reconnect, the current target is re-sent within 2 ticks.
- [ ] AC6 (B5): When zone control stops for any reason — user stop, `stop_session`, `pause_session` (pauses control; resume restores), window close, duration complete, sensor lost, abnormal end — the backend itself (not just the UI) stops/pauses the controller and commands the trainer to a defined safe state (design decides: release ERG or set a low safe wattage), with a log line.
- [ ] AC7 (B6): All target-power commands are clamped to [0, MAX_TRAINER_WATTS] (2000 W) in one place on the backend path to the trainer; no `as i16` wrap is possible.
- [ ] AC8 (B8): Pausing for 10 minutes adds 0 to time-in-zone and the first post-resume PID update uses a normal tick dt.
- [ ] AC9 (B9): The falling-HR integral decay triggers only when the smoothed HR is lower than the previous tick's smoothed HR; test uses real controller state progression, not an injected fake previous value.
- [ ] AC10 (B10): `command_trainer` is bounded by a timeout (≤ 3 s; amended 2026-10-18, design review ruling 9) and a timeout is treated like a failed command; `stop_zone_control` doesn't hold the controller lock while awaiting task shutdown (status polling stays responsive).
- [ ] AC11 (B11): PID update with dt ≤ 0 returns 0 adjustment (no NaN); the HR-ceiling safety uses smoothed HR (or requires the excursion to persist ≥ 2 consecutive samples) so a single spike doesn't cut power.
- [ ] AC12: All existing tests pass; each AC has a deterministic test (no sleeps/real clocks; use tokio time pause or injected clocks) that fails before and passes after.

## Constraints
Trainer command path is shared with manual ERG (`set_trainer_power`) — clamping must apply there too. Tick periods (1 s power / 5 s HR) unchanged unless the design records why.

## Assumptions and open questions
- Safe state on stop (AC6): default = release control (FTMS: Reset/stop; FE-C: basic resistance 0 % or target power to a low value) — implementer decides per protocol and records an ADR.
- Rounding of what counts as "custom": default zone 0 = custom (current frontend contract).
