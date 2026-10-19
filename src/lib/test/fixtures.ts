import type { SessionSummary, ZoneControlStatus } from '$lib/tauri';

/** Active power zone ride status (zone 3, 150–180 W, 600 s). */
export function activeZoneStatus(overrides: Partial<ZoneControlStatus> = {}): ZoneControlStatus {
  return {
    active: true,
    mode: 'Power',
    target_zone: 3,
    lower_bound: 150,
    upper_bound: 180,
    commanded_power: 165,
    time_in_zone_secs: 100,
    elapsed_secs: 120,
    duration_secs: 600,
    paused: false,
    phase: 'in_zone',
    safety_note: null,
    ...overrides,
  };
}

/** Status the backend reports right after the controller stopped itself. */
export function selfStoppedZoneStatus(overrides: Partial<ZoneControlStatus> = {}): ZoneControlStatus {
  return activeZoneStatus({ active: false, commanded_power: null, ...overrides });
}

export function summary(id: string, overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    id,
    start_time: '2025-06-15T14:00:00Z',
    duration_secs: 3600,
    ftp: 200,
    avg_power: null,
    max_power: null,
    normalized_power: null,
    tss: null,
    intensity_factor: null,
    avg_hr: null,
    max_hr: null,
    avg_cadence: null,
    avg_speed: null,
    ...overrides,
  };
}

/** A promise whose settlement the test controls. */
export function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}
