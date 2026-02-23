/**
 * Effort state machine — smoothly interpolates between power zone effort levels.
 * Transitions take ~300ms to prevent jarring posture snaps.
 */

import { EFFORT_LEVELS, zoneToEffort, type EffortLevel } from '../types.js';

/** Interpolation duration in seconds */
const TRANSITION_DURATION = 0.3;

export interface EffortMachine {
  /** Current interpolated effort level */
  current: EffortLevel;
  /** Target effort level from latest zone reading */
  target: EffortLevel;
  /** Progress through current transition [0, 1] */
  progress: number;
}

export function createEffortMachine(): EffortMachine {
  return {
    current: 'idle',
    target: 'idle',
    progress: 1,
  };
}

/**
 * Update effort machine with new zone and delta time.
 * Returns the effort level to use for rendering.
 */
export function updateEffort(machine: EffortMachine, zone: number | null, dt: number): EffortLevel {
  const newTarget = zoneToEffort(zone);

  if (newTarget !== machine.target) {
    // New target — start transition from current
    machine.current = machine.target;
    machine.target = newTarget;
    machine.progress = 0;
  }

  if (machine.progress < 1) {
    machine.progress = Math.min(1, machine.progress + dt / TRANSITION_DURATION);
  }

  // Return the target once transition is >50% complete, otherwise current.
  // This gives a snappy but not instant feel.
  return machine.progress >= 0.5 ? machine.target : machine.current;
}

/**
 * Get the numeric effort index (0-6) for interpolation purposes.
 */
export function effortIndex(effort: EffortLevel): number {
  const idx = EFFORT_LEVELS.indexOf(effort);
  return idx >= 0 ? idx : 0;
}
