# Device layer (BLE / ANT+ / trainers) — Spec

**Track:** Standard   **Product stage:** Introduction/early Growth
**Status:** Agreed   **Last updated:** 2026-10-18
**Findings:** C1–C17

## Problem
Sensor and trainer I/O has protocol and lifecycle bugs: ANT+ simulation mode sends grade at the wrong byte offset (every sim ride feels ~5.5 %), an invalid FE-C power value is recorded as 4095 W, separate speed and cadence sensors can't both report, ANT+ reconnects leak channels and then show Connected with no data, reconnect/connect races duplicate listeners, and long scans/reconnects hold the device lock and stall trainer commands. Several common setups (power-meter cadence, combined ANT+ speed/cadence sensors, FTMS trainers with non-0–100 resistance ranges) are unsupported.

## Users
All riders with sensors; ANT+ trainer users in sim mode; riders with separate speed/cadence pods, pedal/crank power meters, or ANT+ combo S&C sensors.

## Goals
- Protocol encodings/decodings match the Bluetooth SIG and ANT+ device profiles.
- Device lifecycle (connect, drop, reconnect, disconnect) never leaks channels/listeners and never shows Connected for a device that isn't delivering data.
- Device I/O never blocks trainer commands for longer than a bounded time.

## Non-goals
- New device types beyond those listed (e.g. no muscle-oxygen, no radar).
- UI redesign of the Devices page (frontend only gets data it can already render).

## Acceptance criteria
- [ ] AC1 (C1): FE-C page 0x33 encodes `[0x33, 0xFF, 0xFF, 0xFF, 0xFF, grade_lo, grade_hi, crr]` per ANT+ FE-C profile (grade 0.01 % units, offset −200 %; Crr 5e-5 units). If wind coefficient is supported, it's sent on page 0x32 per spec. Byte-level tests assert spec layout for 0 %, −3 %, +5 %, and a hand-computed Crr.
- [ ] AC2 (C10): Grade encoding rounds to nearest 0.01 % (2.3 % → raw 230 + offset) and can't overflow for any input in the clamped range, for both FE-C and FTMS.
- [ ] AC3 (C5): An FE-C page 0x19 instantaneous power of 0xFFF produces no Power reading.
- [ ] AC4 (C3): With a speed-only and a cadence-only sensor connected (any transport), both speed and cadence readings reach the session; primary-device selection still de-duplicates two devices that provide the *same* metric. Primary selection persisted today must keep working after upgrade (backward compatible).
- [ ] AC5 (C2): When the watchdog detects a stale ANT+ device, the old channel is closed and its listener stopped before any reconnect; a reconnect is only reported as Connected once a data page is received; a device that never returns is shown as reconnecting/disconnected, not Connected; no duplicated readings when it returns. Channel search timeout handling means a device returning after 30 s is still picked up (design decides: infinite/low-priority search or reopen on EVENT_CHANNEL_CLOSED).
- [ ] AC6 (C4): `connect()` for an already-connected device is a no-op success; a manual connect clears any pending auto-reconnect for that device; any existing listener handle is aborted before a new one is stored. Test: connect twice → one listener.
- [ ] AC7 (C6): FE-C acknowledged sends wait (bounded, ≤ 1 s) for EVENT_TRANSFER_TX_COMPLETED / TX_FAILED; failure is retried once then returned as an error.
- [ ] AC8 (C7): Scan and reconnect I/O does not hold the DeviceManager lock for its duration; a trainer command issued during a 4 s scan or reconnect attempt completes within 500 ms (test with a fake/slow transport or a structural test of lock scope).
- [ ] AC9 (C8): If service discovery or classification fails after a BLE connect, the peripheral is disconnected before the error is returned.
- [ ] AC10 (C9): The first CSC notification after connect/reconnect initialises state and emits no cadence/speed.
- [ ] AC11 (C11): Cadence is decoded from BLE Cycling Power Measurement crank revolution data (flag bit 5, with correct walking of preceding optional fields per flags) and from ANT+ power page 0x10 byte 3 (0xFF = invalid); hand-computed byte-level tests for each, including counter rollover.
- [ ] AC12 (C12): ANT+ combined speed & cadence sensors (device type 121, period 8086) are discoverable and produce both speed and cadence readings, with rollover handled.
- [ ] AC13 (C13): FTMS resistance level is scaled into the trainer's Supported Resistance Level Range (0x2AD6) read at connect; falls back to current behaviour if the characteristic is absent.
- [ ] AC14 (C14): ANT+ HR (and legacy S&C) background pages 2/3 (manufacturer/serial, HW/SW version) are decoded with the page-toggle bit masked, populating device metadata.
- [ ] AC15 (C15): BLE pedal balance with unknown reference reports None (matching ANT+).
- [ ] AC16 (C16): Device dedup only matches an ANT device number against a BLE name on a whole-token boundary; a short ANT number doesn't match an unrelated BLE name containing those digits.
- [ ] AC17 (C17): An ANT+ scan can discover more than one device of the same profile (design decides mechanism, e.g. re-opening the wildcard channel after a device is found, excluding found device numbers).
- [ ] AC18: All existing tests pass; each AC has a test that fails before and passes after; protocol tests use hand-computed byte vectors with a cited spec section.

## Constraints
No hardware in CI — all behaviour proven with byte-level and fake-transport tests. Existing `known_devices` rows and persisted primaries must keep working.

## Assumptions and open questions
- Assumption: ANT+ FE-C profile rev 5.x and Bluetooth SIG CPS/CSC/FTMS v1.0 are authoritative; implementer cites the section in test comments.
- Open: hardware validation of C1/C11–C13 — owner: user (manual check on real devices after merge); default: ship on spec-conformant tests, note in PR.
