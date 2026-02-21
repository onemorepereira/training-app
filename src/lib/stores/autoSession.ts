import { writable, get } from 'svelte/store';
import { currentCadence, currentSpeed, liveMetrics } from '$lib/stores/sensor';
import { sessionActive, sessionId, sessionPaused } from '$lib/stores/session';
import { api } from '$lib/tauri';

export const autoSessionEnabled = writable(false);
export const autoSessionCountdown = writable<number | null>(null);

const AUTO_START_THRESHOLD_SECS = 5;
const AUTO_STOP_THRESHOLD_SECS = 3;

let tickInterval: ReturnType<typeof setInterval> | null = null;
let cadenceAboveZeroTicks = 0;
let speedZeroTicks = 0;
let starting = false;
let stopping = false;

function tick() {
  const enabled = get(autoSessionEnabled);
  if (!enabled) {
    resetCounters();
    return;
  }

  const active = get(sessionActive);
  const cadence = get(currentCadence);
  const speed = get(currentSpeed);

  if (!active) {
    // Auto-start logic: cadence > 0 for 5 consecutive seconds
    if (cadence != null && cadence > 0) {
      cadenceAboveZeroTicks++;
      const remaining = AUTO_START_THRESHOLD_SECS - cadenceAboveZeroTicks;
      if (remaining > 0) {
        autoSessionCountdown.set(remaining);
      } else {
        autoSessionCountdown.set(null);
        cadenceAboveZeroTicks = 0;
        startSession();
      }
    } else {
      cadenceAboveZeroTicks = 0;
      autoSessionCountdown.set(null);
    }
    // Reset stop counter when no session
    speedZeroTicks = 0;
  } else {
    // Auto-stop logic: speed = 0 (or stale sensor) for 3 consecutive seconds
    const metrics = get(liveMetrics);
    const speedStale = metrics?.stale_speed ?? false;
    if (speed == null || speed === 0 || speedStale) {
      speedZeroTicks++;
      if (speedZeroTicks >= AUTO_STOP_THRESHOLD_SECS) {
        speedZeroTicks = 0;
        stopSession();
      }
    } else {
      speedZeroTicks = 0;
    }
    // Reset start counter when session is active
    cadenceAboveZeroTicks = 0;
    autoSessionCountdown.set(null);
  }
}

async function startSession() {
  if (starting) return;
  starting = true;
  try {
    const id = await api.startSession();
    sessionId.set(id);
    sessionActive.set(true);
    sessionPaused.set(false);
  } catch {
    // Ignore — user can still start manually
  } finally {
    starting = false;
  }
}

async function stopSession() {
  if (stopping) return;
  stopping = true;
  try {
    await api.stopSession();
    sessionActive.set(false);
    sessionId.set(null);
    sessionPaused.set(false);
  } catch {
    // Ignore — user can still stop manually
  } finally {
    stopping = false;
  }
}

function resetCounters() {
  cadenceAboveZeroTicks = 0;
  speedZeroTicks = 0;
  autoSessionCountdown.set(null);
}

export function initAutoSession() {
  if (tickInterval) return;
  tickInterval = setInterval(tick, 1000);
}

export function destroyAutoSession() {
  if (tickInterval) {
    clearInterval(tickInterval);
    tickInterval = null;
  }
  resetCounters();
  starting = false;
  stopping = false;
}
