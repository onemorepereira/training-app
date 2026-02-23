/**
 * Central game state — updated each frame from sensor data.
 */

import type { GameState, SensorSnapshot } from '../types.js';
import { createEffortMachine, updateEffort, type EffortMachine } from './effortMachine.js';

/** Pixels scrolled per km/h per second */
const SPEED_TO_PX = 2.0;

/** Exponential smoothing time constant for climb grade (seconds) */
const CLIMB_SMOOTH_TAU = 1.0;

let effortMachine: EffortMachine = createEffortMachine();

export function createGameState(): GameState {
  effortMachine = createEffortMachine();
  return {
    cyclist: {
      effort: 'idle',
      pedalAngle: 0,
      bobY: 0,
    },
    roadOffset: 0,
    sensors: {
      power: null,
      hr: null,
      cadence: null,
      speed: null,
      powerZone: null,
      elapsedSecs: 0,
      tss: null,
    },
    timeOfDay: 0.35, // start at mid-morning
    popups: [],
    npcs: [],
    streak: null,
    score: 0,
    comboMultiplier: 1,
    muted: true,
    frameCount: 0,
    climbGrade: 0,
  };
}

/**
 * Update game state from sensor snapshot and delta time.
 * @param state - Mutable game state
 * @param sensors - Current sensor readings
 * @param dt - Delta time in seconds (capped externally at 50ms)
 */
export function updateGameState(state: GameState, sensors: SensorSnapshot, dt: number): void {
  state.sensors = sensors;
  state.frameCount++;

  // Effort from power zone
  state.cyclist.effort = updateEffort(effortMachine, sensors.powerZone, dt);

  // Pedal rotation from cadence
  const cadence = sensors.cadence ?? 0;
  if (cadence > 0) {
    // cadence RPM → radians/sec: (rpm / 60) * 2π
    const radsPerSec = (cadence / 60) * Math.PI * 2;
    state.cyclist.pedalAngle = (state.cyclist.pedalAngle + radsPerSec * dt) % (Math.PI * 2);
    // Vertical bob synced to pedal (subtle)
    state.cyclist.bobY = Math.sin(state.cyclist.pedalAngle * 2) * 0.5;
  } else {
    // Idle breathing sway (~2s period) so the scene looks alive
    state.cyclist.bobY = Math.sin(state.frameCount * 0.05) * 0.3;
  }

  // Road scroll from speed
  const speed = sensors.speed ?? 0;
  state.roadOffset += speed * SPEED_TO_PX * dt;

  // Time-of-day cycle: 1 hour real time = full day cycle
  // 3600 secs = 1.0 time-of-day unit
  if (sensors.elapsedSecs > 0) {
    state.timeOfDay = (sensors.elapsedSecs / 3600) % 1;
  }

  // Climb grade: high power zone + low cadence = climbing
  const zone = sensors.powerZone ?? 0;
  const cad = sensors.cadence ?? 90; // default to flat when no cadence data
  const zoneFactor = Math.max(0, Math.min(1, (zone - 2) / 4));
  const cadenceFactor = Math.max(0, Math.min(1, (85 - cad) / 30));
  const targetClimb = zoneFactor * cadenceFactor;
  const alpha = 1 - Math.exp(-dt / CLIMB_SMOOTH_TAU);
  state.climbGrade += alpha * (targetClimb - state.climbGrade);
}
