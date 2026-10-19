# Session metrics, pauses & FIT export — Spec

**Track:** Standard   **Product stage:** Introduction/early Growth (0.7.x-beta)
**Status:** Agreed   **Last updated:** 2026-10-18
**Findings:** A1–A11 (see findings list)

## Problem
Riders who pause, or whose sensors drop out briefly, get wrong ride numbers: NP/TSS/IF/VI can vanish or reflect only the end of the ride, work and distance are inflated by pause time, power-curve bests are fabricated from paused time, the last part of a paused ride disappears from the analysis charts, and FIT exports are incomplete or mis-timed. These numbers feed long-term training load (PMC), so errors compound.

## Users
Every rider who pauses or has a sensor dropout (most rides). Secondary: anyone importing FIT exports into Garmin Connect / Strava.

## Goals
- Pauses and gaps are handled by **one consistent rule** across live metrics, stored summaries, analysis and FIT export.
- Previously saved sessions are repaired once on upgrade (user decision).

## Non-goals
- Changing how active `duration_secs` is computed (`session/manager.rs` already caps gaps at `MAX_READING_GAP_SECS` = 5 s and excludes pauses).
- Detecting pauses retroactively in old raw logs (raw `.bin` logs have no pause markers; a pause and a long dropout are treated identically).
- New metrics or UI.

## Acceptance criteria
The **gap rule**: two consecutive samples of a channel more than `MAX_READING_GAP_SECS` (5 s) apart are separated by a *gap*. Time spans across a gap contribute at most 5 s (integrals) or no data (curves/series).

- [ ] AC1 (A1): Given a 60-min ride at constant 200 W with one 3 s dropout at 59:45, NP = 200 ± 0.1 W and TSS/IF/VI are present. Given 30 min at 100 W, a gap, then 30 min at 300 W, NP equals NP computed over both halves (gap clears only the 30 s rolling buffer, never the whole-ride accumulators).
- [ ] AC2 (A2): Given 30 min at 200 W, a 10-min gap, 30 min at 200 W, work = 720 ± 1 kJ (a gap contributes ≤ 5 s). Same rule for distance: 30 km/h for 1 h with a 10-min gap in the middle = 30 ± 0.1 km (plus ≤ 5 s of travel).
- [ ] AC3 (A3): Given 300 W for 5 min, a 20-min gap, then 100 W for 5 min, the 20-min best power is not ≥ 300 W; values are not carried across a gap (short ≤ 5 s holes may still be held).
- [ ] AC4 (A4): Given 60 min riding + 15-min pause, the analysis timeseries contains the readings from the final 15 minutes of riding (no truncation by active duration).
- [ ] AC5 (A5): FIT export of an HR+speed-only ride (no power) contains one record per second of data. With a pause, session/lap end timestamps are ≥ the last record's timestamp, `total_elapsed_time` = wall-clock span, `total_timer_time` = active time, and a timer stop/start event pair is written around each gap. Record distance follows the gap rule. HR/cadence fields are omitted (invalid) when their sensor has had no reading for > 5 s.
- [ ] AC6 (A6): FIT session message writes FTP as `threshold_power` field 45 (uint16); field 38 is not written. Exports still pass the existing FIT-structure/CRC tests and are verified by decoding with an independent FIT decoder (e.g. `fitparse`/`fitdecode` in Python, or the Garmin FIT SDK) in a test or recorded manual check.
- [ ] AC7 (A7): At 1 Hz, the "3 s" average uses exactly the last 3 samples (same for 10 s / 30 s); the test `avg_power_window_boundary_is_inclusive` is updated to assert the new contract.
- [ ] AC8 (A8): Stored avg power and NP are rounded (199.5 → 200, 199.4 → 199), not truncated.
- [ ] AC9 (A9): Session analysis zone distribution uses the zones in effect when the session was recorded. Sessions recorded before this change fall back to current zones. (Snapshot mechanism is a design decision; must be backward compatible.)
- [ ] AC10 (A10): Time-to-zone uses each point's `elapsed_secs`, not its array index; a timeseries with a missing second yields the correct time.
- [ ] AC11 (A11): On first launch after upgrade, `training.db` is copied to `training.db.bak-<schema-or-metrics-version>` and every session with a raw log has avg/max power, NP, TSS, IF, VI, work, distance, avg HR/cadence/speed recomputed with the fixed code and its power-curve cache rebuilt. `duration_secs`, FTP, title/type/RPE/notes are untouched. The repair runs exactly once (re-launch doesn't redo it), is resumable/idempotent if interrupted, skips sessions with missing/corrupt raw logs with a logged warning, and never blocks app start (runs in background). A log line reports counts (repaired/skipped/failed).
- [ ] AC12: All existing tests pass; each AC has a test that fails before and passes after (CLAUDE.md test rules: hand-computed reference values, assert_approx epsilons watts ±0.1, ratios ±0.01, TSS ±0.5).

## Success signals
Paused rides show NP/TSS consistent with an unpaused equivalent; no 20-min bests above plausible values; FIT imports show correct elapsed vs moving time in Strava/Garmin.

## Constraints
No change to the session file format (`.bin` sensor log) — old logs must load. SQLite schema changes must be idempotent and work on existing user DBs.

## Assumptions and open questions
- Assumption: 5 s is the right gap threshold everywhere (reuses `MAX_READING_GAP_SECS`). Default: reuse it.
- Decision: the run-once marker for AC11 is a row in a new `app_meta(key TEXT PRIMARY KEY, value TEXT)` table, created idempotently with today's inline mechanism in `Storage::new` (key `metrics_version`). Area E's schema consolidation will absorb it.
- Open: FIT field numbers — verify against the FIT SDK Profile, not memory. Owner: implementer; default: field 45 for threshold_power.
