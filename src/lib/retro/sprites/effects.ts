/**
 * Visual effects — screen shake, chromatic aberration, zone entry flash, scanlines.
 *
 * These are applied as post-processing passes after the main scene render.
 */

import { CANVAS_WIDTH, CANVAS_HEIGHT } from '../types.js';

export interface EffectState {
  /** Screen shake offset X (pixels) */
  shakeX: number;
  /** Screen shake offset Y (pixels) */
  shakeY: number;
  /** Flash alpha (0 = no flash, fades down from trigger) */
  flashAlpha: number;
  /** Whether currently in high-intensity zone (6-7) */
  highIntensity: boolean;
  /** Previous zone for detecting zone entry */
  prevZone: number;
}

export function createEffectState(): EffectState {
  return {
    shakeX: 0,
    shakeY: 0,
    flashAlpha: 0,
    highIntensity: false,
    prevZone: 0,
  };
}

/**
 * Update effect state based on current zone.
 */
export function updateEffects(effects: EffectState, zone: number, dt: number): void {
  const wasHighIntensity = effects.highIntensity;
  effects.highIntensity = zone >= 6;

  // Zone entry flash
  if (effects.highIntensity && !wasHighIntensity) {
    effects.flashAlpha = 0.5;
  }

  // Also flash on any zone entry to 6+
  if (zone !== effects.prevZone && zone >= 6) {
    effects.flashAlpha = 0.4;
  }
  effects.prevZone = zone;

  // Decay flash
  if (effects.flashAlpha > 0) {
    effects.flashAlpha = Math.max(0, effects.flashAlpha - dt * 2);
  }

  // Screen shake for Z6-7
  if (effects.highIntensity) {
    const intensity = zone === 7 ? 2 : 1;
    effects.shakeX = (Math.random() * 2 - 1) * intensity;
    effects.shakeY = (Math.random() * 2 - 1) * intensity;
  } else {
    effects.shakeX = 0;
    effects.shakeY = 0;
  }
}

/**
 * Apply pre-render effects: translate canvas for screen shake.
 * Call this BEFORE the main render pass.
 */
export function applyPreEffects(ctx: CanvasRenderingContext2D, effects: EffectState): void {
  if (effects.shakeX !== 0 || effects.shakeY !== 0) {
    ctx.save();
    ctx.translate(Math.round(effects.shakeX), Math.round(effects.shakeY));
  }
}

/**
 * Apply post-render effects: flash overlay, chromatic aberration, scanlines.
 * Call this AFTER the main render pass.
 */
export function applyPostEffects(ctx: CanvasRenderingContext2D, effects: EffectState): void {
  // Restore shake transform
  if (effects.shakeX !== 0 || effects.shakeY !== 0) {
    ctx.restore();
  }

  // Zone entry flash
  if (effects.flashAlpha > 0) {
    ctx.fillStyle = `rgba(255, 255, 255, ${effects.flashAlpha})`;
    ctx.fillRect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT);
  }

  // Chromatic aberration for Z6-7: shift R channel slightly
  if (effects.highIntensity) {
    // Simple implementation: draw a semi-transparent tinted overlay shifted by 1px
    ctx.globalAlpha = 0.08;
    ctx.globalCompositeOperation = 'screen';
    ctx.drawImage(ctx.canvas, 1, 0);
    ctx.globalCompositeOperation = 'source-over';
    ctx.globalAlpha = 1;
  }

  // Scanlines (subtle, always-on)
  ctx.fillStyle = 'rgba(0, 0, 0, 0.06)';
  for (let y = 0; y < CANVAS_HEIGHT; y += 2) {
    ctx.fillRect(0, y, CANVAS_WIDTH, 1);
  }
}
