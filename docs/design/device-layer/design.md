# Device layer (BLE / ANT+ / trainers) — Design

**Spec:** ./spec.md   **Last updated:** 2026-10-18   **Status:** Draft

## Overview

Two kinds of work:

1. **Protocol fixes.** These are pure functions with byte-level tests against cited spec tables: C1, C5, C9, C10, C11, C12, C13 (decode part), C14, C15, C16. Each one is small and low risk.
2. **Lifecycle and concurrency fixes.** These cover C2, C3, C4, C6, C7, C8 and C17, and they hinge on three structural changes:
   - **I/O seam plus two-phase locking.** `DeviceManager` gets an `Io` type parameter (default `RealIo` = BLE + ANT). Scan, connect, disconnect and reconnect run as *prepare (lock) → I/O (no lock) → commit (lock)*, driven by associated functions on the shared `Arc<Mutex<DeviceManager>>`. A per-device "gate" mutex serialises operations on one device. Trainer commands are never queued behind a 4 s scan. A fake `Io` makes connect-twice, stale, reconnect and lock-scope behaviour testable without hardware.
   - **Persistent ANT+ channels.** Connected ANT+ slave channels open with *high-priority search disabled and low-priority search infinite*. A device that drops out is re-acquired by the radio on the same channel, so there is no close/reopen, no duplicate channel and no duplicated readings. The watchdog only flips the status Connected ⇄ Reconnecting from the last-data timestamp. Connect reports Connected only after the first data page arrives.
   - **Primaries keyed by metric** (Power, HeartRate, Cadence, Speed, Trainer) instead of `DeviceType`. A device claims a metric on its first reading of that metric (lazy claim) if no primary exists. The `set_primary_device` / `get_primary_devices` command contract stays backward compatible.

Note on the spec: primaries are **not persisted today**. They live only in memory (`Arc<RwLock<HashMap<DeviceType,String>>>`) and are re-derived on every connect. No stored primaries need migrating. Backward compatibility means the Tauri command contract plus the `known_devices.device_type` strings, which are unchanged. See ADR-3.

## Architecture

Components touched. All are under `src-tauri/src/device/` unless noted.

| Component | Change |
|---|---|
| `manager.rs` | Generic `DeviceManager<Io: DeviceIo = RealIo>`. Holds bookkeeping only: connected map, listener handles, trainer backends, reconnect targets, per-device gates, capabilities, primaries, storage. New associated fns `scan_shared`, `connect_shared`, `disconnect_shared`, `watchdog_tick` take `&Arc<Mutex<Self>>`. The existing sync getters and trainer command methods stay `&mut self`, which keeps the Area B call sites unchanged. |
| `io.rs` (new) | `trait DeviceIo` and `RealIo`, which owns `BleManager` (lazy init via `tokio::sync::OnceCell`) and `Arc<std::sync::Mutex<AntSlot>>`. A test-only `FakeIo` records an ordered event log. |
| `ant/manager.rs` | Dynamic scan-channel allocation. Multi-device scan via exclusion lists. Connect waits for the first page. Connect is idempotent. Unsolicited channel close is detected. Combined S&C profile. |
| `ant/channel.rs` | Search timeouts. `send_acknowledged_and_wait`. Channel-filtered drains. New message/event constants in `ant/usb.rs`. |
| `ant/listener.rs` | HR and legacy S&C background pages, with the toggle bit gate. Lazy primary claim. Combined S&C decode. Closed flag on exit. |
| `ant/protocol.rs` | 0xFFF power invalid. Power page 0x10 cadence. Combined S&C decoder. Background page decoders. |
| `fec.rs` | Page 0x33 layout. Page 0x32. Rounding. Ack wait plus one retry, using a per-channel send mutex. |
| `ftms.rs` | Grade rounding. Supported Resistance Level Range parse and scale. |
| `protocol.rs`, `listener.rs` (BLE) | `CscState` with initialised flags. CPS decoder becomes stateful (`CpsState`) with field walking and crank cadence. Unknown balance reference → `None`. Lazy primary claim. |
| `ble.rs` | Disconnect-on-error after `connect()`. `BleManager: Clone`, which is cheap because its fields are an `Adapter` plus `Arc`s. Read 0x2AD6 for trainers. |
| `types.rs` | `Metric` enum, `Primaries` type, `admit_reading()` replacing `is_dominated()`, `Capabilities`. |
| `dedup.rs` | Whole-token match. |
| **Outside `device/`** (minimal, unavoidable) | `lib.rs` watchdog becomes a single `DeviceManager::watchdog_tick(&dm, &tx)` call plus the existing emits. `commands.rs`: `scan_devices`, `connect_device` and `disconnect_device` call the `*_shared` fns; `set_primary_device` / `get_primary_devices` map through `PrimaryKey`. The `AppState.primary_devices` type changes. |

Existing patterns followed: blocking ANT work in `spawn_blocking`; lock-free `AtomicI64` last-seen; the router thread dispatching per-channel `mpsc`; the `ReconnectManager` backoff; inline `#[cfg(test)]` modules; `assert_approx`.

```mermaid
sequenceDiagram
  participant C as command / watchdog
  participant DM as DeviceManager (Mutex)
  participant G as device gate
  participant IO as RealIo (BLE / ANT lock)
  C->>DM: lock: prepare (already connected? clear reconnect, get gate)
  DM-->>C: plan + gate (lock released)
  C->>G: lock gate (serialises this device only)
  C->>DM: lock: re-check state (still needed?)
  C->>IO: connect / scan / disconnect I/O (no DM lock)
  IO-->>C: outcome
  C->>DM: lock: commit (abort old listener, insert, claims)
```

## Data and contracts

**Rust types (`types.rs`)**

```rust
pub enum Metric { Power, HeartRate, Cadence, Speed, Trainer }
pub type Primaries = Arc<std::sync::RwLock<HashMap<Metric, String>>>;
/// Metrics a device may auto-claim as primary (claimed lazily on first reading).
pub type Capabilities = Arc<std::sync::RwLock<HashMap<String /*device_id*/, MetricSet>>>;
impl SensorReading { pub fn metric(&self) -> Option<Metric> }   // Cadence→Cadence, Speed→Speed, TrainerCommand→None
/// Replaces is_dominated. Returns true if the reading should be forwarded.
/// primary==device → true; primary==other → false; no primary → claim if metric ∈ caps(device), forward either way.
pub fn admit_reading(primaries: &Primaries, caps: &Capabilities, r: &SensorReading) -> bool
```

- `DeviceType` stays as it is. `CadenceSpeed` is still the stored and displayed type, so `known_devices` rows and the TS `DeviceInfo.device_type` union are untouched.
- Capability table:

  | Device | Claimable metrics |
  |---|---|
  | HR | {HeartRate} |
  | Power (BLE CPS / ANT 11) | {Power}. Power-meter cadence is *not* claimable, so a dedicated cadence sensor wins. It still flows when no cadence primary exists. |
  | CadenceSpeed | {Cadence, Speed}. Lazy claim means a speed-only pod never claims Cadence. ANT 122 → {Cadence}, 123 → {Speed}, 121 → {Cadence, Speed}. |
  | FitnessTrainer | {Trainer}. Trainer power, cadence, HR and speed are not claimable, which matches today: trainer data flows only when no dedicated sensor is primary for that metric. |

**Tauri commands (backward compatible)**

- `set_primary_device(device_type: PrimaryKey, device_id)`. `PrimaryKey` deserialises from the legacy strings `"HeartRate" | "Power" | "CadenceSpeed" | "FitnessTrainer"` and the new `"Cadence" | "Speed"`.
  - Legacy `"CadenceSpeed"` sets the primary for every metric in `capabilities(device_id) ∩ {Cadence, Speed}`. If capabilities are unknown, for example the device is not connected, it sets both.
  - `"FitnessTrainer"` → `Metric::Trainer`.
- `get_primary_devices() -> Record<string,string>` returns the legacy keys plus the new keys:
  - `HeartRate`, `Power`, `FitnessTrainer` as today.
  - `CadenceSpeed` = Cadence primary, falling back to Speed primary.
  - `Cadence`, `Speed`.
  - The existing Devices page `primaryDevices[device.device_type]` keeps working. Extra keys are ignored.
- **Cross-area (D):** to show "primary" correctly for two separate speed/cadence pods, the Devices page should look up `Cadence` / `Speed` for `CadenceSpeed` devices. That is optional; there is no breakage without it.

**Events (unchanged names and payloads):** `device_disconnected(id)`, `device_reconnecting{device_id, device_type, attempt}`, `device_reconnected(id)`, `device_list_updated(DeviceInfo[])`.
- An ANT+ device whose data goes stale emits `device_disconnected` once, then `device_reconnecting` on each watchdog tick while stale, then `device_reconnected` when pages resume.
- While stale it stays in `connected_devices` with `status: Reconnecting`, a value the TS union already includes.

**`DeviceIo` trait (`io.rs`), using return-position `impl Future` (stable since 1.75; MSRV 1.77.2)**

```rust
pub trait DeviceIo: Send + Sync + 'static {
    fn scan(&self) -> impl Future<Output = ScanOutcome> + Send;                 // BLE+ANT concurrently
    fn connect(&self, id: &str, ctx: ListenerCtx) -> impl Future<Output = Result<Connected, AppError>> + Send;
    fn disconnect(&self, id: &str) -> impl Future<Output = Result<(), AppError>> + Send;
    fn probe(&self, ids: Vec<String>) -> impl Future<Output = Vec<(String, LinkState)>> + Send; // BLE is_connected, ANT last_seen/closed
    fn ant_metadata(&self) -> Option<Arc<StdMutex<HashMap<String, AntDeviceMetadata>>>>;
}
pub struct ListenerCtx { tx: broadcast::Sender<SensorReading>, primaries: Primaries, caps: Capabilities }
pub struct Connected { info: DeviceInfo, listener: Option<JoinHandle<()>>, trainer: Option<TrainerBackend>, caps: MetricSet }
pub enum LinkState { Alive, Stale /* ANT: no page > DEVICE_DISCONNECT_TIMEOUT_SECS, channel still searching */,
                     Lost /* BLE disconnected, or ANT channel closed */ }
```

`TrainerBackend` gains `#[cfg(test)] Fake(Arc<FakeTrainer>)`. `Fec { usb, channel, queue, send_lock: Arc<std::sync::Mutex<()>>, last_cw_raw: Option<u8> }`.

## Decisions

### ADR-1: Lock scope — two-phase prepare/IO/commit with a per-device gate
- **Context.** The scan (BLE 3 s sleep + ANT 4 s), the connect (`find_peripheral` up to 4 s scan + connect; ANT scan-before-connect) and the watchdog reconnect all run while holding `Arc<tokio::Mutex<DeviceManager>>`. Zone control's `command_trainer` (Area B, including SAFETY_POWER) waits behind them.
- **Options.**
  - **A. Per-transport locks.** Split `DeviceManager` into `Arc` fields each with their own lock, and change `AppState` to hold the pieces. This ripples into `zone_control/controller.rs` (Area B) and every command.
  - **B. Two-phase prepare/IO/commit.** Keep `Arc<Mutex<DeviceManager>>`. I/O runs on cloned `Arc<Io>` handles outside the lock. Correctness under interleaving comes from a per-device gate and a re-check at commit.
  - **C. Narrowest fix.** Drop the lock only around the BLE `sleep` in `scan_all` and in `find_peripheral`. This leaves the ANT scan, the ANT connect and the reconnects holding the lock.
- **Decision: B.** The BLE and ANT transports are internally synchronised: `BleManager` already uses inner `tokio::Mutex` maps, and ANT goes behind `Arc<std::sync::Mutex<AntSlot>>` with blocking work in `spawn_blocking`. That is "per-transport locks" *inside* `RealIo`, invisible to callers.
- **Why.** No `AppState` type change. Area B's `connected_trainer_id()` / `set_target_power()` call sites are untouched. AC8 becomes testable with `FakeIo` sleeping 4 s under `tokio::time::pause`. C fails AC8 for ANT and reconnect.
- **Interleaving rules:**
  1. `connect_shared(id, origin)`:
     - lock → if `connected[id].status == Connected` → return `Ok(info)`, a no-op (AC6).
     - If `origin == Manual`, `reconnect.remove(id)`.
     - Clone the gate `Arc` → unlock → lock the gate → lock DM and re-check: still not connected; for `AutoReconnect`, the target still registered. Otherwise return `Ok(existing)`, or `Skipped` for auto.
     - Unlock → `io.connect` → lock → commit. Commit aborts any existing listener handle for `id` before inserting the new one (AC6 defence in depth), inserts the trainer backend, and records caps.
  2. `disconnect_shared`:
     - Takes the gate.
     - Under the lock: removes bookkeeping, aborts the listener, clears primaries for `id`, removes the reconnect target.
     - Then `io.disconnect` without the DM lock.
  3. `scan_shared`: no gate (it touches no device). `io.scan()` runs unlocked. The merge with `connected_devices` and the storage persist happen under the lock, which is DB-fast. ANT scan and ANT connect serialise on the ANT slot lock, not on the DM lock.
  4. `watchdog_tick`:
     - Lock → snapshot ids → unlock.
     - `io.probe` (BLE `is_connected` is async D-Bus).
     - Lock → apply transitions → unlock.
     - For each due reconnect → `connect_shared(.., AutoReconnect)`. Each attempt takes and releases the lock itself.
- **Not addressed (record for B).** Trainer commands themselves still hold the DM lock across their own I/O: FTMS indication wait ≤ 2 s; FE-C ack ≤ 2 × 600 ms after ADR-5. A slow trainer command blocks another trainer command. That is acceptable for now. A future `trainer_handle()` returning a cloneable backend would remove it, but that needs Area B call-site changes.
- **Reversibility: easy.** Internal API. Call sites outside `device/` change by one line each.

### ADR-2: ANT+ channel lifecycle — persistent channel with infinite low-priority search; Connected only after first page
- **Context (C2).** Today:
  - The watchdog marks stale but never closes the old channel.
  - Reconnect opens a *second* channel to the same device, which returns "success" immediately with `last_seen = 0`, so the watchdog never re-flags it.
  - The 30 s high-priority search timeout (`0x44 = 12`) silently closes the channel.
  - If the device returns, both channels deliver, so readings are duplicated.
- **Facts.** ANT Message Protocol & Usage D00000652 Rev 5.1:
  - §9.5.2.5: 0x44 is the high-priority timeout in 2.5 s units; 0 = disable HP search; 255 = infinite.
  - §9.5.2.15: 0x63 is the low-priority timeout in 2.5 s units, default 2; 255 = infinite. "A low priority search will not interrupt other open channels".
  - §9.5.6.1: after missed messages the channel drops to search (`EVENT_RX_FAIL_GO_TO_SEARCH` 0x08). When the search times out it is automatically closed (`EVENT_RX_SEARCH_TIMEOUT` 0x01). An explicit 0x07 may or may not follow.
  - §5.2.1.4.2: use low priority when other channels are open.
  - §9.4: the LP timeout is supported on nRF24AP2-USB / ANTUSB-m, i.e. both supported sticks.
- **Options.**
  - **A. Close and reopen.** On stale: close the channel, stop the listener, unregister, then reconnect via `ReconnectManager` with a new channel; reopen on `EVENT_CHANNEL_CLOSED`.
  - **B. Persistent channel.** Configure LP = 0xFF and HP = 0 on *connection* channels. The radio re-acquires indefinitely in the background; the listener and channel stay; the watchdog maps last-seen to Connected/Reconnecting.
  - **C.** B, but with HP infinite. Rejected: HP search interrupts other channels (§9.5.2.15), so it would drop HR/power pages while the trainer is missing.
- **Decision: B, with A as the fallback path.**
  - On `LinkState::Lost`, the channel actually closed: an `EVENT_RX_SEARCH_TIMEOUT`/`EVENT_CHANNEL_CLOSED` channel event for a connection channel we did not close ourselves, or the router exited. In that case the manager runs the full teardown *before* registering for reconnect: `io.disconnect` → listener stop flag, sender removed, `close_channel` tolerant of an already-closed channel, `last_seen` entry removed. Reconnect goes through `ReconnectManager` as for BLE.
  - The ordering is asserted with `FakeIo`'s event log.
  - **Connected only after first page.** `AntManager::connect` opens the channel, then blocks (inside `spawn_blocking`, under the ANT slot lock, not the DM lock) for up to `ANT_FIRST_PAGE_TIMEOUT = 5 s` for the first page on the channel's mpsc receiver. The page is forwarded to the listener, so it is not lost.
    - Implementation: the listener is spawned with a `first_page: oneshot`-like signal; simplest is `wait_first_page(&rx, timeout) -> Option<Vec<u8>>` *before* spawning the listener, then the page is handed to `listen_ant_channel` as `initial: Option<[u8;8]>`.
    - On timeout the channel is closed and `Err(AntError::Channel("no data from device"))` is returned. `last_seen` is set from the first page, so the watchdog can flag it later.
  - A device that never returns stays `Reconnecting`. It is never shown as Connected and never silently becomes "Connected with no data".
  - `device_reconnecting.attempt` counts watchdog ticks while stale.
  - Primary claims for a stale device are released. A backup device can then take over via lazy claim, and the returning device re-claims only if the slot is free. This is the same semantics as today's `remove_primary`/`auto_set_primary`.
  - The **scan** wildcard channels keep a finite search: LP default, HP 0x44 = 2 (5 s). Their window is 4 s anyway.
- **Why.** No duplicate channels by construction. No reconnect churn on a USB stick with 8 channels. Re-acquisition latency is that of LP search. It fixes "dead after 30 s".
- **Risk.** Acquisition speed with HP disabled is unproven on hardware. Mitigation, kept as a constant to tune: initial connect uses `LP = 0xFF, HP = 2` (5 s HP to acquire fast). After the first page the channel is reconfigured with `0x44 = 0`. Rev 5.1 §9.5.2.5 allows setting search timeout on an open channel; it takes effect on the next search. If a spike shows that is not honoured on an open channel, keep HP = 0 from the start.
- **Reversibility: easy.** These are constants and an internal state machine.

### ADR-3: Primaries keyed by metric, lazy claim, legacy command shape preserved
- **Context (C3).** Two `CadenceSpeed` pods: the first claims `CadenceSpeed`; every Cadence *and* Speed reading from the second is dominated.
- **Options.**
  - **A. Key by metric with lazy claim** (as in Data and contracts).
  - **B. Key by `(DeviceType, sub-type)`.** Split `CadenceSpeed` into `Cadence` and `Speed` device types. That changes the `known_devices` strings, the TS union and the dedup type-matching; one-way for stored rows.
  - **C. Keep `DeviceType` keys** but let each CadenceSpeed device claim based on the CSC Feature char (0x2A5C) or the ANT device type. This still breaks for a combo sensor plus a separate pod.
- **Decision: A.**
- **Why.** The de-duplication question is inherently per metric: trainer power vs power meter, trainer HR vs strap, power-meter cadence vs pod. Lazy claim needs no extra GATT reads, because a device claims only what it actually emits. CSC Feature (CSCS v1.0 §3.2, mandatory) is not needed.
- **Persistence.** Nothing is persisted today (verified: no `primary` in migrations or storage; `commands.rs` writes only the in-memory map). Keep in-memory. Persisting primaries is out of scope; it is noted for Area E if wanted.
- **Behaviour change (intended).** Primary is set on the device's first reading rather than at connect. Between connect and the first reading, `get_primary_devices` may not list it. The Devices page reloads primaries on mount, which is acceptable.
- **Reversibility: easy.** In-memory only; the command shape is a superset of today's.

### ADR-4: ANT+ scan — dynamic scan channels and exclusion lists for multiple devices per profile (C12, C17)
- **Context.**
  - Scan channels are reserved as `0..ALL_SCAN_PROFILES.len()` permanently. Adding combo S&C (6 profiles) would leave 2 connection channels out of 8.
  - Each wildcard channel locks onto the first master and never sees a second one (C17).
- **Facts.** D00000652 Rev 5.1:
  - §9.5.2.10 Add Channel ID to List (0x59): `[ch, dev_lo, dev_hi, dev_type, trans_type, index 0..3]`.
  - §9.5.2.12 Config ID List (0x5A): `[ch, size 0..4, exclude=1]`; max 4 per channel; slave only.
  - §5.7 / §9.5.4.5 continuous scan mode (0x5B) requires all other channels closed. That makes it unusable while sensors are connected.
- **Options.**
  - **A. Close and re-open with an exclusion list.** On a Channel ID response, close that profile's wildcard channel, add the found IDs (up to 4) to its exclusion list, and re-open it for the rest of the 4 s window.
  - **B. Continuous scan.** Only works with nothing connected.
  - **C. Background-scan channel type.** One per device; still one master at a time.
- **Decision: A.** Scan channels are allocated dynamically from channels not used by connections. Scan and connect are serialised by the ANT slot lock, so connections can use all 8 channels.
  - If free channels < profiles, profiles are scanned in batches of `free` channels, 4 s each. That is rare: it needs ≥ 3 ANT connections.
  - Up to 5 devices per profile per scan: 1 locked plus 4 excluded, after which the channel stays closed.
  - The startup "cleanup" closes only non-connection channels. The `response_queue.clear()` becomes a per-channel drain, so it can no longer eat a concurrent FE-C TX event.
- **Reversibility: easy.**

### ADR-5: FE-C acknowledged sends — bounded wait, one retry, serialised per channel (C6)
- **Facts.** D00000652 Rev 5.1 §9.5.5.2: a slave ack is sent right after the next master message.
  - The outcome is `EVENT_TRANSFER_TX_COMPLETED` (0x05) or `EVENT_TRANSFER_TX_FAILED` (0x06). There is no automatic resend. If the channel drops to search, `EVENT_RX_FAIL_GO_TO_SEARCH` (0x08) arrives instead.
  - A second ack while one is in flight gets response `TRANSFER_IN_PROGRESS` (0x1F).
  - FE-C channel period 8192 → 4 Hz → ≤ 250 ms per attempt nominally.
- **Options.**
  - A. Fire and forget (today).
  - B. Wait on the shared `response_queue` for the channel-scoped event, with a timeout and one retry.
  - C. A dedicated per-channel event mpsc from the router.
- **Decision: B.** Implementation:
  - `send_acknowledged_and_wait(tx: &dyn Fn(&AntMessage)->Result<()>, queue, ch, page, per_attempt = 600 ms) -> Result<(), AppError>`.
  - Before sending, it purges stale TX events for `ch`.
  - It succeeds on 0x05. It fails the attempt on 0x06, 0x08, response 0x1F, or the timeout. It retries once, then returns `AntError::Channel("FE-C ack failed: …")`.
  - It runs inside the existing `spawn_blocking`.
  - A per-backend `send_lock` serialises sends so `TRANSFER_IN_PROGRESS` cannot arise from our own concurrency.
  - The worst case is 1.2 s, which satisfies "≤ 1 s wait" per attempt.
- **Why not C.** It changes router fan-out for a small gain. B's queue is already channel-filtered everywhere after ADR-4.
- **Reversibility: easy.**

### ADR-6: CPS cadence and field walking (C11); CSC and CPS share a crank-delta helper (C9)
- **Facts.**
  - CPS v1.1 §3.2.1.1 / GSS §3.75 field order after flags (uint16): Instantaneous Power sint16 (2); then, each present only if its flag is set:
    - bit 0 → Pedal Power Balance uint8 (1)
    - bit 2 → Accumulated Torque uint16 (2)
    - bit 4 → Wheel Rev Data uint32 + uint16 at 1/2048 s (6)
    - bit 5 → Crank Rev Data uint16 + uint16 at 1/1024 s (4)
    - and so on.
  - Crank offset = `4 + (b0?1) + (b2?2) + (b4?6)`.
  - Crank event time rolls over every 64 s (§3.2.1.6).
  - ANT+ Bicycle Power D00001086 Rev 5.0 Table 8-1: page 0x10 byte 3 is instantaneous cadence, 0–254 rpm, 0xFF = invalid.
- **Decision.**
  - `struct CrankState { prev: Option<(u16 revs, u16 time)> }` with `fn update(revs, time) -> Option<f32 rpm>`. The first sample initialises and emits nothing. `time_diff == 0` means no new event, so nothing is emitted. `wrapping_sub` handles rollover. Out-of-range (≥ 200 rpm) emits nothing.
  - `CscState { wheel: Option<(u32,u16)>, crank: CrankState }` (C9). `CpsState { crank: CrankState }`.
  - `decode_cycling_power(&mut CpsState, data, id) -> Vec<SensorReading>` returns Power plus an optional Cadence.
  - Truncated payloads (flag set, bytes missing) skip only that field.
  - ANT page 0x10 cadence ≠ 0xFF → Cadence reading on new event-count samples. It follows the same event-count gating as power. On the first sample both are emitted.
- **Reversibility: easy.**

### ADR-7: FTMS resistance range (C13)
- **Facts.**
  - FTMS v1.0 §4.13 + Table 4.1 C.3: Supported Resistance Level Range (0x2AD6) is mandatory if Resistance Target Setting is supported.
  - The format is **inconsistent across official sources**. The 2017 SIG XML gives `sint16 min, sint16 max, uint16 inc` at 0.1 (6 octets). GSS (2023–2026) §3.231 gives `uint8 ×3` (3 octets) with self-contradictory scaling.
  - The op 0x04 parameter is **SINT16 at 0.1** per Errata Service Release 11, E8991 (the original FTMS 1.0 Table 4.15 said UINT8). The current 3-byte encoding is therefore spec-correct and stays.
  - GoldenCheetah (`BT40Device.cpp`) uses the 6-octet parse and the 3-byte write.
- **Decision.**
  - Read 0x2AD6 once in `TrainerController::new` (now async, run during `io.connect`, outside the DM lock). Parse by length:
    - 6 octets → the 2017 layout, raw in 0.1 units.
    - 3 octets → uint8 levels, treated as whole units, raw = v × 10.
    - Anything else, or a read error → `None`.
  - `encode_resistance(pct, range: Option<Range>)` maps `pct ∈ [0,100]` → `raw = min + round((max-min) × pct/100 / inc) × inc`, clamped to `[min, max]`.
  - `None` → today's `pct × 10`.
  - The 3-octet interpretation is unproven, so hardware validation is listed in Risks.
- **Reversibility: easy.**

### ADR-8: ANT+ background pages (C14) and combo S&C (C12)
- **Facts.**
  - HRM D00000693 Rev 1.13 §5.3: byte 0 bit 7 is the page toggle; bits 0–6 are the page number. Bytes 1–3 must not be interpreted until the toggle has been seen as both 0 and 1.
    - Page 2: byte 1 = manufacturer ID; bytes 2–3 = serial upper 16 bits (the lower 16 = device number).
    - Page 3: byte 1 = HW version; byte 2 = SW version; byte 3 = model.
  - BSC pages 2/3 have the same layout per ANT's official `ant-nrfconnect/sdk-ant` `ant_bsc` encoders (the BSC profile PDF was not obtainable).
  - The combo sensor (type 121, period 8086, RF 57) has no page byte: `[cad_time u16, cad_revs u16, spd_time u16, spd_revs u16]` LE.
- **Decision.**
  - `PageToggleGate` (has-seen-0, has-seen-1) per listener. Once open, pages `data[0] & 0x7F ∈ {2, 3}` are decoded into `AntDeviceMetadata`:
    - `manufacturer_id = data[1] as u16`.
    - `serial_number = (u16(data[2..4]) << 16) | device_number`.
    - `hw_revision`, `sw_revision`, `model_number = data[3] as u16`.
  - Page 7 (HR battery) is decoded only if a spec copy confirms it (openant-only today). It is **not** in scope for AC14.
  - The device number is passed to the listener; it is parsed from `ant:<type>:<n>`.
  - The combo uses a new `PROFILE_SPEED_CADENCE` and `AntDecoder::decode_combined_sc` with independent cadence and speed state.
- **Reversibility: easy.**

### ADR-9: Grade and simulation encoding (C1, C10)
- **Facts.** FE-C D00001231 Rev 5.0:
  - Table 8-42, page 0x33: `[0x33, FF, FF, FF, FF, grade_lo, grade_hi, crr]`. Grade is 0.01 % with offset −200 % (0x4E20 = 0 %). Crr is in 5×10⁻⁵ units, 0xFF = default 0.004.
  - Table 8-38, page 0x32: `[0x32, FF, FF, FF, FF, cw, wind, draft]`. Cw is 0.01 kg/m, range 0–1.86, 0xFF = default 0.51. Wind = km/h + 127, 0xFF = default. Draft is 0.01, 0xFF = default 1.0.
- **Decision.**
  - FE-C:
    - `grade_raw = round((clamp(g, -200, 200) + 200) × 100) as u16`, at most 40000, so it cannot overflow.
    - NaN grade → 0 %.
    - `crr_raw = round(clamp(crr, 0, 0.0127) / 5e-5)`.
  - `set_simulation` sends 0x32 `[0x32, FF×4, cw_raw, 0xFF, 0xFF]` with `cw` clamped 0–1.86 and rounded, *only when `cw_raw` differs from the last successfully sent value*, then sends 0x33. Wind and draft are left at their defaults (no input for them).
  - FTMS 0x11 (FTMS v1.0 Table 4.20): `grade_raw = round(clamp(g, -100, 100) × 100) as i16`; NaN → 0; Crr and Cw rounded.
- **Reversibility: easy.**

### ADR-10: Small decode fixes
- **C5.** FE-C page 0x19: 12-bit power `== 0xFFF` → no Power reading (FE-C Rev 5.0 Table 8-25: "0xFFF = invalid").
- **C15.** CPS: balance-reference bit 1 = 0 (Unknown) → `pedal_balance = None`.
  - CPS v1.1 §3.2.1.3 does not define which side an unknown-reference value describes.
  - Reference = Left → `100 − left%`, as today.
  - Also fix the integer `raw / 2` truncation to `round(raw / 2)`.
- **C16.** `name_number_match`:
  - Split the BLE name on non-alphanumeric chars.
  - Match if any all-digit token parses to the same `u32` as the ANT device number (so leading zeros are tolerated).
  - A token like `4A3B` never matches `4`.
- **C8.** `ble.rs`: wrap everything after `peripheral.connect()` in `async fn finish_connect(&Peripheral, ...) -> Result<DeviceInfo>`. On `Err`, `peripheral.disconnect().await` (failure logged, not swallowed) and the error is returned. The policy lives in a generic `async fn disconnect_on_err<T, F, D>(work: F, cleanup: D)` so it is unit-testable with closures.

## Failure modes

| Failure | Detection | Behavior | Recovery |
|---|---|---|---|
| ANT device out of range | `last_seen` > 10 s | status Reconnecting, `device_disconnected` once, then `device_reconnecting` each tick; primary claims released | Pages resume → Connected, `device_reconnected`; LP search continues indefinitely |
| ANT channel closed unexpectedly | Channel event 0x01/0x07 for a connection channel not being closed by us; or listener rx disconnected | `LinkState::Lost` → full teardown via `io.disconnect` → `ReconnectManager` | Reconnect opens a fresh channel; Connected only after first page |
| ANT stick unplugged / router exits | Router loop exit sets all connection `closed` flags | All ANT devices Lost → reconnect attempts fail (`NoUsbStick`) | Re-plug → `ensure_ant` re-init on next attempt |
| ANT first page never arrives on connect | 5 s timeout | Channel closed, connect returns error | User retries / reconnect backoff |
| FE-C ack fails | TX_FAILED / GO_TO_SEARCH / 600 ms timeout | Retry once; then `Err` to caller (zone control logs/handles) | Next command |
| Blocking ANT task panics | `spawn_blocking` JoinError / poisoned slot | Slot reset to `None`, AntManager dropped (router stopped) | `ensure_ant` re-inits on next use (as today) |
| BLE discovery/classify fails after connect | `Err` from `finish_connect` | Peripheral disconnected, error returned | User retry |
| Concurrent connect of same device | Per-device gate | Second caller waits, then sees Connected → no-op Ok | — |
| Connect commit finds device was disconnected meanwhile | Not possible: disconnect also takes the gate | — | — |
| 0x2AD6 missing / malformed | Read error or length ∉ {3, 6} | Range `None` → legacy 0–100 % mapping | — |
| Scan while ≥ 3 ANT devices connected | free channels < profiles | Profiles scanned in batches (scan takes longer) | — |

## Migration and rollback

- No schema change. `known_devices.device_type` strings are unchanged. Combo S&C devices are stored as `CadenceSpeed` with id `ant:121:<n>`.
- Primaries are in-memory. Upgrading resets nothing that was persisted, because nothing is.
- Rollback = revert the branch. There is no data migration to undo.
- **One-way decisions: none.**

## Traceability

| AC | Component(s) | Test(s) (planned names) |
|---|---|---|
| AC1 (C1) | `fec.rs` | `fec_track_resistance_zero_grade_layout`, `fec_track_resistance_minus_3_percent`, `fec_track_resistance_plus_5_percent`, `fec_track_resistance_crr_0_004_is_raw_80`, `fec_wind_resistance_page_layout_cw_0_51`, `fec_wind_page_sent_only_when_cw_changes` |
| AC2 (C10) | `fec.rs`, `ftms.rs` | `fec_grade_2_3_rounds_to_raw_20230`, `fec_grade_extremes_do_not_overflow`, `fec_grade_nan_encodes_zero_percent`, `ftms_grade_2_3_rounds_to_230`, `ftms_grade_clamped_to_plus_minus_100` |
| AC3 (C5) | `ant/protocol.rs` | `fec_page_0x19_power_0xfff_yields_no_power_reading`, `fec_page_0x19_power_4094_is_valid` (replaces `..._12bit_mask`, which asserted 4095 W) |
| AC4 (C3) | `types.rs`, `manager.rs`, `commands.rs` | `admit_speed_only_and_cadence_only_pods_both_forwarded`, `admit_second_cadence_device_dominated`, `admit_power_meter_cadence_not_claimable_but_forwarded`, `admit_trainer_power_dominated_by_power_meter_primary`, `primary_key_legacy_cadence_speed_sets_device_capabilities`, `get_primaries_includes_legacy_and_metric_keys` |
| AC5 (C2) | `ant/channel.rs`, `ant/manager.rs`, `manager.rs` | `ant_connection_channel_search_config_bytes` (0x44 = 0, 0x63 = 0xFF), `wait_first_page_times_out_without_data`, `wait_first_page_returns_first_page`, `watchdog_stale_ant_marked_reconnecting_not_connected`, `watchdog_stale_ant_recovers_without_new_channel`, `watchdog_lost_ant_tears_down_before_reconnect` (event-log order), `ant_reconnect_yields_single_listener` |
| AC6 (C4) | `manager.rs` | `connect_twice_spawns_one_listener`, `concurrent_connects_same_device_one_io_connect`, `manual_connect_clears_pending_reconnect`, `commit_aborts_existing_listener_handle` |
| AC7 (C6) | `ant/channel.rs`, `fec.rs` | `ack_wait_succeeds_on_tx_completed`, `ack_wait_retries_once_on_tx_failed_then_succeeds`, `ack_wait_errors_after_two_failures`, `ack_wait_times_out_within_bound`, `ack_wait_ignores_other_channel_events`, `ack_wait_treats_go_to_search_as_failure` |
| AC8 (C7) | `manager.rs` (FakeIo, paused time) | `trainer_command_during_scan_completes_within_500ms`, `trainer_command_during_reconnect_completes_within_500ms` |
| AC9 (C8) | `ble.rs` | `disconnect_on_err_runs_cleanup_on_failure`, `disconnect_on_err_skips_cleanup_on_success`, `disconnect_on_err_returns_original_error_when_cleanup_fails` |
| AC10 (C9) | `protocol.rs` | `csc_first_notification_emits_nothing`, `csc_second_notification_computes_cadence_60rpm`, `csc_new_state_after_reconnect_reinitialises` |
| AC11 (C11) | `protocol.rs`, `ant/protocol.rs` | `cps_crank_only_cadence_90rpm`, `cps_crank_offset_after_balance_torque_wheel`, `cps_crank_event_time_rollover`, `cps_crank_first_sample_initialises`, `cps_truncated_crank_field_still_yields_power`, `ant_power_page_cadence_byte3`, `ant_power_page_cadence_0xff_omitted` |
| AC12 (C12) | `ant/protocol.rs`, `ant/channel.rs` | `combined_sc_first_page_initialises`, `combined_sc_speed_and_cadence_from_one_page`, `combined_sc_counter_rollover`, `profile_speed_cadence_is_type_121_period_8086`, `scan_profiles_include_combined_sc` |
| AC13 (C13) | `ftms.rs` | `resistance_range_parse_6_octet_layout`, `resistance_range_parse_3_octet_layout`, `resistance_range_parse_bad_length_is_none`, `resistance_50pct_scaled_into_range_with_increment`, `resistance_without_range_uses_legacy_mapping` |
| AC14 (C14) | `ant/listener.rs` | `hr_page2_ignored_before_toggle_seen`, `hr_page2_manufacturer_and_serial_with_toggle_bit_set`, `hr_page3_hw_sw_model`, `sc_page2_serial_combines_device_number` |
| AC15 (C15) | `protocol.rs` | `cps_balance_unknown_reference_is_none` (replaces `decode_power_pedal_balance_right_ref`), `cps_balance_left_reference_inverted_rounded` |
| AC16 (C16) | `dedup.rs` | `name_number_requires_whole_token`, `name_number_matches_token_after_separator`, `name_number_leading_zero_token_matches` |
| AC17 (C17) | `ant/channel.rs`, `ant/manager.rs` | `exclusion_list_messages_bytes` (0x59 / 0x5A layout), `scan_channel_allocation_skips_connection_channels`, `scan_profiles_batched_when_channels_scarce` |
| AC18 | all | `cargo test` full suite green; every test above fails on the pre-change code (verified per slice) |

## Risks and spikes

1. **Riskiest: the ADR-1 refactor.** It introduces generics, re-entrancy and gate-ordering bugs into code that has no hardware tests. Mitigation:
   - Slice 3 lands the seam and two-phase flow first, with `FakeIo` tests for AC6 and AC8, *before* the ANT changes.
   - Lock order is always DM → (released) → gate → DM. The DM lock is never held while awaiting a gate.
   - A debug assertion helper documents this. Spike: write `trainer_command_during_scan_completes_within_500ms` first against current code; it must fail.
2. **ANT LP-only search acquisition latency (ADR-2).** This is unprovable in CI. Constants (`ANT_CONN_HP_TIMEOUT`, `ANT_CONN_LP_TIMEOUT`, `ANT_FIRST_PAGE_TIMEOUT`) are in `config.rs`-style consts in `ant/channel.rs`. Hardware check by the user after merge.
3. **0x2AD6 3-octet scaling ambiguity (ADR-7).** Hardware check; fallback is legacy behaviour.
4. **HR page 7 and BSC background page layouts** come only from implementation sources. Page 7 is excluded; BSC pages 2/3 are cited to ANT's own SDK.
5. **Exclusion-list behaviour on real sticks (ADR-4).** It is spec-supported (§9.4 table); hardware check.

## Implementation plan

Each slice is one commit with its tests (failing first).

1. **FE-C/FTMS grade and track resistance (C1, C10).** Page 0x33 layout, page 0x32 with cw cache (`TrainerBackend::Fec.last_cw_raw`), rounding, NaN, clamps. Update the existing `encode_simulation_*` tests to the spec layout.
2. **FE-C 0xFFF invalid power (C5).** Replace the test that asserted 4095 W.
3. **Spike + seam: `DeviceIo`, `RealIo`, `FakeIo`, two-phase `scan_shared`/`connect_shared`/`disconnect_shared`/`watchdog_tick`, per-device gates, idempotent connect (C7, C4).**
   - Move the BLE/ANT ownership from `DeviceManager` into `RealIo`. The ANT slot becomes `Arc<StdMutex<Option<AntManager>>>` with poison → reset.
   - Update `lib.rs` and `commands.rs` call sites (one line each).
   - Tests: AC6 and AC8.
   - Behaviour is otherwise unchanged; the old ANT reconnect logic is retained for now.
4. **ANT persistent channel lifecycle (C2).**
   - Search timeout messages: add `MSG_SET_LP_SEARCH_TIMEOUT = 0x63` and the event constants.
   - `wait_first_page`; listener `initial` page.
   - `closed` flag via router channel events / listener exit; `LinkState` probe.
   - Watchdog Stale/Recovered/Lost transitions; teardown-before-reconnect.
   - `AntManager::connect` idempotence guard.
5. **Metric primaries (C3).** `Metric`, `admit_reading`, `Capabilities`, lazy claim in the BLE and ANT listeners, `PrimaryKey` commands, release on stale/disconnect. Remove `is_dominated` and migrate its tests to `admit_reading`.
6. **FE-C ack wait (C6).** `send_acknowledged_and_wait`, `send_lock`, queue in the backend, per-channel drains.
7. **BLE connect cleanup (C8).** `disconnect_on_err` + `finish_connect`.
8. **Crank/CSC state (C9, C11).** `CrankState`, `CscState`, `CpsState`, CPS field walking, ANT page 0x10 cadence.
9. **ANT scan rework (C12, C17).** Dynamic scan channels, batching, exclusion lists, combo S&C profile and decoder.
10. **Background pages (C14).** `PageToggleGate`, pages 2/3 for HR and legacy S&C; device number passed to the listener.
11. **FTMS resistance range (C13).** Async `TrainerController::new` reads 0x2AD6; range-aware `encode_resistance`.
12. **Small fixes (C15, C16).** Balance unknown → None plus rounding; whole-token dedup.

## Cross-area dependencies

- **B (zone control).** It may add a power clamp near `DeviceManager::set_target_power`. That signature is kept (`&mut self, &str, i16`), so a clamp inside or before it merges cleanly. Area B must not take the DM lock around scans; it does not today. A trainer command can still wait ≤ ~1.2 s (FE-C) / 2 s (FTMS) behind *another* trainer command (ADR-1 note).
- **D (frontend).** Optional: use the `Cadence`/`Speed` keys from `get_primary_devices` for `CadenceSpeed` devices, and `setPrimaryDevice('Cadence' | 'Speed', id)`. No breaking change. The ANT stale device now appears with `status: 'Reconnecting'` in `device_list_updated` (already in the TS union).
- **A (session metrics).** No contract change. Readings may now include power-meter cadence and both pods' speed and cadence; `SensorReading` variants are unchanged.
- **E (storage/infra).** Nothing required. If primaries should survive restarts, that is a new storage feature (not requested).
- **Files outside `device/` touched:** `src-tauri/src/lib.rs` (watchdog body), `src-tauri/src/commands.rs` (`scan_devices`, `connect_device`, `disconnect_device`, `set_primary_device`, `get_primary_devices`, `AppState.primary_devices` type). Merge-conflict risk with B and E is in `commands.rs`.

## References

- Bluetooth SIG: Cycling Power Service v1.1 §3.2.1; Cycling Speed and Cadence Service v1.0 §3.1–3.2; Fitness Machine Service v1.0 §4.13, §4.16, Tables 4.1/4.15/4.20; Errata Service Release 11 (E8991, E9135); GATT Specification Supplement (2026-09-09) §3.75, §3.70–3.71, §3.231.
- ANT: ANT Message Protocol and Usage D00000652 Rev 5.1 §5.2.1.4.2, §5.3, §5.7, §9.4, §9.5.2.5, §9.5.2.10, §9.5.2.12, §9.5.2.15, §9.5.5.2, §9.5.6.1; ANT+ Fitness Equipment D00001231 Rev 5.0 Tables 8-25, 8-36, 8-37, 8-38, 8-42; ANT+ Bicycle Power D00001086 Rev 5.0 Tables 8-1, 10-1; ANT+ Heart Rate Monitor D00000693 Rev 1.13 §5.3, Tables 6–7; ANT `ant-nrfconnect/sdk-ant` `ant_bsc` (BSC pages, combined-sensor layout). ANT+ profile PDFs are adopter-only; they are cited here, not vendored.
