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
      reason = await api.stopZoneControl();
    } catch (e) {
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
 * Power zones are stored as %FTP boundaries (zone 1 < power_zone_1% < zone 2 < ... < zone 7).
 * HR zones are absolute bpm boundaries (zone 1 < hr_zone_1 < zone 2 < ... < zone 5).
 */
export function resolveZoneBounds(
  mode: ZoneMode,
  zone: number,
  config: SessionConfig,
): { lower: number; upper: number } {
  if (mode === 'Power') {
    // power_zones = [z1_upper%, z2_upper%, z3_upper%, z4_upper%, z5_upper%, z6_upper%]
    // zone 1: 0 - pz[0]%, zone 2: pz[0] - pz[1]%, ... zone 7: pz[5]% +
    const pz = config.power_zones;
    const ftp = config.ftp;
    const boundaries = pz.map((p) => Math.round((p / 100) * ftp));
    if (zone <= 1) return { lower: 0, upper: boundaries[0] };
    if (zone >= 7) return { lower: boundaries[5], upper: Math.round(ftp * 1.5) };
    return { lower: boundaries[zone - 2], upper: boundaries[zone - 1] };
  } else {
    // hr_zones = [z1_upper, z2_upper, z3_upper, z4_upper, z5_upper]
    const hz = config.hr_zones;
    if (zone <= 1) return { lower: 0, upper: hz[0] };
    if (zone >= 5) return { lower: hz[4], upper: config.max_hr ?? 220 };
    return { lower: hz[zone - 2], upper: hz[zone - 1] };
  }
}
