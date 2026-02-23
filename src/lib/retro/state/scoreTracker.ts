/**
 * Score tracker — TSS×100 base score with zone-fidelity combo multiplier.
 *
 * Combo multiplier based on time in same zone:
 *   0-30s:   ×1
 *   30-60s:  ×2
 *   60-120s: ×3
 *   120s+:   ×4
 */

import type { GameState } from '../types.js';

export interface ScoreState {
  /** Last TSS reading for delta calculation */
  lastTss: number;
}

export function createScoreState(): ScoreState {
  return { lastTss: 0 };
}

/**
 * Calculate combo multiplier from zone streak duration.
 */
function comboFromStreak(streakSecs: number): number {
  if (streakSecs >= 120) return 4;
  if (streakSecs >= 60) return 3;
  if (streakSecs >= 30) return 2;
  return 1;
}

/**
 * Update score based on TSS delta and zone streak.
 */
export function updateScore(state: GameState, scoreState: ScoreState): void {
  const tss = state.sensors.tss ?? 0;

  // Combo multiplier from streak
  const streakDuration = state.streak?.duration ?? 0;
  state.comboMultiplier = comboFromStreak(streakDuration);

  // Score from TSS delta
  const tssDelta = tss - scoreState.lastTss;
  if (tssDelta > 0) {
    state.score += tssDelta * 100 * state.comboMultiplier;
  }
  scoreState.lastTss = tss;
}
