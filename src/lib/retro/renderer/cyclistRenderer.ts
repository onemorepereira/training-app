/**
 * Cyclist renderer — draws the player cyclist sprite with effort-based posture.
 * Sprite is drawn at 2x scale for readability on the 384x216 canvas.
 */

import type { CyclistState } from '../types.js';
import { drawCyclist, SPRITE_W, SPRITE_H, SPRITE_SCALE, EFFORT_ADJUSTMENTS } from '../sprites/cyclist.js';
import { getRoadY, getPlayerX } from './roadRenderer.js';

/** Scaled sprite dimensions on screen */
const DRAWN_W = SPRITE_W * SPRITE_SCALE;
const DRAWN_H = SPRITE_H * SPRITE_SCALE;

/**
 * Render the player cyclist on the road.
 * @param zone - Current power zone for palette selection (0-7)
 */
export function renderCyclist(
  ctx: CanvasRenderingContext2D,
  cyclist: CyclistState,
  zone: number,
): void {
  const frame = pedalAngleToFrame(cyclist.pedalAngle);
  const adj = EFFORT_ADJUSTMENTS[cyclist.effort];

  // Place cyclist at fixed position in left quarter of screen
  const x = Math.round(getPlayerX() - DRAWN_W / 2);
  const baseY = getRoadY() - DRAWN_H;

  // Apply bob (synced to pedal angle + effort extra bob)
  const baseBob = Math.sin(cyclist.pedalAngle * 2) * 1;
  const extraBob = Math.sin(cyclist.pedalAngle * 2) * adj.extraBob;
  const y = baseY + Math.round(baseBob + extraBob + cyclist.bobY);

  drawCyclist(ctx, x, y, frame, zone, cyclist.effort);
}

/**
 * Convert continuous pedal angle to discrete frame index (0-3).
 */
function pedalAngleToFrame(angle: number): number {
  const normalized = ((angle % (Math.PI * 2)) + Math.PI * 2) % (Math.PI * 2);
  return Math.floor(normalized / (Math.PI / 2)) % 4;
}
