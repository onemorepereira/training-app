import { writable, derived, get } from 'svelte/store';
import { api, extractError } from '$lib/tauri';
import type {
  ZoneControlStatus,
  ZoneTarget,
  ZoneMode,
  SessionConfig,
  StopReason,
  ZoneRideConfig,
} from '$lib/tauri';

export const zoneStatus = writable<ZoneControlStatus | null>(null);
export const zoneActive = derived(zoneStatus, ($s) => $s?.active ?? false);

/** Outcome of the most recent zone ride in the current session. */
export interface ZoneRideResult {
  config: ZoneRideConfig;
  reason: StopReason | null;
  safetyNote: string | null;
  /** Error from stopping zone control or saving the result, shown to the user. */
  error: string | null;
}

/**
 * Last finished zone ride, held until it is persisted when the session ends
 * (zone_config can only be written once the session row exists).
 * Lifecycle: replaced by the next finished zone ride, cleared after a successful
 * save or at the next session start.
 */
export const zoneRideResult = writable<ZoneRideResult | null>(null);

let pollInterval: ReturnType<typeof setInterval> | null = null;

export function startZonePolling() {
  if (pollInterval) return;
  pollInterval = setInterval(pollOnce, 1000);
}

async function pollOnce() {
  let status: ZoneControlStatus;
  try {
    status = await api.getZoneControlStatus();
  } catch (e) {
    console.warn('get_zone_control_status failed:', extractError(e));
    return;
  }
  // Polling may have been stopped while the request was in flight.
  if (!pollInterval) return;
  const prev = get(zoneStatus);
  zoneStatus.set(status);
  if (prev?.active && !status.active) {
    // Backend controller stopped on its own (duration complete, safety stop, sensor lost).
    await finishZoneRide(completionSnapshot(prev, status));
  }
}

export function stopZonePolling() {
  if (pollInterval) {
    clearInterval(pollInterval);
    pollInterval = null;
  }
  zoneStatus.set(null);
}

export async function startZoneRide(target: ZoneTarget): Promise<void> {
  await api.startZoneControl(target);
  startZonePolling();
}

/** User-initiated zone ride stop: finishes the zone ride and keeps its result. */
export async function stopZoneRide(): Promise<void> {
  const snap = get(zoneStatus);
  if (snap?.active) {
    await finishZoneRide(snap);
  } else {
    await api.stopZoneControl();
    stopZonePolling();
  }
}

/**
 * Build the snapshot describing a zone ride that ended on its own.
 * The target (mode, zone, bounds, duration) and commanded power come from the
 * last active status `prev` — once inactive the backend reports
 * commanded_power = null and may drop the target. Progress fields come from `next`.
 */
export function completionSnapshot(
  prev: ZoneControlStatus,
  next: ZoneControlStatus,
): ZoneControlStatus {
  return {
    ...prev,
    active: false,
    time_in_zone_secs: Math.max(prev.time_in_zone_secs, next.time_in_zone_secs),
    elapsed_secs: Math.max(prev.elapsed_secs, next.elapsed_secs),
    paused: next.paused,
    phase: next.phase,
    safety_note: next.safety_note ?? prev.safety_note,
    stop_reason: next.stop_reason ?? null,
  };
}

/** Convert a zone status snapshot into the persisted zone ride config. */
export function buildZoneRideConfig(snap: ZoneControlStatus): ZoneRideConfig | null {
  if (snap.mode == null) return null;
  return {
    mode: snap.mode,
    zone: snap.target_zone ?? 0,
    lower_bound: snap.lower_bound ?? 0,
    upper_bound: snap.upper_bound ?? 0,
    duration_secs: snap.duration_secs ?? null,
    time_in_zone_secs: snap.time_in_zone_secs,
    commanded_power_series: snap.commanded_power != null ? [snap.commanded_power] : [],
    time_to_zone_secs: null,
  };
}

let finishInflight: Promise<void> | null = null;

/**
 * Finish a zone ride: stop backend zone control (collecting its stop reason),
 * stop polling and record the result. Single-flight: concurrent callers share one stop.
 */
export function finishZoneRide(snap: ZoneControlStatus): Promise<void> {
  if (finishInflight) return finishInflight;
  finishInflight = (async () => {
    let reason: StopReason | null = null;
    let error: string | null = null;
    try {
      const stopped = await api.stopZoneControl();
      // Prefer the reason the backend recorded when it stopped on its own.
      reason = snap.stop_reason ?? stopped;
    } catch (e) {
      reason = snap.stop_reason ?? null;
      error = `Zone control stop failed: ${extractError(e)}`;
    }
    stopZonePolling();
    const config = buildZoneRideConfig(snap);
    if (config) {
      zoneRideResult.set({ config, reason, safetyNote: snap.safety_note, error });
    }
  })().finally(() => {
    finishInflight = null;
  });
  return finishInflight;
}

/** Human-readable notice for a finished zone ride. */
export function describeZoneStop(result: Pick<ZoneRideResult, 'reason' | 'safetyNote'>): string {
  let text: string;
  switch (result.reason) {
    case 'DurationComplete': text = 'Zone ride complete'; break;
    case 'SafetyStop': text = 'Zone ride stopped: safety stop'; break;
    case 'SensorLost': text = 'Zone ride stopped: sensor lost'; break;
    case 'TrainerDisconnected': text = 'Zone ride stopped: trainer disconnected'; break;
    case 'TrainerUnresponsive': text = 'Trainer stopped responding — zone ride ended for safety'; break;
    case 'ControllerFault': text = 'Zone ride ended: controller fault'; break;
    case 'SessionStopped': text = 'Zone ride ended with the session'; break;
    case 'AppClosing': text = 'Zone ride ended: app closing'; break;
    default: text = 'Zone ride stopped';
  }
  if (result.safetyNote) text += ` (${result.safetyNote})`;
  return text;
}

export async function pauseZoneRide(): Promise<void> {
  await api.pauseZoneControl();
}

export async function resumeZoneRide(): Promise<void> {
  await api.resumeZoneControl();
}

/**
 * Resolve zone number to watts/bpm bounds from user config.
 * Power zones are stored as %FTP upper boundaries of zones 1–6; zone 7 is open-ended,
 * so its target upper bound is max(lower + 1, 2·FTP) to keep the range non-empty.
 * HR zones are absolute bpm upper boundaries of zones 1–5.
 * Zone k ≥ 2 spans [upper(k−1), upper(k)]. Out-of-range zone numbers are clamped.
 */
export function resolveZoneBounds(
  mode: ZoneMode,
  zone: number,
  config: SessionConfig,
): { lower: number; upper: number } {
  if (mode === 'Power') {
    const ftp = config.ftp;
    const b = config.power_zones.map((p) => Math.round((p / 100) * ftp));
    const z = Math.min(7, Math.max(1, zone));
    if (z === 1) return { lower: 0, upper: b[0] };
    if (z === 7) return { lower: b[5], upper: Math.max(b[5] + 1, Math.round(2 * ftp)) };
    return { lower: b[z - 2], upper: b[z - 1] };
  }
  const hz = config.hr_zones;
  const z = Math.min(5, Math.max(1, zone));
  if (z === 1) return { lower: 0, upper: hz[0] };
  return { lower: hz[z - 2], upper: hz[z - 1] };
}
