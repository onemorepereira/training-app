import { writable, get } from 'svelte/store';
import { listen } from '@tauri-apps/api/event';
import type { SensorReading, LiveMetrics } from '$lib/tauri';
import { api } from '$lib/tauri';
import { sessionActive, sessionPaused } from '$lib/stores/session';

export const currentPower = writable<number | null>(null);
export const currentHR = writable<number | null>(null);
export const currentCadence = writable<number | null>(null);
export const currentSpeed = writable<number | null>(null);
export const liveMetrics = writable<LiveMetrics | null>(null);

export interface MetricHistoryEntry {
  t: number;
  power: number | null;
  hr: number | null;
  cadence: number | null;
  speed: number | null;
}

const MAX_HISTORY = 600;
export const metricHistory = writable<MetricHistoryEntry[]>([]);

let latestPower: number | null = null;
let latestHR: number | null = null;
let latestCadence: number | null = null;
let latestSpeed: number | null = null;

let metricsInterval: ReturnType<typeof setInterval> | null = null;
let unlistenFn: (() => void) | null = null;
let initializing = false;

export async function startSensorListening() {
  if (unlistenFn || initializing) return;
  initializing = true;

  try {
    unlistenFn = await listen<SensorReading>('sensor_reading', (event) => {
      const reading = event.payload;
      if (reading.Power) { latestPower = reading.Power.watts; currentPower.set(latestPower); }
      if (reading.HeartRate) { latestHR = reading.HeartRate.bpm; currentHR.set(latestHR); }
      if (reading.Cadence) { latestCadence = reading.Cadence.rpm; currentCadence.set(latestCadence); }
      if (reading.Speed) { latestSpeed = reading.Speed.kmh; currentSpeed.set(latestSpeed); }

      if (get(sessionActive) && !get(sessionPaused)) {
        metricHistory.update((h) => {
          const next = [...h, { t: Date.now(), power: latestPower, hr: latestHR, cadence: latestCadence, speed: latestSpeed }];
          return next.length > MAX_HISTORY ? next.slice(next.length - MAX_HISTORY) : next;
        });
      }
    });

    metricsInterval = setInterval(async () => {
      try {
        const metrics = await api.getLiveMetrics();
        liveMetrics.set(metrics);
      } catch {
        // Session may not be active
      }
    }, 250);
  } finally {
    initializing = false;
  }
}

export function stopSensorListening() {
  initializing = false;
  if (unlistenFn) {
    unlistenFn();
    unlistenFn = null;
  }
  if (metricsInterval) {
    clearInterval(metricsInterval);
    metricsInterval = null;
  }
  liveMetrics.set(null);
  metricHistory.set([]);
  latestPower = null;
  latestHR = null;
  latestCadence = null;
  latestSpeed = null;
}
