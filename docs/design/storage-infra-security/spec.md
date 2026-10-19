# Storage, infra & security — Spec

**Track:** Standard   **Product stage:** Introduction/early Growth
**Status:** Built   **Last updated:** 2026-10-18
**Findings:** E1–E7

## Problem
Ride data can be lost or corrupted at the edges of the storage lifecycle: autosave recovery can overwrite a finished ride with an older snapshot or resurrect a deleted ride, and a failed save on stop drops the in-memory log. The Linux setup fix copies a root-installed udev rule through a predictable `/tmp` path (local privilege-escalation risk) and prompts for a password several times. Logs lose their last lines at exit and panics are never logged. The schema lives in two places that have already diverged, and CI doesn't run the frontend tests.

## Users
All users (data safety); Linux users running the setup fix; contributors (schema, CI).

## Goals
- No sequence of stop / crash / restart / delete loses or resurrects ride data.
- Privileged setup runs with no attacker-controllable path and one authentication prompt.

## Non-goals
- Changing the `.bin` sensor-log format.
- Cloud backup or export features.

## Acceptance criteria
- [ ] AC1 (E1): Recovering an autosave for a session that already exists in the DB never overwrites its raw log; the stale autosave is deleted. Recovering an autosave for a session that was deleted does not recreate it (design decides the tombstone/ordering mechanism). `save_session` writes the raw file only when the row is newly inserted (or the design makes the write ordering otherwise safe). The autosave writer can't recreate an autosave file after stop has removed it.
- [ ] AC2 (E2): If saving on stop fails, the session's data is still on disk (autosave or raw log written before the in-memory session is dropped) and is recovered on next launch; the user gets an error message.
- [ ] AC3 (E3): The udev rule is installed without any world-writable intermediate path (e.g. private 0700 temp dir, or `pkexec install -m 0644 /dev/stdin …` / reading from the read-only bundle path); the whole fix (rule install, udevadm reload/trigger, BlueZ install, bluetooth enable) needs one authentication prompt. Arguments remain argv vectors (no shell string interpolation of paths).
- [ ] AC4 (E4): `delete_session` deletes the session row and its power-curve rows in one transaction, then removes the raw file; a failure leaves no partial state in the DB.
- [ ] AC5 (E5): The logger handle is kept and flushed on exit; a panic hook logs panics (message + location) at error level before the default hook runs.
- [ ] AC6 (E6): Schema is defined in exactly one place and every change is applied by a single, ordered, idempotent mechanism. Existing user DBs (any prior version, including ones where inline ALTERs already ran) upgrade without data loss — proven by a test that builds a DB with the current `main` schema and data, runs the new startup, and checks data intact. Dead/duplicate `.sql` files removed or made authoritative. README/CONTRIBUTING/CLAUDE.md "schema changes" guidance updated to match.
- [ ] AC7 (E7): CI and the release workflow run `npm test`.
- [ ] AC8: All existing tests pass; each AC has a test that fails before and passes after, except AC3's pkexec flow and AC5's exit flush, which get unit tests on the command construction plus a recorded manual check.

## Constraints
Schema change is the one-way door here: must be backward compatible with every shipped DB and coordinate with Area A's one-time recompute marker.

## Assumptions and open questions
- Decision: Area A adds an `app_meta(key, value)` table inline in `Storage::new` for its one-time recompute marker. Areas merge independently; whichever of A/E merges second rebases, and E's consolidated schema must include every inline schema object present on `main` at merge time (including `app_meta` if A landed first). E's upgrade test must cover a DB produced by the latest `main`.
