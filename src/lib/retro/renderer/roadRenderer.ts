/**
 * Road renderer — side-scrolling horizontal road with gradient sky,
 * parallax hills, and grass dither.
 */

import { CANVAS_WIDTH, CANVAS_HEIGHT } from '../types.js';
import { ENV_COLORS } from '../sprites/palette.js';

/** Sky ends / horizon row */
export const SKY_BOTTOM = 96;

/** Road band boundaries */
const ROAD_TOP = 120;
const ROAD_BOTTOM = 168;

/** Cyclist feet Y position (on road surface) */
const ROAD_Y = 160;

/** Cyclist screen X position (left quarter of screen) */
const PLAYER_X = 80;

/** Center-line Y (midway through road) */
const CENTER_LINE_Y = Math.round((ROAD_TOP + ROAD_BOTTOM) / 2);

// Sky gradient endpoints (same palette as before)
const SKY_R1 = 0x33, SKY_G1 = 0x55, SKY_B1 = 0xaa; // top
const SKY_R2 = 0x99, SKY_G2 = 0xbb, SKY_B2 = 0xdd; // horizon

// Hill silhouette: 48 height values that tile across the canvas width.
// Values represent how many pixels above SKY_BOTTOM the hill peaks.
const HILL_HEIGHTS = [
  3, 4, 5, 7, 8, 9, 10, 10, 11, 11, 10, 9, 8, 7, 6, 5,
  4, 4, 5, 6, 8, 10, 12, 14, 15, 15, 14, 13, 11, 9, 7, 6,
  5, 4, 4, 5, 6, 7, 8, 8, 7, 6, 5, 4, 3, 3, 3, 3,
];

/** Shoulder strip width in pixels */
const SHOULDER_W = 3;

/**
 * Render the full side-scrolling scene background.
 */
export function renderRoad(
  ctx: CanvasRenderingContext2D,
  scrollOffset: number,
  climbGrade: number = 0,
): void {
  // === Sky gradient (per-scanline) ===
  for (let y = 0; y < SKY_BOTTOM; y++) {
    const t = y / SKY_BOTTOM;
    const r = Math.round(SKY_R1 + (SKY_R2 - SKY_R1) * t);
    const g = Math.round(SKY_G1 + (SKY_G2 - SKY_G1) * t);
    const b = Math.round(SKY_B1 + (SKY_B2 - SKY_B1) * t);
    ctx.fillStyle = `rgb(${r},${g},${b})`;
    ctx.fillRect(0, y, CANVAS_WIDTH, 1);
  }

  // Horizon haze line
  ctx.fillStyle = ENV_COLORS.skyHorizon;
  ctx.fillRect(0, SKY_BOTTOM - 2, CANVAS_WIDTH, 3);

  // === Hill silhouette (parallax boosted during climbs) ===
  const hillParallax = 0.3 + climbGrade * 0.15;
  const hillOffset = scrollOffset * hillParallax;
  const hillColor = '#2a4a2a';
  ctx.fillStyle = hillColor;
  const tileLen = HILL_HEIGHTS.length;
  for (let x = 0; x < CANVAS_WIDTH; x++) {
    const idx = Math.floor(((x + hillOffset) % (tileLen * 8)) / 8 + tileLen) % tileLen;
    const h = HILL_HEIGHTS[idx];
    ctx.fillRect(x, SKY_BOTTOM - h, 1, h);
  }

  // === Upper grass (SKY_BOTTOM to ROAD_TOP) ===
  renderGrass(ctx, SKY_BOTTOM, ROAD_TOP, scrollOffset, 0.8);

  // === Shoulder strip at road top edge ===
  ctx.fillStyle = ENV_COLORS.shoulder;
  ctx.fillRect(0, ROAD_TOP, CANVAS_WIDTH, SHOULDER_W);

  // === Road surface ===
  ctx.fillStyle = ENV_COLORS.road;
  ctx.fillRect(0, ROAD_TOP + SHOULDER_W, CANVAS_WIDTH, ROAD_BOTTOM - ROAD_TOP - SHOULDER_W * 2);

  // === Shoulder strip at road bottom edge ===
  ctx.fillStyle = ENV_COLORS.shoulder;
  ctx.fillRect(0, ROAD_BOTTOM - SHOULDER_W, CANVAS_WIDTH, SHOULDER_W);

  // === Road edge lines ===
  ctx.fillStyle = ENV_COLORS.roadLine;
  ctx.fillRect(0, ROAD_TOP, CANVAS_WIDTH, 1);
  ctx.fillRect(0, ROAD_BOTTOM - 1, CANVAS_WIDTH, 1);

  // === Center dashed line (scrolls with road) ===
  const dashLen = 10;
  const gapLen = 8;
  const pattern = dashLen + gapLen;
  ctx.fillStyle = ENV_COLORS.roadLine;
  for (let x = -pattern; x < CANVAS_WIDTH + pattern; x += pattern) {
    const sx = Math.round(x - (scrollOffset % pattern));
    if (sx + dashLen > 0 && sx < CANVAS_WIDTH) {
      const clampedX = Math.max(0, sx);
      const clampedW = Math.min(dashLen, CANVAS_WIDTH - clampedX, sx + dashLen - clampedX);
      if (clampedW > 0) {
        ctx.fillRect(clampedX, CENTER_LINE_Y, clampedW, 1);
      }
    }
  }

  // === Lower grass (ROAD_BOTTOM to canvas bottom) ===
  renderGrass(ctx, ROAD_BOTTOM, CANVAS_HEIGHT, scrollOffset, 0.8);
}

/**
 * Render a grass band with dither highlights that scroll.
 */
function renderGrass(
  ctx: CanvasRenderingContext2D,
  yStart: number,
  yEnd: number,
  scrollOffset: number,
  scrollFactor: number,
): void {
  // Solid grass base
  ctx.fillStyle = ENV_COLORS.grassNear;
  ctx.fillRect(0, yStart, CANVAS_WIDTH, yEnd - yStart);

  // Dither highlights — scattered pixels that scroll
  ctx.fillStyle = ENV_COLORS.grassHighlight;
  const shift = Math.floor(scrollOffset * scrollFactor);
  for (let y = yStart; y < yEnd; y++) {
    const lineIdx = y - yStart;
    if (lineIdx % 3 !== 0) continue;
    for (let gx = 0; gx < CANVAS_WIDTH; gx += 7) {
      const offset = (lineIdx * 3 + (gx + shift) * 7) % 11;
      if (offset < 3) {
        const px = (gx + offset + shift) % CANVAS_WIDTH;
        ctx.fillRect(px, y, 1, 1);
      }
    }
  }
}

/** Get the Y position where the cyclist feet touch the road. */
export function getRoadY(): number {
  return ROAD_Y;
}

/** Get the screen X position for the player cyclist. */
export function getPlayerX(): number {
  return PLAYER_X;
}
