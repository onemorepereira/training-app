# Storage, infra & security — Design

**Spec:** ./spec.md   **Last updated:** 2026-10-18   **Status:** Draft

## Overview

Seven small fixes. Two of them change on-disk behaviour that can't be undone, and both are flagged below.

1. **Schema (E6, one-way door).** The schema gets one ordered migration list. Each step runs once, inside a transaction, and is tracked by `PRAGMA user_version`. Step 1 is a frozen **baseline** that replays exactly what `main` runs today (001 + the inline ALTERs, ignoring duplicate columns, + `session_power_curves` + Area A's `app_meta`). Because of that, every shipped DB state converges to the same schema, and later steps can be plain SQL. We do not use `sqlx::migrate!`: it can't adopt existing DBs that have no `_sqlx_migrations` table, and its checksum check can stop the app from starting (ADR-1).
2. **Ride data (E1/E2/E4).** Persisting a ride becomes "ensure, never overwrite". The row is inserted only if absent, and the raw file is written only if absent, atomically (tmp + rename). The autosave writer and stop are serialized by one *autosave gate*, so no autosave can appear after stop. Stop writes a full final autosave **before** it touches the DB, so a failed save loses nothing. Delete removes the autosave first, then runs one DB transaction, then removes the raw file. With that ordering no tombstones are needed (ADR-3).
3. **Privileged setup (E3).** One `pkexec` call runs a constant script compiled into the binary. The script gets fixed tokens as argv and the udev rule on **stdin**: `install -m 0644 /dev/stdin /etc/udev/rules.d/…`. No path is ever written to disk before install, no path comes from outside the binary, and it works the same for deb, rpm, AppImage and dev builds (ADR-4).
4. **Logging (E5).** The `LoggerHandle` lives in a `static OnceLock`. It is flushed on `RunEvent::Exit` and in a panic hook that logs message + location at error level before chaining to the default hook (ADR-5).
5. **CI (E7).** `npm test` runs in `ci.yml` and `release.yml`.

## Architecture

| Component | File(s) | Change |
|---|---|---|
| Migration runner | `src-tauri/src/session/storage/schema.rs` (new), `storage/mod.rs` | `MIGRATIONS: &[Migration]` + `migrate(&pool)`; `Storage::new` calls it in place of the inline DDL |
| Migration SQL | `src-tauri/migrations/0001_baseline.sql` (new); delete `001_init.sql`…`008_work_and_vi.sql` | Single source of schema |
| Session persistence | `storage/sessions.rs` | `save_session` → returns `SaveOutcome`; atomic write-if-absent raw file; `delete_session` reordered + transaction |
| Autosave | `storage/autosave.rs` | `remove_autosave` returns `Result` (NotFound = Ok); recovery uses ensure-semantics, cleans stale autosaves; new `finalize_session` |
| Autosave gate | `session/manager.rs` | `autosave_gate: tokio::sync::Mutex<()>`; `stop_session_with_log` takes it; new `autosave_ticket()` |
| Autosave task | `lib.rs` → `session/autosave_task.rs` (new, small) | Loop body extracted as `autosave_tick(&SessionManager, &Storage, &mut Accumulator)` so it can be tested |
| Stop / close paths | `commands.rs::stop_session`, `lib.rs` `CloseRequested` | Both call `Storage::finalize_session` |
| Prereq fix | `prerequisites.rs`, `commands.rs::fix_prerequisites`, `resources/linux-setup.sh` (new) | Single invocation builder; no `/tmp`, no resource dir |
| Logger | `lib.rs` | `static LOGGER: OnceLock<LoggerHandle>`; `Builder::build(..).run(|_, ev| …)`; panic hook + pure `format_panic` |
| CI | `.github/workflows/ci.yml`, `release.yml` | `npm test` step after `npm run check` |
| Docs | `CONTRIBUTING.md`, `CLAUDE.md` | "Schema changes" section |

Existing patterns are kept: `AppError` everywhere, `tokio::fs` in async code, inline `#[cfg(test)]` modules, `tempfile` in tests only, and argv vectors for privileged commands.

### Lock order (deadlock-free by construction)

`autosave_gate` → `current_session`. Both the stop path and the autosave tick take the gate first. No code takes the gate while it holds `current_session`.

```mermaid
sequenceDiagram
    participant T as Autosave tick
    participant G as autosave_gate
    participant S as Stop (finalize)
    T->>G: lock
    T->>T: snapshot (session active) + write .autosave
    T->>G: unlock
    S->>G: lock
    S->>S: take session (now None for future ticks)
    S->>G: unlock
    S->>S: write final autosave (full log)
    S->>S: insert row if absent, write raw if absent
    S->>S: remove autosave
    T->>G: lock
    T->>T: snapshot → None, write nothing
```

## Data and contracts

### Schema

- **`PRAGMA user_version`** now carries meaning: 0 = "legacy / unknown", N = the last migration applied. All shipped DBs read 0 (the app has never set it). **Area A and every other area must not use `user_version` for anything else.**
- **`0001_baseline.sql`** holds, in order: the exact contents of today's `001_init.sql`, then every inline ALTER from `Storage::new` on `main` (002…"009" distance_km), then `CREATE TABLE IF NOT EXISTS session_power_curves (…)` (copied verbatim), then `CREATE TABLE IF NOT EXISTS app_meta (key TEXT PRIMARY KEY, value TEXT)`. The `app_meta` DDL must match Area A's byte-for-byte in column definition; see Cross-area. Column order on a fresh DB is therefore identical to an upgraded legacy DB.
- **Baseline execution**: split on `;` at end of statement (the baseline has no `;` inside literals; a test pins the statement count). Each statement runs on the migration transaction. `duplicate column name` errors are tolerated **in step 1 only**. Spike: SQLite keeps the transaction open after that statement error (verified on SQLite 3.51.2, see Risks).
- **Steps ≥ 2** run as one `sqlx::raw_sql` batch with no error tolerance.
- **Runner**: `pool.begin_with("BEGIN IMMEDIATE")` (sqlx-core 0.8.6 `Pool::begin_with`) per step. The runner re-reads `user_version` inside the transaction, so a second app instance starting at the same moment waits on the write lock (busy_timeout) and then sees the step already applied. It applies the step, runs `PRAGMA user_version = N`, and commits. `user_version` changes are transactional: the spike showed a rollback restoring 0.
- **DB newer than the binary** (`user_version > MIGRATIONS.len()`, i.e. a downgrade): log a `warn!` and continue without applying anything. Migrations are additive by policy, so older code keeps working.

### Rust API changes (all `pub(crate)` within the backend; no Tauri command or TS type changes)

```rust
pub enum SaveOutcome { Inserted, AlreadyExisted }
Storage::save_session(&self, &SessionSummary, raw: &[u8]) -> Result<SaveOutcome, AppError>
Storage::finalize_session(&self, &SessionSummary, &[SensorReading]) -> Result<(), AppError>
Storage::remove_autosave(&self, id: &str) -> Result<(), AppError>   // was: fn -> ()
Storage::delete_session(&self, id: &str) -> Result<(), AppError>    // same signature, new ordering
SessionManager::autosave_ticket(&self) -> Option<AutosaveTicket<'_>> // holds gate guard + id/summary/delta
prerequisites::build_fix_invocation(&PrereqStatus, Option<PackageManager>) -> Option<FixInvocation>
// FixInvocation { program: "pkexec", args: Vec<String>, stdin: Option<&'static str> }
fn format_panic(payload: &(dyn Any + Send), location: Option<&Location>) -> String
```

`stop_session`'s error text changes (see ADR-2). Its success contract is unchanged.

### Raw file write contract

`<data>/sessions/<id>.bin` is written only when it does not exist. The write goes to `<id>.bin.tmp`, then `rename`. A single writer per session id is guaranteed: recovery runs in `setup` before `app.manage(state)`, so no command can run yet, and a live session's id is a fresh UUIDv4. That makes check-then-rename sufficient.

## Decisions

### ADR-1 — Migration mechanism (**ONE-WAY DOOR**)

**Context.** Shipped DBs exist in several states: only `001` (first release), partially ALTERed, fully current, and current + `app_meta` if Area A lands first. None has `_sqlx_migrations`. SQLite has no `ADD COLUMN IF NOT EXISTS`.

**Options.**
- **A: `sqlx::migrate!` with a baseline.** `Migrator::run` creates `_sqlx_migrations` and applies every migration whose version isn't recorded (sqlx-core 0.8.6 `migrate/migrator.rs` `run_direct`). On a legacy DB the baseline's `ALTER TABLE ADD COLUMN` would fail with a duplicate column, and the whole migration fails because sqlx executes the file as one statement batch. Adopting legacy DBs would mean pre-seeding `_sqlx_migrations` rows ourselves, with sqlx's private table layout and SHA-384 checksums. Every later start also re-validates the checksums: `if migration.checksum != applied_migration.checksum { return Err(MigrateError::VersionMismatch) }`, where the checksum is `Sha384::digest(sql.as_bytes())`. Any byte edit to a shipped migration (a comment, CRLF from git `autocrlf`) then makes the app refuse to open the user's DB.
- **B: hand-rolled ordered list keyed by `PRAGMA user_version`, frozen idempotent baseline as step 1.** About 60 LOC. No hidden tables. A legacy DB at 0 runs the baseline, which is safe for every state because it *is* today's startup code.
- **C: keep inline ALTERs, add `IF NOT EXISTS` checks via `pragma_table_info`.** Idempotent but still not ordered. Schema lives in Rust strings, and each future change needs ad-hoc guards.

**Decision.** B.

**Why.** It is the only option that adopts every shipped DB without guessing its state. Startup has no checksum failure mode, the dependency surface is minimal, and from step 2 on each change is one plain `.sql` file.

**Reversibility: one-way.** Once a release ships with `user_version = 1`, the meaning of `user_version` and the content of `0001_baseline.sql` are frozen forever. Moving to sqlx later is still possible: a future step N would seed `_sqlx_migrations`. Re-numbering or editing shipped steps is not. Guard tests: `migration_files_match_registry` (no dead files) and `upgraded_main_db_columns_equal_fresh_db`.

### ADR-2 — Stop ordering for E2

**Context.** Today `save_session(...)?` fails, the session is already `take()`n, and the log is dropped. Only the last periodic autosave (up to 30 s old) survives.

**Options.** (A) write a full final autosave first, then save, then remove the autosave. (B) write the raw `.bin` first, then insert the row. (C) put the session back into the manager on failure.

**Decision.** A, inside `Storage::finalize_session`:
1. Write a full autosave.
2. `save_session`.
3. `remove_autosave`.

A failure at step 1 is logged and the save still proceeds. A failure at step 2 returns `AppError::Session("Ride could not be saved: {e}. It was kept on disk and will be recovered next time the app starts.")`. If step 1 also failed, the message says the last autosave (up to 30 s old) will be recovered. A failure at step 3 is logged at warn and returns Ok, because recovery cleans up.

**Why.** It reuses the existing recovery channel, needs no new format, and works for the window-close path too. B produces a file without a row, plus a stale-summary problem. C resurrects a stopped session in the UI and still loses data on crash.

**Reversibility: easy.**

### ADR-3 — E1 safety: ordering + ensure-semantics, no tombstones

**Context.** Two problems:
- `INSERT OR IGNORE` followed by an unconditional file write lets a stale autosave overwrite the final log.
- The writer can recreate an autosave after stop removed it (race: snapshot → stop → remove → rename). Recovery can then re-insert a session the user deleted.

**Options.** (A) tombstone table `deleted_sessions(id)` checked by recovery and save. (B) close every path that lets a stale autosave outlive the session's DB row, and make recovery never overwrite. (C) check row existence only.

**Decision.** B:
- **Gate**: the writer holds `autosave_gate` across snapshot + write, and stop takes the gate before `take()`. No autosave write can start after stop, and none is in flight once stop proceeds.
- **Ensure-semantics**:
  - Recovery inserts the row only if absent and writes the raw file only if absent. A row that is present but has no file gets its file from the autosave (crash between INSERT and write).
  - The autosave is then deleted. Only `Inserted` counts as recovered.
- **Delete ordering**:
  1. Remove `.autosave_<id>.bin` (NotFound ok; any other error aborts with nothing changed).
  2. One transaction: delete `session_power_curves`, then `sessions`.
  3. Remove `<id>.bin`. Failure is `log::error!` with the path and Ok, since the DB is already consistent and an orphan file is invisible.
- **Recovery runs before the UI exists**, so it always settles leftovers before any delete can happen.

**Why.** Every stale autosave is either removed by stop or removed at the next startup, while the row still exists and before the user can delete. A delete can't leave one behind (step 1 aborts). Tombstones would add a table and a forever-growing list to defend a path that ordering already closes. C alone leaves the race.

**Reversibility: easy.** Tombstones can be added later as migration 0002 if a new path appears.

Note: this reverses #38's "file first" delete order. The spec requires the DB to be atomic first, and the orphan-file cost is accepted and logged.

### ADR-4 — Privileged install with one prompt, no world-writable path

**Context.**
- The current code copies the rule to the predictable `/tmp/99-ant-usb.rules`, then runs 3–5 separate `pkexec` calls.
- The upstream polkit policy for `org.freedesktop.policykit.exec` is `auth_admin`, not `auth_admin_keep` (checked in `/usr/share/polkit-1/actions/org.freedesktop.policykit.policy`, polkit 127), so **each call prompts**.
- Tauri resource dir on Linux is `/usr/lib/<exe>` for deb/rpm and `${APPDIR}/usr/lib/<exe>` for AppImage (tauri-utils 2.8.2 `platform::resource_dir` docs). `${APPDIR}` is the user's FUSE mount, which root can't read without `allow_other`, which is why the `/tmp` copy exists.

**Options.**
- **A: `pkexec /bin/sh -c "$SCRIPT" training-app-setup <tokens…>`**, with `$SCRIPT = include_str!("../resources/linux-setup.sh")` and the rule body piped on stdin.
  - The script is a compile-time constant. All variable data is argv tokens from a fixed allowlist (`udev`, `bluez=apt-get|dnf|pacman`, `bluetooth`) and stdin. Nothing is interpolated into a shell string.
  - No file is created anywhere, so no FUSE or noexec issue.
  - pkexec leaves fds 0–2 open; only fds ≥ 3 are set close-on-exec (`fdwalk_close_on_exec (3)` in polkit `src/programs/pkexec.c`).
  - `install -m 0644 /dev/stdin <dest>` works with GNU coreutils 9.10 (spike).
- **B: helper script bundled as a resource and run with `pkexec <path> <tokens>`.** Root can't read it on AppImage (FUSE), so it would need a copy into a private `0700` `tempfile::TempDir`. `/tmp` may be `noexec`, which forces `pkexec /bin/sh <path>` anyway. That leaves two code paths plus a TOCTOU window on the copied script.
- **C: `pkexec install … <read-only bundle path>` per step.** Fixes the `/tmp` path for deb/rpm only, fails on AppImage, and still prompts several times.

**Decision.** A.

**Note on the brief.** The brief said "one `pkexec sh -c` is NOT acceptable because of interpolation". That concern applies to building the script from paths. Here the script text is a constant compiled into the binary, verbatim from a reviewed file, and every argument is passed as `$1…$n` with no `eval`. The prompt shows `/bin/sh` as the program, which is the one UX cost. If the lead still rejects A, the fallback is B with a private `0700` TempDir, executed via `pkexec /bin/sh <tmpdir>/linux-setup.sh <tokens>`. That fallback is about +40 LOC.

**Script contract** (`resources/linux-setup.sh`, POSIX sh):
- `set -eu`; `PATH=/usr/sbin:/usr/bin:/sbin:/bin`.
- **Validate all args first** with a `case` statement. An unknown token exits 2 before any change is made.
- Then, in order:
  - `udev`: `install -m 0644 -o root -g root /dev/stdin /etc/udev/rules.d/99-ant-usb.rules`, then `udevadm control --reload-rules`, then `udevadm trigger`.
  - `bluez=<pm>`: the fixed install line for that package manager.
  - `bluetooth`: `systemctl enable --now bluetooth`.
- Each step `echo`s its name to stderr and exits with a distinct code (10/11/12/20/30). pkexec reserves 126 (dialog dismissed) and 127 (not authorized) per pkexec(1).
- **Rust side**: the rule content becomes `include_str!("../resources/99-ant-usb.rules")`, so the resource dir is no longer needed for the fix. The resource stays bundled for manual install, and `tauri.linux.conf.json` is unchanged. `fix()` maps the exit code to a message: 126 → "Authentication cancelled".

**Reversibility: easy.**

### ADR-5 — Logger flush and panic hook

**Context.**
- flexi_logger 0.29.8 `WriteMode` docs: "for all options except `Direct` you should keep the `LoggerHandle` alive up to the very end of your program to ensure that all buffered log lines are flushed out (which happens automatically when the `LoggerHandle` is dropped)".
- `BufferAndFlush` = 8 KiB buffer + 1 s flusher thread.
- Tauri 2.10.2 `App::run`: "This function never returns. When the application finishes, the process is exited directly using `std::process::exit`." So a leaked or static handle is never dropped.

**Options.** (A) keep `BufferAndFlush`, store the handle in `static LOGGER: OnceLock<LoggerHandle>`, and call `flush()` on `RunEvent::Exit` and in the panic hook. (B) switch to `WriteMode::Direct` (no buffer, no flush needed).

**Decision.** A. Replace `builder.run(ctx)` with `builder.build(ctx).expect(..).run(|_app, ev| if let RunEvent::Exit = ev { flush_logger() })`.
- The panic hook is installed right after `start()`. It does `prev = take_hook()`, then `set_hook(move |info| { log::error!("{}", format_panic(info.payload(), info.location())); flush_logger(); prev(info) })`.
- `format_panic` downcasts `&str`, then `String`, else returns `"<non-string panic payload>"`, and appends ` at file:line:col`.

**Why.** A keeps the current I/O profile and meets the spec literally. B is a one-line alternative if A misbehaves on macOS, where `Exit` is delivered before `process::exit`. Hook and flush are both unit-tested at the `format_panic` level only; flush on exit is a manual check.

**Reversibility: easy.**

### ADR-6 — Upgrade test fixtures

**Decision.**
- **Fixture.** Generate `src-tauri/src/session/storage/testdata/main_<sha>.sql`, a full `.dump` (schema + rows) of a DB created by **unmodified `main` code** with seeded data (two sessions with every column populated, a custom `user_config`, a known device with metadata, power-curve rows, and an `app_meta` row if A has landed).
- **Generation.** Produce it in slice 1 *before* changing `Storage::new`: a throwaway `#[ignore]` test that runs today's `Storage::new` and inserts the rows, then `python3 -I` `iterdump`. The one-off script stays in the scratchpad.
- **Hand-written fixtures.** `legacy_v001_only.sql` (only `001_init` tables + one session) and `legacy_partial_005.sql` (ALTERs 002–005 only), for older shipped states.
- **Rebase rule.** Whichever of A/E merges second regenerates `main_<sha>.sql` from latest `main`.

**Reversibility: easy.**

## Failure modes

| Failure | Detection | Behavior | Recovery |
|---|---|---|---|
| Baseline statement fails (not dup column), e.g. disk full | `sqlx::Error` | Transaction rolled back, `user_version` stays 0, `Storage::new` errors, app shows startup failure as today | Next launch retries the whole step |
| Two instances start together | `BEGIN IMMEDIATE` blocks | Second waits (busy_timeout 5 s), re-reads version, skips | n/a |
| DB `user_version` > known | Compare at startup | `warn!`, continue | Upgrade binary |
| Stop: final autosave write fails | `Err` from `write_autosave` | `warn!`, continue to DB save | Previous ≤30 s autosave still on disk |
| Stop: DB insert / raw write fails | `Err` from `save_session` | Command returns explanatory `Err`; autosave (full) retained | Startup recovery inserts row / writes missing file |
| Stop: autosave removal fails | `Err` (not NotFound) | `warn!`, Ok | Startup recovery sees row exists and deletes autosave |
| Crash between any two stop steps | Autosave present at startup | Recovery ensures row + file, never overwrites | Automatic |
| Recovery: corrupt / invalid-id autosave | Parse / validate | Delete file, `warn!` (unchanged) | n/a |
| Recovery: autosave removal fails | `Err` | `error!`, file left | Retried next startup; delete aborts if still present |
| Delete: autosave removal fails | `Err` | Delete aborts before DB is touched; error to UI | User retries |
| Delete: DB transaction fails | `sqlx::Error` | Rolled back, no partial state; raw file untouched | User retries |
| Delete: raw file removal fails after commit | `io::Error` | `error!` with path, Ok | Orphan file (invisible), manual cleanup |
| pkexec dismissed / denied | Exit 126 / 127 | `FixResult{success:false, "Authentication cancelled/denied"}` | Retry |
| Setup step fails | Exit 10–30 + stderr | Message names failing step; status re-checked | Retry / manual |
| Panic in any thread | Panic hook | Logged at error + flushed, then default hook | n/a |

## Migration and rollback

- **Forward.**
  - First launch of the new build on any legacy DB runs step 1 (no-op on a current DB, adds columns on older ones) and sets `user_version = 1`.
  - No data is rewritten. Existing autosave files are handled by the new recovery on that same launch.
- **Rollback to an older build.** Older code ignores `user_version` and re-runs its idempotent inline DDL against the same tables, so it works unchanged. A later re-upgrade sees `user_version = 1` and skips the baseline.
- **Future schema changes.** Add `migrations/000N_<name>.sql`, register it in `MIGRATIONS`, never edit a shipped file, additive changes only. This is documented in `CONTRIBUTING.md` and `CLAUDE.md`.

## Traceability

| AC | Component(s) | Test(s) (planned names) |
|---|---|---|
| AC1 | sessions.rs, autosave.rs, manager.rs, autosave_task.rs | `save_session_existing_row_keeps_original_raw_file`, `save_session_returns_inserted_then_already_existed`, `recovery_for_existing_session_keeps_final_log_and_deletes_autosave`, `recovery_for_existing_row_without_file_writes_file_from_autosave`, `deleted_session_is_not_resurrected_by_restart` (save → stale autosave left → restart recovery → delete → restart → absent), `delete_session_removes_leftover_autosave`, `stop_waits_for_in_flight_autosave_ticket`, `autosave_tick_after_stop_writes_nothing`, `remove_autosave_missing_file_is_ok` |
| AC2 | autosave.rs `finalize_session`, commands.rs, lib.rs | `finalize_with_failed_db_save_keeps_full_log_on_disk` (close pool → finalize Err → reopen Storage → recover → `load_sensor_data` has all N readings), `finalize_error_message_mentions_recovery`, `finalize_success_removes_autosave` |
| AC3 | prerequisites.rs, resources/linux-setup.sh, commands.rs | `fix_invocation_all_missing_is_single_pkexec_call`, `fix_invocation_udev_pipes_embedded_rule_on_stdin`, `fix_invocation_args_contain_no_paths`, `fix_invocation_all_met_is_none`, `fix_invocation_script_arg_is_embedded_constant`, `setup_script_rejects_unknown_token_before_any_change` (runs `/bin/sh -c SCRIPT x bogus` unprivileged, asserts exit 2), `pkexec_exit_126_maps_to_cancelled`; **manual**: deb + AppImage on Fedora, one prompt, rule installed 0644 root:root |
| AC4 | sessions.rs | `delete_session_failed_transaction_leaves_row_and_curves` (a test-only `BEFORE DELETE ON sessions` trigger does `RAISE(ABORT)`, so the second DELETE fails. Assert the row and the power-curve rows are still present and the raw file is untouched.), `delete_session_removes_row_curves_and_file` |
| AC5 | lib.rs | `format_panic_str_payload_includes_location`, `format_panic_string_payload`, `format_panic_non_string_payload`; **manual**: log ends with "Application exiting" line after window close; forced test panic appears in log file |
| AC6 | schema.rs, 0001_baseline.sql, docs | `fresh_db_reaches_latest_user_version`, `migrate_twice_is_noop`, `upgrade_from_main_dump_preserves_all_rows` (row-by-row equality of every table), `upgraded_main_db_columns_equal_fresh_db`, `upgrade_from_v001_only_db_adds_all_columns_and_keeps_session`, `upgrade_from_partial_005_db_succeeds`, `baseline_tolerates_only_duplicate_column_errors` (non-dup failure rolls back, `user_version` stays 0), `newer_user_version_is_left_untouched`, `migration_files_match_registry`, `baseline_statement_count_is_pinned` |
| AC7 | ci.yml, release.yml | CI run on the PR shows "Run frontend tests" step with 71+ tests |
| AC8 | all | `cargo test`, `npm test`, `npm run check` green before every commit; each new test above seen failing before its fix |

## Risks and spikes

- **Riskiest assumption:** the baseline really is a no-op on every shipped DB and SQLite keeps the transaction usable after a duplicate-column error. *Spike done (Python sqlite3, SQLite 3.51.2):* the error leaves `in_transaction=True`, later statements commit, and `PRAGMA user_version` rolls back with the transaction. sqlx bundles its own libsqlite3, so the slice-1 tests re-prove this under sqlx.
- **pkexec stdin pass-through:** confirmed from source (fds 0–2 inherited). The textual agent fallback (no GUI agent) reads `/dev/tty`, not stdin. **Manual check required** on GNOME (Fedora) for the GUI agent. `install /dev/stdin` was verified with GNU coreutils 9.10. Non-GNU `install` (BusyBox) is out of scope (not a target distro).
- **`RunEvent::Exit` on macOS:** Tauri docs state that Exit fires before `process::exit`. Manual check on macOS CI artifact; fall back to ADR-5 B if it doesn't.
- **Forcing DB failures in tests:** use `storage.pool.close().await` for save failures and a `BEFORE DELETE` trigger `RAISE(ABORT)` for transaction rollback. Both are public SQL, no internals.

## Implementation plan

1. **Fixture capture (no behaviour change).** Generate `testdata/main_3b10a73.sql` from unmodified code (ADR-6) and hand-write the two legacy fixtures. Commit: "Add legacy database fixtures for schema upgrade tests".
2. **Migration runner + baseline (riskiest).** Add `schema.rs` and `0001_baseline.sql`, delete `001`–`008`, and switch `Storage::new` to `migrate()`. Tests: all AC6 tests. Commit: "Consolidate schema into ordered user_version migrations".
3. **Ensure-semantics save + recovery.** `SaveOutcome`, atomic write-if-absent, recovery rules, `remove_autosave -> Result`. Tests: AC1 storage tests. Commit: "Never overwrite ride logs during autosave recovery".
4. **Autosave gate + task extraction.** Gate in `SessionManager`, `autosave_ticket`, `autosave_tick` extracted from `lib.rs`. Tests: `stop_waits_for_in_flight_autosave_ticket`, `autosave_tick_after_stop_writes_nothing`. Commit: "Serialize autosave writes with session stop".
5. **finalize_session + stop/close wiring.** Tests: AC2. Commit: "Keep ride data on disk when saving on stop fails".
6. **Delete ordering + transaction.** Tests: AC4 + `delete_session_removes_leftover_autosave` + `deleted_session_is_not_resurrected_by_restart`. Commit: "Delete sessions atomically and remove leftover autosaves".
7. **Privileged setup.** `linux-setup.sh`, `build_fix_invocation`, `fix_prerequisites` without `/tmp`. Tests: AC3 + manual check recorded in design.md Verification. Commit: "Install udev rule via single pkexec call without temp files".
8. **Logger.** `OnceLock` handle, `RunEvent::Exit` flush, panic hook, `format_panic`. Tests: AC5 + manual. Commit: "Flush logs on exit and log panics".
9. **CI + docs.** `npm test` in both workflows; "Schema changes" sections in CONTRIBUTING.md and CLAUDE.md. Commit: "Run frontend tests in CI and document schema changes".

## Cross-area dependencies

- **Area A (`app_meta`).** E's baseline contains `CREATE TABLE IF NOT EXISTS app_meta (key TEXT PRIMARY KEY, value TEXT)`.
  - If A merges first, E removes A's inline DDL on rebase, keeps the definition identical, and regenerates the `main` fixture with an `app_meta` row.
  - If E merges first, A drops its inline `CREATE` (the table already exists) and only reads and writes rows. Any further schema A needs goes in `migrations/0002_*.sql`.
  - A must not use `PRAGMA user_version`.
- **Area D (frontend).** After a stop error, `requestStop` (`src/lib/stores/session.ts`) leaves `sessionActive = true` although the backend already dropped the session, and `autoSession.ts` swallows the error with `.catch(() => {})`. D should reset the session stores on error and surface the backend message (toast or banner). E only guarantees the backend `Err` text.
- **Area C (device).** None. `prerequisites.rs` is E's.
