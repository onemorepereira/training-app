import { writable, get } from 'svelte/store';
import { currentCadence, getSensorSnapshot } from '$lib/stores/sensor';
import { sessionActive, sessionPaused } from '$lib/stores/session';
import { startSession, endSession, lifecycleError } from '$lib/stores/sessionLifecycle';
import { extractError } from '$lib/tauri';

export const autoSessionEnabled = writable(false);
export const autoSessionCountdown = writable<number | null>(null);

const AUTO_START_THRESHOLD_SECS = 5;
const AUTO_STOP_THRESHOLD_SECS = 3;

let tickInterval: ReturnType<typeof setInterval> | null = null;
let cadenceAboveZeroTicks = 0;
let idleTicks = 0;

/**
 * Whether the rider has stopped, from the freshest available channel.
 * Speed decides if fresh; otherwise cadence if fresh; otherwise unknown (null),
 * which must never auto-stop (no sensors, or a sensor dropout).
 */
export function isRiderIdle(s: {
  speed: number | null;
  speedFresh: boolean;
  cadence: number | null;
  cadenceFresh: boolean;
}): boolean | null {
  if (s.speedFresh) return s.speed === 0;
  if (s.cadenceFresh) return s.cadence === 0;
  return null;
}

function tick() {
  const enabled = get(autoSessionEnabled);
  if (!enabled) {
    resetCounters();
    return;
  }

  const active = get(sessionActive);
  const cadence = get(currentCadence);

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
        startSession().catch((e) => lifecycleError.set(extractError(e)));
      }
    } else {
      cadenceAboveZeroTicks = 0;
      autoSessionCountdown.set(null);
    }
    // Reset stop counter when no session
    idleTicks = 0;
  } else {
    // Auto-stop: rider idle for 3 consecutive ticks; never while paused.
    const idle = get(sessionPaused) ? null : isRiderIdle(getSensorSnapshot());
    if (idle === true) {
      idleTicks++;
      if (idleTicks >= AUTO_STOP_THRESHOLD_SECS) {
        idleTicks = 0;
        void endSession(); // surfaces errors via lifecycleError
      }
    } else {
      idleTicks = 0;
    }
    // Reset start counter when session is active
    cadenceAboveZeroTicks = 0;
    autoSessionCountdown.set(null);
  }
}

function resetCounters() {
  cadenceAboveZeroTicks = 0;
  idleTicks = 0;
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
}
