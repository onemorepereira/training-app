/**
 * Canvas initialization and pixel-scaling setup.
 * Renders at internal 384x216, CSS-scaled to fill container with crisp pixels.
 */

import { CANVAS_WIDTH, CANVAS_HEIGHT } from '../types.js';

export interface RetroCanvas {
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
}

/**
 * Initialize a canvas element for pixel-art rendering.
 * Sets internal resolution and disables smoothing for crisp upscaling.
 */
export function initCanvas(canvas: HTMLCanvasElement): RetroCanvas {
  canvas.width = CANVAS_WIDTH;
  canvas.height = CANVAS_HEIGHT;

  const ctx = canvas.getContext('2d')!;
  ctx.imageSmoothingEnabled = false;

  return { canvas, ctx };
}

/** Clear the entire canvas to a given color */
export function clearCanvas(ctx: CanvasRenderingContext2D, color: string): void {
  ctx.fillStyle = color;
  ctx.fillRect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT);
}
