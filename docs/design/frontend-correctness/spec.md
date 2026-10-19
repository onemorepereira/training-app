# Frontend correctness — Spec

**Track:** Standard   **Product stage:** Introduction/early Growth
**Status:** Agreed   **Last updated:** 2026-10-18
**Findings:** D1–D16 (+ backend support for D4, D9)

## Problem
The UI ends rides when the rider presses Pause, auto-stop leaves the trainer under zone control, zone-ride targets for the top zones are wrong (HR Z5 targets max HR), users can't clear ride notes/type/RPE, analytics buckets rides on the wrong day/week outside UTC, quick trainer adjustments are dropped, completed zone rides aren't saved, and a webview reload leaves the UI unable to stop a running ride. Several smaller leaks and display inconsistencies add up over long rides.

## Users
All riders using the dashboard, zone rides, trainer control, history and analytics.

## Goals
- Every way a session ends goes through one stop path.
- What the UI shows matches backend state and the user's local calendar.

## Non-goals
- Visual redesign; new pages.
- Backend zone-controller behaviour (Area B) beyond the commands this area needs.

## Acceptance criteria
- [ ] AC1 (D1): While paused, auto-stop never fires. A channel is "fresh" if it had a reading within the last 5 s (frontend last-seen timestamps). If speed is fresh, the rider is idle when `speed === 0`; else if cadence is fresh, idle when `cadence === 0`; else (no sensors or sensor dropout) auto-stop never fires. Auto-stop fires after 3 consecutive idle 1 s ticks. *(Amended 2026-10-18 per design review ruling 4.)*
- [ ] AC2 (D2, D7): A single `endSession()` path (used by the Stop button and auto-stop) stops zone control, saves the zone-ride config/result, and opens the post-ride modal. A zone ride that completes on its own (DurationComplete / SafetyStop / sensor lost) finishes the zone ride but does **not** end the session: its result is held and saved when the session ends, the stop reason or safety note is shown to the user, and zone-status polling stops. *(Amended 2026-10-18 per design review.)*
- [ ] AC3 (D3): `resolveZoneBounds` HR Z5 = [hz[3], hz[4]]; Power Z7 lower = pz[5]·FTP and upper > lower always (e.g. max(lower+1, 2·FTP)). Unit tests cover every zone 1–7 / 1–5 with default and custom configs.
- [ ] AC4 (D4): Clearing activity type, RPE or notes and saving persists the cleared value; an emptied title shows the default title. Backend `update_session_metadata` gains explicit clear semantics (design decides contract: full-replace or tri-state) and keeps working for existing callers.
- [ ] AC5 (D5): Day keys, week starts (Monday), "today" and "this week" are computed in the user's local timezone; a ride at 21:00 local Sunday in UTC−5 counts toward that Sunday's week. DST transitions don't skip or duplicate PMC days. Tests run with a fixed non-UTC TZ (e.g. `TZ=America/Chicago` in vitest config or per-test).
- [ ] AC6 (D6): Rapid ERG/resistance/grade changes while a send is in flight are coalesced: after the in-flight send resolves, the latest value is sent (exactly one follow-up send).
- [ ] AC7 (D8): When a zone ride ends, the live chart's zone band is removed.
- [ ] AC8 (D9): After a webview reload during an active (or paused) session, the UI shows the session as active/paused within 1 s and Stop works. Backend gains a `get_session_status` (or equivalent) command.
- [ ] AC9 (D10): Live chart history is sampled at 1 Hz (one point per second regardless of sensor count), holds ≥ 10 minutes, doesn't call `resize()` per event, and is reset when a new session starts.
- [ ] AC10 (D11, D14): Navigating away from Devices / unmounting PowerGauge / PowerCurve before async init completes leaves no listeners, ECharts instances or ResizeObservers alive.
- [ ] AC11 (D12): Weekly avg power/HR are duration-weighted; weeks with no rides appear with zero volume.
- [ ] AC12 (D13): HR histogram zone boundaries use the same inclusive-upper rule as the backend (`bpm <= upper`).
- [ ] AC13 (D15): A successful manual reconnect clears the device's reconnecting/disconnected banner.
- [ ] AC14 (D16): Toggling units in Settings converts the currently typed weight (not the saved one) and doesn't change the app-wide unit until Save.
- [ ] AC15: `npm test`, `npm run check` and `cargo test` pass; each AC covered by a vitest test where logic is testable (stores/utils), otherwise by a recorded manual check in the running app (`npm run tauri dev` with the dev simulator).

## Constraints
Svelte 5 runes conventions; typed API in `src/lib/tauri.ts`. Backend command changes must be backward compatible within the release (frontend and backend ship together, so compatibility is within one build).

## Assumptions and open questions
- Assumption: Area B makes the backend stop zone control on `stop_session`; this area must not depend on that being merged first (frontend still stops it explicitly).
