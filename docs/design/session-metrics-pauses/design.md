# Session metrics, pauses & FIT export — Design

**Spec:** ./spec.md   **Last updated:** 2026-10-18   **Status:** Current

## Overview

All time-series maths gets one shared **gap rule** (`session/gap.rs`): two consecutive samples of a channel more than 5000 ms apart are separated by a gap. Integrals (work, distance, zone time) count at most 5 s across a gap. 1 Hz curves hold the last value across a hole of 5 s or less, and have no data across a gap. Live metrics (`metrics.rs`), analysis (`analysis.rs`) and FIT export (`fit_export.rs`) all call this helper. None of them keeps its own threshold.

The stored summary is built by one function, `MetricsCalculator::summary_metrics`. It runs on readings fed one at a time, the same way the live session feeds them. The one-time repair (A11) replays each raw `.bin` log through a fresh `MetricsCalculator` and calls that same function, so live and repaired numbers cannot diverge. A parity test checks this. The repair runs in the background after startup. It backs up the DB first with `VACUUM INTO`, recomputes idempotently, and records completion as `app_meta.metrics_version = 2`.

FIT export changes from "one record per power reading" to a merged 1 Hz stream over all sensor channels. Each channel goes invalid after 5 s without a reading. The export writes timer `stop_all`/`start` events around gaps and keeps elapsed time and timer time separate. Field numbers are checked against the official Garmin profile, and an independent decoder (`fitparser` crate) checks the output in `cargo test`.

Zones in effect at recording time are saved per session in a new nullable `sessions.zones` JSON column. Older sessions fall back to the current zones.

## Architecture

```mermaid
flowchart LR
  BLE[device readings] --> MGR[SessionManager.process_reading]
  MGR -->|record&#40;reading&#41;| CALC[MetricsCalculator]
  MGR --> LOG[sensor_log]
  CALC -->|summary_metrics| SUM[SessionSummary]
  LOG --> BIN[(raw .bin)]
  SUM --> DB[(sessions)]
  BIN --> REPAIR[repair::run_metrics_repair]
  REPAIR -->|summarize_readings = same record + summary_metrics| DB
  BIN --> ANA[analysis.rs] --> UI
  BIN --> FIT[fit_export.rs]
  GAP[gap.rs] -.used by.-> CALC & ANA & FIT
```

Components touched (all Area A unless noted):

| File | Change |
|---|---|
| `session/gap.rs` (new) | `MAX_READING_GAP_MS = 5000`, `is_gap(prev_ms, next_ms)`, `capped_interval_ms(prev_ms, next_ms)`, `resample_1hz_segments(&[(u64 ms, f64)]) -> Vec<Segment>` (a `Segment` is `{start_sec: u64, values: Vec<f64>}` covering contiguous seconds, holding values across holes of 5 s or less and splitting at gaps). |
| `session/manager.rs` | `MAX_READING_GAP_SECS` re-exported from `gap.rs` (it is no longer a separate constant). `process_reading` calls `metrics.record(&reading)`. Stop and autosave snapshot share one private `build_summary(&ActiveSession)` that calls `metrics.summary_metrics(active_secs)` and adds `zones` from `session.config`. |
| `session/metrics.rs` | NP fix (A1), gap-capped work and distance (A2), exclusive rolling window (A7), rounding (A8). Adds `record(&SensorReading)`, `SummaryMetrics`, `summary_metrics(active_secs)` and the free function `summarize_readings(readings, ftp, active_secs)`. |
| `session/analysis.rs` | Power curve over gap-split segments (A3). Timeseries sized by wall-clock span, sparse (A4). Zone distribution uses `gap::capped_interval_ms` and the session zones (A9). `build_timeseries_from_readings` loses its unused `duration_secs` argument. |
| `session/fit_export.rs` | Merged 1 Hz records, staleness, timer events, elapsed vs timer time, gap-rule distance (A5). `threshold_power` moves to field 45, and device_info `manufacturer` moves to field 2 (A6). |
| `session/repair.rs` (new) | One-time recompute (A11). |
| `session/storage/mod.rs` | Two inline idempotent schema objects: `app_meta` and `sessions.zones`. |
| `session/storage/meta.rs` (new) | `get_meta`, `set_meta`, `backup_db_to(path)` (`VACUUM INTO`). |
| `session/storage/sessions.rs` | `zones` is read and written. New `replace_session_metrics(id, &SummaryMetrics, &curve) -> Result<bool>`, which runs as one transaction. |
| `session/storage/power_curves.rs` | `save_power_curve` deletes the session's old rows before inserting, so durations that are no longer valid don't keep stale bests. |
| `commands.rs` | `get_session_analysis` resolves zones from `session.zones` or the config. The `build_timeseries_from_readings` call site loses its argument. **Cross-area: B also edits this file.** |
| `lib.rs` | Spawns the repair task after `Arc::new(storage)`. **Cross-area: E (logger, panic hook) and B (window close) also edit this file.** |
| `src/lib/utils/zoneAnalysis.ts` | `computeTimeToZone` uses `elapsed_secs` (A10). |
| `src/lib/components/ZoneRideAnalysis.svelte`, `src/routes/history/[id]/+page.svelte`, `src/lib/tauri.ts` | One- or two-line changes: pass `elapsed_secs` through, use `analysis.zones` for zone bounds, add optional TS fields. **Cross-area: D owns these files. The diffs are kept minimal.** |

Existing patterns followed: inline `run_alter_ignore_duplicate` / `CREATE TABLE IF NOT EXISTS` migrations in `Storage::new`; `spawn_blocking` for `load_sensor_data`; background `tokio::spawn` as already done for power-curve saves; tests in inline `mod tests` with `assert_approx`.

## Data and contracts

**SQLite (idempotent, inline in `Storage::new`, after the existing ALTERs):**
```sql
CREATE TABLE IF NOT EXISTS app_meta (key TEXT PRIMARY KEY, value TEXT);
ALTER TABLE sessions ADD COLUMN zones TEXT;   -- via run_alter_ignore_duplicate; NULL = unknown (pre-change)
```
- `app_meta('metrics_version')`: missing means legacy (version 1). `'2'` means the repair has completed. Area E's consolidation must include both objects (the spec already agrees this).
- `sessions.zones` holds JSON `{"power_zones":[u16;6],"hr_zones":[u8;5]}`. It is written only at save time, from the `SessionConfig` captured at `start_session`, and is never written by the repair.

**Rust types:**
- `pub struct ZoneSnapshot { power_zones: [u16; 6], hr_zones: [u8; 5] }` (Serialize/Deserialize), in `session/types.rs`.
- `SessionSummary` gains `#[serde(default)] pub zones: Option<ZoneSnapshot>`. Because of `serde(default)`, autosave JSON written by an older build still deserializes, and the recovered session keeps `zones = None`.
- `SessionAnalysis` gains `pub zones: ZoneSnapshot`. This always holds the zones the analysis actually used (the session snapshot, or the current config as fallback), so the UI draws bounds that match the distribution.
- `SummaryMetrics { avg_power: Option<u16>, max_power, normalized_power: Option<u16>, tss: Option<f32>, intensity_factor, variability_index, work_kj, distance_km, avg_hr: Option<u8>, max_hr, avg_cadence, avg_speed }`. This is the single source for both the summary and the repair.

**Frontend TS (additive, optional):** `SessionSummary.zones?: {power_zones, hr_zones} | null`; `SessionAnalysis.zones: {...}`; `TimeseriesPoint` already has `elapsed_secs`.

**Changed semantics (callers depend on these):**
- `TimeseriesPoint.elapsed_secs` stays seconds since the first reading, in **wall-clock** time. A pause shows up as missing points. That is unchanged, but the series is no longer cut off at `duration_secs`.
- `PowerCurvePoint` no longer appears for durations longer than the longest contiguous gap-free segment. The UI already handles missing durations, because short rides have them today.
- `LiveMetrics.avg_power_3s/10s/30s` contain at most N samples at 1 Hz.
- FIT files differ structurally (more records, extra events). FIT consumers are external.

**Backward compatibility:** old `.bin` files are read unchanged. Old DBs gain a nullable column and a new table. A DB downgraded to an older build ignores both.

## Decisions

### ADR-1: One gap-rule module shared by metrics, analysis and FIT
- Context: today there are three different thresholds: NP resets after more than 2 s, analysis caps at 5000 ms, and work, distance and FIT have no cap.
- Options: A) a `const` in each module; B) a shared `session/gap.rs` with `is_gap`, `capped_interval_ms` and `resample_1hz_segments`; C) pre-processing the raw log into a cleaned stream once, so every consumer reads that.
- Decision: **B**.
- Why: it keeps one definition, and each primitive can be unit tested with a hand-computed reference. C would need the live path to buffer and post-process, which conflicts with incremental live metrics. The threshold is **ms-based** (`next_ms - prev_ms > 5000`). Second-quantized consumers (NP, FIT) derive their decision from the ms timestamps, not from second indices, so the rule is identical everywhere. A hole that is not a gap spans at most 5 epoch-seconds, so at most 4 seconds are held.
- Reversibility: easy.

### ADR-2: NP handling of holes and gaps
- Context (A1): today a gap clears the whole-ride accumulators, and it also drops the last second before the gap.
- Options: A) on a gap, clear only the 30 s buffer, and skip missing seconds inside short holes; B) the same, but hold the previous second's average for missing seconds inside a hole of 5 s or less; C) zero-fill holes.
- Decision: **B**.
- Why: it uses the same hold semantics as the power curve (ADR-3), so the 1 Hz series that NP sees is the same one the curve sees. Zero-fill would penalise brief radio dropouts. On a gap, the pending second is flushed first, and then `np_buffer` is cleared. `fourth_power_sum` and `fourth_power_count` are never reset.
- Reversibility: easy (a one-function change, and it would be followed by a re-repair, see ADR-6).

### ADR-3: Power curve over gap-split segments
- Options: A) zero-fill gaps; B) split into contiguous segments and run the sliding window inside each segment only.
- Decision: **B**, using `gap::resample_1hz_segments`.
- Why: the spec says curves get "no data" across a gap. Zero-fill would still produce a non-existent "20-min best" built from 10 minutes of riding. Segments also bound memory by data size instead of by wall-clock span: a corrupt epoch can no longer allocate gigabytes. Per-second value = mean of the samples in that second, as today.
- Reversibility: easy (the repair re-runs).

### ADR-4: Analysis timeseries indexed sparsely by wall-clock second
- Decision: replace the `Vec` sized by `duration_secs` with a `BTreeMap<u64 sec, Slot>` keyed by `(epoch_ms - t0)/1000`. Emit only the seconds that have data.
- Why: this fixes the truncation (A4) without a dense allocation over the whole wall-clock span.
- Reversibility: easy.

### ADR-5: FIT record emission model
- Context (A5): today a record is written only on a Power reading, HR and cadence are held forever, and elapsed time is the same as timer time.
- Options: A) one record per raw reading of any type; B) one record per epoch-second that has at least one sample (merged 1 Hz stream), with each channel valid while its last reading is 5 s old or less; C) a dense 1 Hz stream that includes empty seconds.
- Decision: **B**.
- Why: it matches "one record per second of data". It doesn't write data during pauses ("Record messages should not be encoded to the file when the timer is paused" — Garmin, *Encoding FIT Activity Files*, https://developer.garmin.com/fit/articles/cookbook/encoding_activity_files.html). File size is bounded at 1 record/s.
- Details:
  - Per-second aggregation: power = mean of the samples in that second; HR, cadence and speed = last sample in that second.
  - Staleness: a channel is invalid (`0xFF` / `0xFFFF`) in record second `s` if its last sample is more than 5000 ms before the end of `s`. The check uses `gap::is_gap(last_ms, s_ms)`.
  - Distance: cumulative, built by integrating speed with `gap::capped_interval_ms`. It is written in every record, held from the last speed sample. It is omitted (`0xFFFFFFFF`) until the first speed sample.
  - Timer events: one `event` message type (21): `event=timer(0)`. The first event is `event_type=start(0)` at `start_ts`. When consecutive data seconds are separated by a gap (`is_gap(last_ms of prev second, first_ms of next)`), the export writes `stop_all(4)` at the previous record's timestamp and `start(0)` at the next record's timestamp. At the end it writes `stop_all(4)` at `end_ts`. `stop_all` is the type the existing file already uses for the final stop; one type is used throughout.
  - `end_ts = max(last record ts, start_ts + duration_secs)`.
  - Session and lap: `timestamp = end_ts`; `total_elapsed_time = (end_ts - start_ts) × 1000`; `total_timer_time = min(duration_secs, elapsed) × 1000`.
  - The activity message's `total_timer_time` (field 0) holds timer time. Messages appear in time order, with summary messages last ("Devices typically use the summary last message sequencing" — same cookbook page).
- Reversibility: easy (export is stateless).

### ADR-6: FIT field numbers checked against the official profile, plus an independent decoder
- Evidence: Garmin FIT JavaScript SDK `@garmin/fitsdk@21.217.0` (official, published by Garmin), `src/profile.js`, dumped by script:
  - session(18): 7 `total_elapsed_time` u32 ×1000; 8 `total_timer_time` u32 ×1000; 34 `normalized_power` u16; 35 `training_stress_score` u16 ×10; 36 `intensity_factor` u16 ×1000; **38 `end_position_lat` sint32**; **45 `threshold_power` u16 watts**.
  - lap(19): 7, 8, 25 `sport`, 39 `sub_sport`.
  - record(20): 3 HR u8, 4 cadence u8, 5 distance u32 ×100, 6 speed u16 ×1000, 7 power u16.
  - event(21): 0 `event`, 1 `event_type` (0 start, 1 stop, 4 stop_all). The enum value `timer` is 0.
  - activity(34): 0 `total_timer_time` ×1000, 1 `num_sessions`, 2 `type` (0 manual), 3 `event` (26 activity), 4 `event_type`.
  - **device_info(23): 2 `manufacturer` u16, 3 `serial_number` uint32z.** The current exporter writes manufacturer=255 into field 3 with size 2 and base type uint16. This is a second profile violation of the same kind as A6 (not in the findings list), and it is fixed in the same slice. The existing test `fit_export_includes_device_info` asserts the wrong field and will be updated.
- Decision: write field 45 and don't write field 38. Add code comments that cite the profile version.
- Independent decoder (automated): add dev-dependency `fitparser = "=0.11.0"` (crates.io, published 2026-05-01, MIT, generated from the FIT SDK profile, independent of our writer). Tests in `fit_export.rs` decode exported bytes with it and assert named fields (`threshold_power`, `total_elapsed_time`, record count, timer events).
- Independent decoder (recorded manual check): `fitdecode==0.11.0` (PyPI, 2025-08-06) in the scratchpad venv. An `#[ignore]` test `write_fit_fixtures` writes three fixtures (power+HR, HR+speed only, paused) to `$FIT_FIXTURE_DIR`. A script, not committed, decodes them with `fitdecode`, and its output goes into the Verification section of this document.
- Reversibility: easy. The dev-dependency can be dropped if it misbehaves; the fallback is the existing hand parser plus the fitdecode check.

### ADR-7: Zone snapshot per session
- Options: A) a new nullable JSON column `sessions.zones`; B) 11 integer columns; C) reuse `zone_config` (this is the zone-*ride* target, written only for zone rides, and owned by the zone-ride flow, so it is unsuitable).
- Decision: **A**.
- Why: it is one idempotent `ALTER`, there is nothing to backfill, NULL means "unknown, use current zones" (AC9 fallback), and it stays easy to absorb into Area E's single schema.
- Data flow: `start_session` already copies `SessionConfig` into `ActiveSession`. `build_summary` sets `zones: Some(ZoneSnapshot::from(&config))`, and `save_session` binds it as JSON. Autosave goes through `SessionSummary`, so it carries the snapshot.
- Reversibility: **costly, not one-way.** The column is additive, but once written it is user data that Area E has to keep.

### ADR-8: One-time recompute (A11)
- Where: `session/repair.rs::run_metrics_repair(storage: Arc<Storage>)`. It is spawned with `tauri::async_runtime::spawn` in `lib.rs` after autosave recovery and `Arc::new(storage)`, so it never blocks start-up, and recovered autosaves are repaired too.
- Same code path: `summarize_readings(readings, ftp, duration_secs)` = `MetricsCalculator::new(ftp)`, then `record(r)` for each reading in log order, then `summary_metrics(duration_secs)`. The live session calls the same `record` and `summary_metrics`. The raw log contains exactly the readings the live calculator processed, in the same order, because paused readings are dropped before both. Power curve = `compute_power_curve_from_readings`, as on stop. Parity test: run readings through `SessionManager`, stop, and assert that the summary metrics equal `summarize_readings(sensor_log, ftp, duration)` field by field.
- Algorithm:
  1. If `get_meta("metrics_version") >= 2`, return.
  2. `list_sessions()`. If there are no sessions, `set_meta("metrics_version","2")` and return. A fresh install gets no backup.
  3. Backup: if `<data_dir>/training.db.bak-metrics-v2` doesn't exist, run `VACUUM INTO '<path>'`. If the backup fails, log an error and **abort without setting the marker**, so it retries on the next launch. An existing backup is never overwritten, so a resumed run keeps the pre-repair copy.
  4. For each session, run `load_sensor_data` in `spawn_blocking`:
     - Err (missing or corrupt log): skipped, warn.
     - Empty log: skipped.
     - Otherwise compute the metrics and curve, then call `replace_session_metrics`. It runs one transaction: `UPDATE sessions SET avg_power, max_power, normalized_power, tss, intensity_factor, variability_index, work_kj, distance_km, avg_hr, avg_cadence, avg_speed WHERE id=?`. If `rows_affected == 0`, the session was deleted meanwhile: roll back and count it as skipped. Otherwise `DELETE FROM session_power_curves WHERE session_id=?`, then insert the new curve, then commit. A DB error counts as failed.
  5. If `failed == 0`, `set_meta("metrics_version","2")`.
  6. `info!("Metrics repair: repaired={} skipped={} failed={}")`.
- Untouched columns: `duration_secs`, `ftp`, title/type/RPE/notes, `zones`, `zone_config`, `max_hr`.
- FTP used: `session.ftp`. If that is NULL (pre-migration-004 rows), use the current config FTP for computing IF and TSS only, without writing `ftp`. This matches what `compute_analysis` already does for such rows. **Open question for the user**, see Risks.
- Resumability: each session's recompute is a pure function of (raw log, ftp, duration), so re-running is idempotent. An interrupted run simply runs again in full on the next launch, and no per-session cursor is needed. Each session's update is atomic.
- Why `VACUUM INTO` instead of `fs::copy`: it gives a consistent snapshot while the pool's other connections may be writing (a session stop or autosave during startup), and the copy is a single file. Bundled SQLite is 3.46.0, and `VACUUM INTO` requires 3.27 or later. sqlx 0.8.6 does not force WAL; it sets journal_mode only on request.
- Reversibility: **one-way for users' stored numbers** (old values are overwritten). It is mitigated by the backup file, which can be restored by hand. The marker can be cleared to re-run.

### ADR-9: Rolling window is exclusive at the old edge (A7)
- Decision: include `ts > last_ms - window*1000`.
- Why: at 1 Hz, a 3 s window then holds exactly 3 samples (t-2, t-1, t). The test `avg_power_window_boundary_is_inclusive` is renamed `avg_power_window_excludes_sample_exactly_window_old` and asserts 200.0 (only the 10000 ms sample).
- Reversibility: easy.

### Review rulings (2026-10-18) — amendments to the ADRs above
- **ADR-8 note A (FTP for NULL-ftp rows, supersedes the "current config FTP" default):** when `session.ftp` is NULL, derive `ftp = round(stored NP / stored IF)` when both are present and IF > 0; use it for IF/TSS but never write it to the `ftp` column. If not derivable, the repair leaves the stored IF and TSS unchanged for that session (other metrics are still recomputed). Tests: `repair_null_ftp_derives_ftp_from_stored_np_and_if`, `repair_null_ftp_without_if_keeps_stored_if_and_tss`.
- **ADR-8 note B (max_hr):** the repair recomputes and writes **all** `SummaryMetrics` fields, including `max_hr`. Untouched list is therefore `duration_secs`, `ftp`, title/type/RPE/notes, `zones`, `zone_config`.
- **ADR-8 note C (power-curve writes):** `save_power_curve` itself does delete-then-insert inside one transaction, so every writer (stop-time save, `backfill_power_curves`, repair) gets the same atomic replace; `replace_session_metrics` does the same delete+insert inside its own transaction.
- **ADR-8 note D (retry cap):** `app_meta.metrics_repair_attempts` is incremented at the start of each run. If a run ends with `failed > 0` and attempts ≥ 3, `metrics_version` is set anyway and the failing session ids are logged at warn. Test: `repair_gives_up_after_three_failed_attempts`.
- **ADR-5 note (short pauses):** pauses ≤ 5 s are indistinguishable from sensor holes in the raw log, so they produce **no** FIT timer stop/start events (records just skip those seconds; values are held/invalidated per the 5 s rule).
- **ADR-8 note E (Area E verify review, deleted-session race):** the background curve save after stop can race `delete_session`. `save_power_curve` (via the shared `replace_power_curve_in_tx`) runs the DELETE first (taking SQLite's write lock), then checks `SELECT 1 FROM sessions WHERE id = ?` inside the same transaction and inserts nothing if the row is gone. `get_best_power_curve` joins `sessions` in both period branches, so orphan rows left by older builds never count. Tests: `save_power_curve_for_deleted_session_writes_nothing`, `best_power_curve_ignores_orphan_rows`.
- **ADR-8 note F (verify review, backup integrity):** the backup is written with `VACUUM INTO <name>.tmp`, opened read-only and checked with `PRAGMA quick_check`, then renamed to `training.db.bak-metrics-v2`. A stale `.tmp` is deleted at start. An existing final backup is trusted only if it passes `quick_check`; otherwise it is moved aside to `.corrupt-<unix ms>` and redone. No session row is rewritten until a verified backup exists (a failed check aborts the run without setting the marker). Tests: `repair_replaces_corrupt_existing_backup_with_verified_one`, `repair_removes_stale_tmp_backup`.
- **ADR-1 note (verify review, active duration):** `TrainerCommand` readings are still logged but no longer advance `active_elapsed_ms` in `SessionManager`; only sensor readings do, capped by the same 5 s gap rule. Live duration, the repair (which keeps stored `duration_secs`) and FIT (which ignores commands) now share one notion of activity. This changes `duration_secs` only for future sessions where commands arrived while every sensor was silent. Tests: `trainer_commands_do_not_extend_active_time`, `active_time_counts_continuous_readings`.
- **ADR-8 note G (verify review, CPU work):** per session, loading the raw log, `summarize_readings` and the power curve all run inside one `spawn_blocking` call, awaited before the next session (sequential, one blocking thread at a time). Only the DB transaction runs on the async runtime. A behaviour-neutral refactor; covered by the existing `repair::tests::*`.
- **Integration note (rebased onto Area E, `fix/storage-infra-security`):** Area E's ordered migration runner (`storage/schema.rs`, `migrations/NNNN_*.sql`) is authoritative. This branch's inline `ALTER TABLE sessions ADD COLUMN zones TEXT` and inline `CREATE TABLE app_meta` were dropped. E's `0002_session_zones.sql` and `0003_app_meta.sql` provide the identical DDL (`zones TEXT` nullable; `app_meta (key TEXT PRIMARY KEY, value TEXT)`). No migration file was edited and none was added. The SHA-pin test passes. E's ride-lifecycle rules are kept as-is: raw file durable first, never overwrite, `SaveOutcome`, and the delete transaction. `save_session` now only adds the `zones` column to E's INSERT, and encodes the zones JSON before any file is written, so an encoding error can't leave a raw file without a row. Known gap: E's `RepairedRaw` path (row exists, raw file missing) updates metrics but not `zones`, so such a row keeps whatever zones it had (NULL for legacy rows), which falls back to the current zones. The power-curve deleted-session check sits alongside E's `delete_session` transaction. The repair's `training.db.bak-metrics-v2` and E's pre-migration backup are separate files.- **ADR-5 note B (build, staleness reference point):** a channel is valid in a record when its last sample is ≤ 5000 ms before the record's own timestamp (start of that epoch-second), i.e. `!is_gap(last_sample_ms, sec*1000)`, not "the end of s" as written above. Session `total_distance` falls back to the last record distance and is written invalid (not 0) when there is no speed data at all.
- **Schema note (Area E):** `app_meta` is exactly `CREATE TABLE IF NOT EXISTS app_meta (key TEXT PRIMARY KEY, value TEXT)` (nullable `value`). `PRAGMA user_version` is never used. `sessions.zones` stays an inline idempotent ALTER; Area E absorbs it (or Area A converts it to a migration file later if E lands first).
- **Cross-area:** CI/release `npm test` step comes from Area E (E7); this branch does not edit workflows. Frontend touches in `tauri.ts`, `history/[id]/+page.svelte`, `ZoneRideAnalysis.svelte` stay minimal (Area D edits nearby).

## Failure modes

| Failure | Detection | Behavior | Recovery |
|---|---|---|---|
| Raw log missing or corrupt during repair | `load_sensor_data` Err | Session skipped, warning logged, stored values kept | None needed; marker still set |
| DB error writing one session | sqlx Err | Transaction rolled back, `failed += 1`, marker not set | Retried on next launch (idempotent) |
| Backup fails (disk full, permissions) | `VACUUM INTO` Err | Repair aborts before touching any row | Retried on next launch |
| App quits mid-repair | none | Sessions done so far are committed, the rest unchanged | Full re-run on next launch; backup kept |
| Session deleted mid-repair | `rows_affected == 0` | Rollback (no orphan curve rows), counted as skipped | n/a |
| Corrupt epoch_ms (e.g. 0) in a log | n/a | Sparse timeseries and segmented curve avoid huge allocations; FIT emits records at that timestamp | n/a |
| Old autosave JSON without `zones` | serde default | `zones = None`, falls back to current zones | n/a |
| User edits metadata during repair | n/a | Repair updates only metric columns, so there is no lost update | n/a |
| Two writers replace the same session's power curve concurrently (stop-time save vs backfill vs repair) | n/a | `save_power_curve`/`replace_session_metrics` delete+insert in one transaction; SQLite serialises write transactions, so the last commit wins with a complete curve and no interleaved rows | n/a |
| Background curve save races `delete_session` | session row missing inside the save transaction | Nothing inserted; best-curve query also joins `sessions` | n/a |
| Repair keeps failing on the same session(s) | `metrics_repair_attempts` ≥ 3 with failed > 0 | Marker set anyway; failing ids logged at warn | Manual: clear the marker after fixing |
| `fitparser` rejects a file we believe is valid | test failure | Investigate against the SDK profile; fitdecode cross-check | Fix exporter or document the decoder bug |

## Migration and rollback
- Schema: additive only (`app_meta`, `sessions.zones`), inline and idempotent. Area E consolidates later.
- Data: one-time repair, guarded by the marker, with the backup at `training.db.bak-metrics-v2`.
- Rollback of the app version: an older build ignores the new table and column and reads the repaired numbers (same columns, same types).
- Rollback of the data: quit the app, then replace `training.db` with the `.bak-metrics-v2` file. This will be documented in the PR description.
- Downgrade-then-reupgrade (sessions recorded by an older build after the repair ran) is **unsupported during beta**: the marker is not re-checked per session and no finer-grained marker is kept.
- Future algorithm changes: bump `CURRENT_METRICS_VERSION` and use a new backup suffix. The repair is written generically for this.

## Traceability

| AC | Component(s) | Test(s) (planned names) |
|---|---|---|
| AC1 | metrics.rs NP | `np_short_dropout_keeps_whole_ride_np` (60 min 200 W, 3 s hole at 3585 s, NP 200 ± 0.1, IF 1.0 ± 0.01, VI 1.0 ± 0.01, TSS 100 ± 0.5 for 3600 s); `np_gap_clears_only_rolling_buffer` (100 W/gap/300 W, NP = hand-computed ≈ 253.0 ± 0.1); `np_hole_within_5s_holds_previous_second`; replaces `np_gap_resets_buffer` |
| AC2 | metrics.rs + gap.rs | `work_gap_contributes_at_most_5s` (720.6 ± 1 kJ); `distance_gap_contributes_at_most_5s` (30 ± 0.1 km); `capped_interval_caps_at_5000ms`; `is_gap_boundary_5000ms_is_not_gap` |
| AC3 | analysis.rs + gap.rs | `power_curve_not_carried_across_gap` (no 1200 s point, 300 s = 300 W); `power_curve_holds_value_across_short_hole`; `resample_splits_segments_at_gap` |
| AC4 | analysis.rs | `timeseries_includes_readings_after_pause` (last point at 4499 s, 3600 points with duration 3600) |
| AC5 | fit_export.rs | `fit_hr_speed_only_ride_has_one_record_per_second`; `fit_paused_ride_end_ts_not_before_last_record`; `fit_paused_ride_elapsed_is_wall_clock_timer_is_active`; `fit_writes_timer_stop_start_around_gap`; `fit_record_distance_follows_gap_rule`; `fit_hr_invalid_after_5s_without_reading` |
| AC6 | fit_export.rs | `fit_session_threshold_power_is_field_45` (and field 38 absent); `fit_device_info_manufacturer_is_field_2`; `fit_export_decodes_with_fitparser` (named fields); existing CRC/structure tests; recorded fitdecode check |
| AC7 | metrics.rs | `avg_power_3s_uses_exactly_last_3_samples_at_1hz`; renamed `avg_power_window_excludes_sample_exactly_window_old` |
| AC8 | metrics.rs | `summary_rounds_avg_power_half_up` (199.5 → 200); `summary_rounds_avg_power_down` (199.4 → 199); `summary_rounds_np` |
| AC9 | analysis.rs, storage, manager, commands | `zone_distribution_uses_session_zones_snapshot`; `analysis_falls_back_to_config_zones_when_snapshot_missing`; `save_session_round_trips_zones`; `legacy_row_has_no_zones`; `stop_summary_captures_zones_from_session_config` |
| AC10 | zoneAnalysis.ts | `computeTimeToZone uses elapsed_secs when a second is missing`; existing tests updated to the new signature |
| AC11 | repair.rs, storage/meta.rs, sessions.rs | `repair_recomputes_metrics_from_raw_log`; `repair_leaves_duration_ftp_and_metadata_untouched`; `repair_rebuilds_power_curve_dropping_stale_durations`; `repair_runs_once` (marker set, second run doesn't touch rows); `repair_creates_backup_before_writing`; `repair_rerun_after_interruption_is_idempotent`; `repair_skips_missing_raw_log_and_still_sets_marker`; `repair_does_not_set_marker_when_a_write_fails`; `repair_fresh_db_sets_marker_without_backup`; `live_summary_equals_replayed_summary` (parity); `app_meta_and_zones_schema_is_idempotent` (Storage::new twice) |
| AC12 | all | `cd src-tauri && cargo test`, `npm test`, `npm run check` |

## Risks and spikes
1. **Riskiest: `fitparser` compiles in this workspace and decodes our files.** Spike in slice 1: add the dev-dependency, decode the *current* exporter's output, and expect it to show field 38 as `end_position_lat` and device_info field 3 as `serial_number`. If the crate doesn't build or misbehaves, fall back to the hand parser plus a recorded `fitdecode` check (the spec allows a recorded manual check).
2. Live/replay parity: `SessionManager` computes `duration_secs` from `Instant`, while metrics use reading `epoch_ms`. The parity test compares metrics given the same `active_secs`, so the duration source is not a variable in it. Readings dropped by the broadcast channel (`Lagged`) are missing from both the live calculator and the log, so parity still holds.
3. AC2 tolerance: the hand computation gives 3603 s × 200 W = 720.6 kJ, inside ±1, as the spec intends (gap ≤ 5 s).
4. **Open question (user):** for sessions with NULL `ftp`, the design computes IF and TSS with the current FTP (same as analysis today). The alternative is to leave IF and TSS as they are for those rows. The default is the current FTP, and the count is logged.
5. Repaired numbers change PMC history. This is intended by the user's decision, and the backup protects it.

## Implementation plan
Each slice is one commit with its tests, test first.
1. **FIT decoder spike and A6.** Add `fitparser = "=0.11.0"` as a dev-dependency. Write `fit_export_decodes_with_fitparser`. Move threshold_power to 45 and device_info manufacturer to 2; update `fit_export_session_summary_fields` and `fit_export_includes_device_info`; add profile-citation comments.
2. **`gap.rs`.** `is_gap`, `capped_interval_ms`, `resample_1hz_segments` with boundary tests. Manager and analysis switch their constants to it with no behaviour change.
3. **metrics.rs A1, A2, A7.** NP (ADR-2), gap-capped work and distance, exclusive window. Update `np_gap_resets_buffer` and `np_one_second_gap_no_reset` to the new contract.
4. **Summary refactor and A8.** `record()`, `SummaryMetrics`, `summary_metrics`, `summarize_readings`, manager `build_summary`, rounding, parity test.
5. **analysis.rs A3, A4.** Segmented power curve, sparse timeseries, `build_timeseries_from_readings` signature (commands.rs call site). `save_power_curve` deletes before inserting.
6. **Zone snapshot (A9).** `ZoneSnapshot`, `SessionSummary.zones`, `sessions.zones` column, save and read, `SessionAnalysis.zones`, command fallback, TS types, history page uses `analysis.zones`.
7. **A10.** `computeTimeToZone` by `elapsed_secs`; `ZoneRideAnalysis.svelte` passes `elapsed_secs`.
8. **FIT A5.** Merged 1 Hz stream, staleness, distance, timer events, elapsed vs timer time; `#[ignore] write_fit_fixtures`; run the fitdecode check in the scratchpad venv and record the output here.
9. **Repair (A11).** `app_meta`, `meta.rs` (get, set, `VACUUM INTO`), `replace_session_metrics`, `repair.rs`, the `lib.rs` spawn, and all AC11 tests (tempdir DBs with real `.bin` files written via `save_session`).
10. **Finish.** Full suites; Verification section; statuses updated.

## Verification

Commands (2026-10-18, branch `fix/session-metrics-pauses`):
`cd src-tauri && cargo test` → 414 passed, 1 ignored (`write_fit_fixtures`, manual helper), 0 failed, after rebasing onto Area E, which brings its own tests (this branch alone, before the rebase: 361; baseline on `main`: 289). `npm test` → 72 passed (baseline 71).
Updated after the verify-review follow-ups (backup verification, trainer commands, blocking pool, FIT durations). `npm run check` → 0 errors, 0 warnings. `cargo build --features production` → builds.
Every new behaviour test below was run and seen failing against the pre-fix code before the fix landed. The exceptions are regression guards that already passed and tests written together with a new type/API, which were mutation-checked instead (zone snapshot: reverting the analysis/manager wiring fails `zone_distribution_uses_session_zones_snapshot` and `stop_summary_captures_zones_from_session_config`).

| AC | Proven by (all in `src-tauri/src/session/…` unless noted) |
|---|---|
| AC1 | `metrics::tests::np_short_dropout_keeps_whole_ride_np`, `np_gap_clears_only_rolling_buffer` (253.03 ± 0.1 W), `np_rolling_window_does_not_span_gap`, `np_hole_within_5s_fills_missing_seconds`, `np_hole_holds_previous_second_value` |
| AC2 | `metrics::tests::work_gap_contributes_at_most_5s` (720.6 kJ), `distance_gap_contributes_at_most_5s` (30.033 km); `gap::tests::*` (boundary 5000 ms not a gap, cap, backward timestamps) |
| AC3 | `analysis::tests::power_curve_not_carried_across_gap`, `power_curve_holds_value_across_short_hole`, `power_curve_best_found_in_later_segment`; `gap::tests::resample_splits_segments_at_gap`; `storage::tests::save_power_curve_replaces_stale_durations` |
| AC4 | `analysis::tests::timeseries_includes_readings_after_pause`, `timeseries_bad_epoch_does_not_allocate_wall_clock_span` |
| AC5 | `fit_export::tests::fit_hr_speed_only_ride_has_one_record_per_second`, `fit_power_at_4hz_yields_one_record_per_second_with_mean_power`, `fit_paused_ride_end_ts_not_before_last_record`, `fit_paused_ride_elapsed_is_wall_clock_timer_is_active`, `fit_writes_timer_stop_start_around_gap`, `fit_short_pause_writes_no_timer_events`, `fit_record_distance_follows_gap_rule`, `fit_hr_invalid_after_5s_without_reading`, `fit_paused_ride_decodes_with_fitparser`, `fit_end_ts_falls_back_to_active_duration_when_longer_than_span`, `fit_end_ts_is_last_record_when_span_exceeds_duration` (the last two were added after the fix as coverage tests, so they passed from the start). All make_summary-based exports with readings use a `duration_secs` that matches the readings. |
| AC6 | `fit_export::tests::fit_session_threshold_power_is_field_45`, `fit_device_info_manufacturer_is_field_2`, `fit_export_decodes_with_fitparser` (fitparser =0.11.0), the existing CRC/structure tests, and the recorded fitdecode check below |
| AC7 | `metrics::tests::avg_power_3s_uses_exactly_last_3_samples_at_1hz`, `avg_power_30s_uses_exactly_last_30_samples_at_1hz`, `avg_power_window_excludes_sample_exactly_window_old` (replaces `avg_power_window_boundary_is_inclusive`) |
| AC8 | `manager::tests::stop_summary_rounds_avg_power_half_up`, `stop_summary_rounds_avg_power_down`, `stop_summary_rounds_np_to_nearest` |
| AC9 | `analysis::tests::zone_distribution_uses_session_zones_snapshot`, `analysis_falls_back_to_config_zones_when_snapshot_missing`; `manager::tests::stop_summary_captures_zones_from_session_config`; `storage::tests::save_session_round_trips_zones`, `legacy_row_has_no_zones`, `autosave_summary_json_without_zones_still_deserializes`, `reopening_storage_is_idempotent_with_app_meta_and_zones` |
| AC10 | `src/lib/utils/zoneAnalysis.test.ts` › computeTimeToZone › `uses elapsed_secs when a second is missing from the series`, `measures from the first point when the series starts after 0` |
| AC11 | `repair::tests::repair_recomputes_metrics_from_raw_log`, `repair_leaves_duration_ftp_and_metadata_untouched`, `repair_rebuilds_power_curve_dropping_stale_durations`, `repair_runs_once`, `repair_creates_backup_before_writing`, `repair_rerun_after_interruption_is_idempotent`, `repair_skips_missing_raw_log_and_still_sets_marker`, `repair_skips_corrupt_raw_log`, `repair_does_not_set_marker_when_a_write_fails`, `repair_gives_up_after_three_failed_attempts`, `repair_fresh_db_sets_marker_without_backup`, `repair_null_ftp_derives_ftp_from_stored_np_and_if`, `repair_null_ftp_without_if_keeps_stored_if_and_tss`, `replace_metrics_for_deleted_session_writes_nothing`; live/repair parity: `manager::tests::live_summary_equals_replayed_summary`; deleted-session race: `storage::tests::save_power_curve_for_deleted_session_writes_nothing`, `best_power_curve_ignores_orphan_rows`; meta: `get_meta_missing_key_returns_none`, `set_meta_overwrites_value`; backup integrity: `repair_replaces_corrupt_existing_backup_with_verified_one`, `repair_removes_stale_tmp_backup`; active time from sensor readings only: `manager::tests::trainer_commands_do_not_extend_active_time`, `active_time_counts_continuous_readings` |
| AC12 | the suite results above |

**Recorded independent decoder check (AC6).** Steps:
1. `FIT_FIXTURE_DIR=<dir> cargo test --lib write_fit_fixtures -- --ignored` wrote three fixtures.
2. A script decoded them with `fitdecode==0.11.0` (Python venv, `CrcCheck.RAISE`, so the header and file CRCs were verified).

Output, regenerated after the duration fix to the fixtures (`hr_speed_only` and `power_hr` now use `duration_secs = 60`), record dicts trimmed:
```
== paused.fit
  messages: {'file_id': 1, 'device_info': 1, 'event': 4, 'record': 120, 'lap': 1, 'session': 1, 'activity': 1}
  device_info (manufacturer, serial): ('development', False)
  session: {'total_elapsed_time': 719.0, 'total_timer_time': 120.0, 'threshold_power': 200, 'normalized_power': 190, 'training_stress_score': 75.0, 'intensity_factor': 0.95, 'has_end_position_lat': False}
  timer events: [('start', '2024-06-15T10:00:00+00:00'), ('stop_all', '2024-06-15T10:00:59+00:00'), ('start', '2024-06-15T10:11:00+00:00'), ('stop_all', '2024-06-15T10:11:59+00:00')]
  first record: {'timestamp': 10:00:00, 'power': 200, 'heart_rate': 140, 'cadence': None, 'speed': None, 'distance': None}
  last record:  {'timestamp': 10:11:59, 'power': 200, 'heart_rate': 140, 'cadence': None, 'speed': None, 'distance': None}
== hr_speed_only.fit
  messages: {'file_id': 1, 'device_info': 1, 'event': 2, 'record': 60, 'lap': 1, 'session': 1, 'activity': 1}
  device_info (manufacturer, serial): ('development', False)
  session: {'total_elapsed_time': 60.0, 'total_timer_time': 60.0, 'threshold_power': 200, ..., 'has_end_position_lat': False}
  timer events: [('start', '2024-06-15T10:00:00+00:00'), ('stop_all', '2024-06-15T10:01:00+00:00')]
  first record: {'timestamp': 10:00:00, 'power': None, 'heart_rate': 140, 'speed': 10.0, 'distance': 0.0}
  last record:  {'timestamp': 10:00:59, 'power': None, 'heart_rate': 140, 'speed': 10.0, 'distance': 590.0}
== power_hr.fit
  messages: {'file_id': 1, 'device_info': 1, 'event': 2, 'record': 60, 'lap': 1, 'session': 1, 'activity': 1}
  session: {'total_elapsed_time': 60.0, 'total_timer_time': 60.0, 'threshold_power': 200, ..., 'has_end_position_lat': False}
  timer events: [('start', '2024-06-15T10:00:00+00:00'), ('stop_all', '2024-06-15T10:01:00+00:00')]
  first/last record: power 200, heart_rate 140 at 10:00:00 / 10:00:59
```

**Not proven automatically:**
- An actual import into Garmin Connect or Strava. Both decoders accept the files, but no upload was done.
- The `lib.rs` startup wiring (spawning the repair in the background). It is covered by a compile and a code read only; the repair itself is fully tested.
