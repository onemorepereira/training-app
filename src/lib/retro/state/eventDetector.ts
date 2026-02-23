/**
 * Event detector — zone changes, streaks, TSS milestones, ride-best PRs.
 */

import type { GameState, PopupMessage, ZoneStreak } from '../types.js';
import { ZONE_HUD_COLORS } from '../sprites/palette.js';

/** Minimum streak duration in seconds before display */
const STREAK_THRESHOLD = 30;

/** TSS milestone thresholds */
const TSS_MILESTONES = [10, 25, 50, 100, 150, 200];

/** Power duration windows for ride-best detection (in seconds) */
const BEST_WINDOWS = [5, 60, 300, 1200];

/** Minimum watts improvement to trigger a new-best popup */
const BEST_MIN_IMPROVEMENT_W = 5;

/** Minimum % improvement (as fraction) to trigger a new-best popup */
const BEST_MIN_IMPROVEMENT_FRAC = 0.03;

/** Cooldown per window in seconds before another best popup can fire */
const BEST_COOLDOWN = 30;

/** Pop-up display duration in seconds */
const POPUP_DURATION = 2.0;

export interface EventState {
  /** Last known power zone */
  lastZone: number | null;
  /** Time spent in current zone (seconds) */
  zoneTime: number;
  /** Highest TSS milestone already triggered */
  lastMilestone: number;
  /** Rolling power samples: { time, power } */
  powerSamples: Array<{ t: number; w: number }>;
  /** Best average power for each window duration */
  bestPower: Map<number, number>;
  /** Last sample time to avoid duplicates */
  lastSampleTime: number;
  /** Last elapsed time a best popup was shown for each window */
  bestCooldown: Map<number, number>;
}

export function createEventState(): EventState {
  return {
    lastZone: null,
    zoneTime: 0,
    lastMilestone: 0,
    powerSamples: [],
    bestPower: new Map(),
    lastSampleTime: 0,
    bestCooldown: new Map(),
  };
}

/**
 * Detect events and update game state accordingly.
 * Returns any new popup messages to display.
 */
export function detectEvents(
  state: GameState,
  events: EventState,
  dt: number,
): PopupMessage[] {
  const popups: PopupMessage[] = [];
  const zone = state.sensors.powerZone ?? 0;
  const tss = state.sensors.tss ?? 0;
  const power = state.sensors.power;
  const elapsed = state.sensors.elapsedSecs;

  // --- Zone change detection ---
  if (zone !== events.lastZone) {
    if (events.lastZone !== null && events.zoneTime >= STREAK_THRESHOLD) {
      // Previous streak ended — could trigger audio
    }
    events.lastZone = zone;
    events.zoneTime = 0;
  } else {
    events.zoneTime += dt;
  }

  // --- Zone streak tracking ---
  if (zone > 0 && events.zoneTime >= STREAK_THRESHOLD) {
    state.streak = { zone, duration: events.zoneTime };
  } else {
    state.streak = null;
  }

  // --- TSS milestones ---
  for (const milestone of TSS_MILESTONES) {
    if (tss >= milestone && events.lastMilestone < milestone) {
      events.lastMilestone = milestone;
      popups.push({
        text: `TSS ${milestone}!`,
        color: '#ffcc00',
        timeLeft: POPUP_DURATION,
        totalDuration: POPUP_DURATION,
      });
    }
  }

  // --- Ride-best power detection ---
  if (power != null && power > 0 && elapsed > events.lastSampleTime) {
    events.powerSamples.push({ t: elapsed, w: power });
    events.lastSampleTime = elapsed;

    // Prune samples older than the largest window + margin
    const maxWindow = BEST_WINDOWS[BEST_WINDOWS.length - 1];
    const cutoff = elapsed - maxWindow - 5;
    while (events.powerSamples.length > 0 && events.powerSamples[0].t < cutoff) {
      events.powerSamples.shift();
    }

    // Check each window
    for (const window of BEST_WINDOWS) {
      if (elapsed < window) continue;

      const windowStart = elapsed - window;
      const windowSamples = events.powerSamples.filter(s => s.t >= windowStart);
      if (windowSamples.length < 2) continue;

      const avgPower = windowSamples.reduce((sum, s) => sum + s.w, 0) / windowSamples.length;
      const prev = events.bestPower.get(window) ?? 0;

      if (avgPower > prev && avgPower > 0) {
        events.bestPower.set(window, avgPower);

        // Only show popup for meaningful improvements past the initial setting
        const improvement = avgPower - prev;
        const threshold = Math.max(BEST_MIN_IMPROVEMENT_W, prev * BEST_MIN_IMPROVEMENT_FRAC);
        const lastPopupTime = events.bestCooldown.get(window) ?? -Infinity;
        const offCooldown = elapsed - lastPopupTime >= BEST_COOLDOWN;

        if (prev > 0 && improvement >= threshold && offCooldown) {
          events.bestCooldown.set(window, elapsed);
          const label = window < 60 ? `${window}S` : `${window / 60}MIN`;
          popups.push({
            text: `BEST ${label}: ${Math.round(avgPower)}W!`,
            color: '#ff88ff',
            timeLeft: POPUP_DURATION,
            totalDuration: POPUP_DURATION,
          });
        }
      }
    }
  }

  return popups;
}
