import { writable, get } from 'svelte/store';
import { listen } from '@tauri-apps/api/event';
import type { SensorReading, LiveMetrics } from '$lib/tauri';
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

let unlistenSensor: (() => void) | null = null;
let unlistenMetrics: (() => void) | null = null;
let generation = 0;

export async function startSensorListening() {
  if (unlistenSensor) return;

  const myGen = ++generation;

  const [sensorUn, metricsUn] = await Promise.all([
    listen<SensorReading>('sensor_reading', (event) => {
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
    }),
    listen<LiveMetrics>('live_metrics', (event) => {
      liveMetrics.set(event.payload);
    }),
  ]);

  // If stop was called (or another start began) while we were awaiting,
  // our generation is stale — clean up immediately to avoid leaking.
  if (myGen !== generation) {
    sensorUn();
    metricsUn();
    return;
  }

  unlistenSensor = sensorUn;
  unlistenMetrics = metricsUn;
}

export function stopSensorListening() {
  generation++; // Invalidate any in-flight startSensorListening
  if (unlistenSensor) {
    unlistenSensor();
    unlistenSensor = null;
  }
  if (unlistenMetrics) {
    unlistenMetrics();
    unlistenMetrics = null;
  }
  liveMetrics.set(null);
  metricHistory.set([]);
  latestPower = null;
  latestHR = null;
  latestCadence = null;
  latestSpeed = null;
}
