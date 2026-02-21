import { writable, derived } from 'svelte/store';
import { api } from '$lib/tauri';
import type { ZoneControlStatus, ZoneTarget, ZoneMode, SessionConfig } from '$lib/tauri';

export const zoneStatus = writable<ZoneControlStatus | null>(null);
export const zoneActive = derived(zoneStatus, ($s) => $s?.active ?? false);

let pollInterval: ReturnType<typeof setInterval> | null = null;

export function startZonePolling() {
  if (pollInterval) return;
  pollInterval = setInterval(async () => {
    try {
      const status = await api.getZoneControlStatus();
      zoneStatus.set(status);
    } catch {
      // Zone control may not be active
    }
  }, 1000);
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

export async function stopZoneRide(): Promise<void> {
  await api.stopZoneControl();
  stopZonePolling();
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
