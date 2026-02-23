/**
 * Roadside scenery renderer — procedurally places trees, houses, markers,
 * and signs on the grass bands flanking the road. Objects scroll with the
 * road at parallax speeds matching their depth.
 */

import { CANVAS_WIDTH } from '../types.js';

// ── Road geometry (must match roadRenderer.ts) ──────────────────────
const SKY_BOTTOM = 96;
const ROAD_TOP = 120;
const ROAD_BOTTOM = 168;

// ── Parallax speeds ─────────────────────────────────────────────────
const UPPER_PARALLAX = 0.5;
const LOWER_PARALLAX = 0.8;

// ── Slot spacing (world-space pixels between placement slots) ───────
const UPPER_SLOT_SPACING = 60;
const LOWER_SLOT_SPACING = 80;

// ── Culling margin ──────────────────────────────────────────────────
const MARGIN = 50;

// ── Road message placement ──────────────────────────────────────────
const MSG_SLOT_SPACING = 500; // world-space px between message slots
const MSG_PARALLAX = 1.0;    // painted on road, scrolls with surface

const ROAD_MESSAGES = [
  'ALLEZ',
  'PUSH',
  'VAMOS',
  'FORZA',
  'GO',
  'VENGA',
  'RIDE',
  'OLE',
  'POWER',
  'UP',
];

// Two paint colors: white and yellow, like real road graffiti
const MSG_COLORS = ['#8a8a78', '#9a9060'];

// ── 3x5 pixel font for road paint ──────────────────────────────────
// Each glyph: array of 5 rows, each row a bitmask (3 bits wide, MSB=left)
// Bit layout: 0b100=left, 0b010=center, 0b001=right
const PIXEL_FONT: Record<string, number[]> = {
  A: [0b010, 0b101, 0b111, 0b101, 0b101],
  D: [0b110, 0b101, 0b101, 0b101, 0b110],
  E: [0b111, 0b100, 0b110, 0b100, 0b111],
  F: [0b111, 0b100, 0b110, 0b100, 0b100],
  G: [0b011, 0b100, 0b101, 0b101, 0b011],
  H: [0b101, 0b101, 0b111, 0b101, 0b101],
  I: [0b111, 0b010, 0b010, 0b010, 0b111],
  L: [0b100, 0b100, 0b100, 0b100, 0b111],
  M: [0b111, 0b101, 0b101, 0b101, 0b101],
  N: [0b101, 0b111, 0b101, 0b101, 0b101],
  O: [0b010, 0b101, 0b101, 0b101, 0b010],
  P: [0b110, 0b101, 0b110, 0b100, 0b100],
  R: [0b110, 0b101, 0b110, 0b101, 0b101],
  S: [0b011, 0b100, 0b010, 0b001, 0b110],
  T: [0b111, 0b010, 0b010, 0b010, 0b010],
  U: [0b101, 0b101, 0b101, 0b101, 0b010],
  V: [0b101, 0b101, 0b101, 0b010, 0b010],
  W: [0b101, 0b101, 0b111, 0b111, 0b101],
  Z: [0b111, 0b001, 0b010, 0b100, 0b111],
};

// Glyph dimensions (in font-pixels)
const GLYPH_W = 3;
const GLYPH_H = 5;

// ── Scenery colors ──────────────────────────────────────────────────
const TRUNK = '#6b4226';
const CANOPY_DARK = '#2d5e2d';
const CANOPY_LIGHT = '#3a7a3a';
const HOUSE_WALL = '#c4a882';
const HOUSE_ROOF = '#8b4513';
const HOUSE_WINDOW = '#3355aa';
const MARKER_POST = '#ddddcc';
const SIGN_FACE = '#ddddcc';
const SIGN_POST = '#888877';

// ── Simple integer hash (deterministic, no randomness) ──────────────
function hash(n: number): number {
  let h = (n * 2654435761) | 0;
  h = ((h >>> 16) ^ h) * 0x45d9f3b | 0;
  h = ((h >>> 16) ^ h) | 0;
  return h >>> 0; // unsigned
}

// ── Object types per zone ───────────────────────────────────────────
const UPPER_TYPES = 3; // pine, deciduous, house
const LOWER_TYPES = 3; // large pine, km marker, road sign

// ── Draw functions ──────────────────────────────────────────────────

/** Small pine tree on upper grass (~20px tall, 2x scale) */
function drawSmallPine(ctx: CanvasRenderingContext2D, x: number, baseY: number): void {
  // Trunk: 4x6
  ctx.fillStyle = TRUNK;
  ctx.fillRect(x - 2, baseY - 6, 4, 6);
  // Canopy: triangle as stacked rects (dark)
  ctx.fillStyle = CANOPY_DARK;
  ctx.fillRect(x - 2, baseY - 10, 4, 4);
  ctx.fillRect(x - 4, baseY - 14, 8, 4);
  ctx.fillRect(x - 6, baseY - 18, 12, 4);
  // Highlight on left side
  ctx.fillStyle = CANOPY_LIGHT;
  ctx.fillRect(x - 2, baseY - 10, 2, 4);
  ctx.fillRect(x - 4, baseY - 14, 2, 4);
  ctx.fillRect(x - 6, baseY - 18, 2, 4);
}

/** Small deciduous tree on upper grass (~18px tall) */
function drawDeciduousTree(ctx: CanvasRenderingContext2D, x: number, baseY: number): void {
  // Trunk: 4x6
  ctx.fillStyle = TRUNK;
  ctx.fillRect(x - 2, baseY - 6, 4, 6);
  // Round canopy: layered rects for circular shape
  ctx.fillStyle = CANOPY_DARK;
  ctx.fillRect(x - 4, baseY - 10, 8, 4);
  ctx.fillRect(x - 6, baseY - 14, 12, 4);
  ctx.fillRect(x - 4, baseY - 18, 8, 4);
  // Highlight
  ctx.fillStyle = CANOPY_LIGHT;
  ctx.fillRect(x - 4, baseY - 14, 4, 4);
  ctx.fillRect(x - 2, baseY - 18, 4, 4);
}

/** House on upper grass (~20px tall) */
function drawHouse(ctx: CanvasRenderingContext2D, x: number, baseY: number, seed: number): void {
  const width = 16;
  const wallH = 10;
  const roofH = 8;
  const left = x - width / 2;
  // Wall
  ctx.fillStyle = HOUSE_WALL;
  ctx.fillRect(left, baseY - wallH, width, wallH);
  // Roof (triangle as stacked rects)
  ctx.fillStyle = HOUSE_ROOF;
  ctx.fillRect(left - 2, baseY - wallH - 2, width + 4, 2);
  ctx.fillRect(left, baseY - wallH - 4, width, 2);
  ctx.fillRect(left + 2, baseY - wallH - 6, width - 4, 2);
  ctx.fillRect(left + 4, baseY - wallH - roofH, width - 8, 2);
  // Window (position varies by seed)
  ctx.fillStyle = HOUSE_WINDOW;
  const winX = left + 2 + (seed % 3) * 4;
  ctx.fillRect(winX, baseY - wallH + 2, 4, 4);
  // Door
  ctx.fillStyle = TRUNK;
  ctx.fillRect(left + width / 2 - 2, baseY - 6, 4, 6);
}

/** Large pine tree on lower grass (~40px tall) */
function drawLargePine(ctx: CanvasRenderingContext2D, x: number, baseY: number): void {
  // Trunk: 6x10
  ctx.fillStyle = TRUNK;
  ctx.fillRect(x - 3, baseY - 10, 6, 10);
  // Canopy tiers (bigger)
  ctx.fillStyle = CANOPY_DARK;
  ctx.fillRect(x - 4, baseY - 16, 8, 6);
  ctx.fillRect(x - 6, baseY - 24, 12, 8);
  ctx.fillRect(x - 8, baseY - 32, 16, 8);
  ctx.fillRect(x - 10, baseY - 38, 20, 6);
  // Highlights
  ctx.fillStyle = CANOPY_LIGHT;
  ctx.fillRect(x - 4, baseY - 16, 4, 6);
  ctx.fillRect(x - 6, baseY - 24, 4, 8);
  ctx.fillRect(x - 8, baseY - 32, 4, 8);
  ctx.fillRect(x - 10, baseY - 38, 4, 6);
}

/** Km marker on lower grass (~12px tall) */
function drawKmMarker(ctx: CanvasRenderingContext2D, x: number, baseY: number): void {
  // Post: 2x12
  ctx.fillStyle = MARKER_POST;
  ctx.fillRect(x - 1, baseY - 12, 2, 12);
  // Top cap: 4x2
  ctx.fillRect(x - 2, baseY - 14, 4, 2);
}

/** Road sign on lower grass (~16px tall) */
function drawRoadSign(ctx: CanvasRenderingContext2D, x: number, baseY: number): void {
  // Post: 2x10
  ctx.fillStyle = SIGN_POST;
  ctx.fillRect(x - 1, baseY - 10, 2, 10);
  // Sign face: 8x6
  ctx.fillStyle = SIGN_FACE;
  ctx.fillRect(x - 4, baseY - 16, 8, 6);
  // Border
  ctx.fillStyle = SIGN_POST;
  ctx.fillRect(x - 4, baseY - 16, 8, 1);
  ctx.fillRect(x - 4, baseY - 11, 8, 1);
  ctx.fillRect(x - 4, baseY - 16, 1, 6);
  ctx.fillRect(x + 3, baseY - 16, 1, 6);
}

// ── Road messages ───────────────────────────────────────────────────

/** Draw a single pixel-font glyph at a given pixel scale. */
function drawGlyph(
  ctx: CanvasRenderingContext2D,
  glyph: number[],
  x: number,
  y: number,
  scale: number,
): void {
  for (let row = 0; row < GLYPH_H; row++) {
    const bits = glyph[row];
    for (let col = 0; col < GLYPH_W; col++) {
      if (bits & (1 << (GLYPH_W - 1 - col))) {
        ctx.fillRect(
          x + col * scale,
          y + row * scale,
          scale,
          scale,
        );
      }
    }
  }
}

/**
 * Render motivational messages painted across the road surface as pixel-art.
 * Text is drawn horizontally then rotated 90° CW so it spans the road width
 * perpendicular to the direction of travel — readable from the rider's POV
 * (top = far side, bottom = near side, like real road graffiti).
 */
function renderRoadMessages(
  ctx: CanvasRenderingContext2D,
  scrollOffset: number,
): void {
  const worldOffset = scrollOffset * MSG_PARALLAX;
  const firstSlot = Math.floor((worldOffset - MARGIN * 2) / MSG_SLOT_SPACING);
  const lastSlot = Math.ceil((worldOffset + CANVAS_WIDTH + MARGIN * 2) / MSG_SLOT_SPACING);

  const roadH = ROAD_BOTTOM - ROAD_TOP;
  const roadCenterY = (ROAD_TOP + ROAD_BOTTOM) / 2;

  ctx.save();
  ctx.globalAlpha = 0.35;

  for (let slot = firstSlot; slot <= lastSlot; slot++) {
    const h = hash(slot * 6247);
    if (h % 4 !== 0) continue;

    const msgIdx = h % ROAD_MESSAGES.length;
    const msg = ROAD_MESSAGES[msgIdx];
    const colorIdx = hash(slot * 4391) % MSG_COLORS.length;
    const n = msg.length;
    if (n === 0) continue;

    // Compute scale so the horizontal text width (which becomes vertical
    // height after rotation) fills most of the road height.
    // Horizontal width = n * GLYPH_W * s + (n-1) * s
    // We want that ≈ roadH, so s = roadH / (n * GLYPH_W + n - 1)
    const scale = Math.floor(roadH / (n * GLYPH_W + n - 1)) || 1;
    const charW = GLYPH_W * scale;
    const charH = GLYPH_H * scale;
    const gap = scale;
    const textW = n * charW + (n - 1) * gap; // horizontal width before rotation

    const worldX = slot * MSG_SLOT_SPACING + (hash(slot * 2903) % 60) - 30;
    const screenX = Math.round(worldX - worldOffset);

    // After 90° CW rotation, textW becomes vertical span, charH becomes horizontal span
    if (screenX + charH / 2 < -MARGIN || screenX - charH / 2 > CANVAS_WIDTH + MARGIN) continue;

    ctx.fillStyle = MSG_COLORS[colorIdx];

    // Rotate 90° CW around the road center point at this X position
    ctx.save();
    ctx.translate(screenX, roadCenterY);
    ctx.rotate(Math.PI / 2);
    // Now draw horizontal text centered at the origin
    // After rotation: +X (text direction) points downward on screen
    //                 +Y points leftward (into the road scroll direction)
    let cursorX = -textW / 2;
    for (let i = 0; i < n; i++) {
      const glyph = PIXEL_FONT[msg[i]];
      if (glyph) {
        drawGlyph(ctx, glyph, Math.round(cursorX), Math.round(-charH / 2), scale);
      }
      cursorX += charW + gap;
    }
    ctx.restore();
  }

  ctx.restore();
}

// ── Main entry point ────────────────────────────────────────────────

/**
 * Render roadside scenery (trees, houses, markers) on both grass bands,
 * plus motivational messages painted on the road.
 * Called between renderRoad and renderNpcs inside the tilt transform block.
 */
export function renderScenery(
  ctx: CanvasRenderingContext2D,
  scrollOffset: number,
): void {
  // === Road messages (painted on surface, under everything else) ===
  renderRoadMessages(ctx, scrollOffset);

  // === Upper grass scenery (background, slower parallax) ===
  renderZone(ctx, scrollOffset, {
    parallax: UPPER_PARALLAX,
    slotSpacing: UPPER_SLOT_SPACING,
    yMin: SKY_BOTTOM + 4,
    yMax: ROAD_TOP - 2,
    numTypes: UPPER_TYPES,
    drawObject: drawUpperObject,
  });

  // === Lower grass scenery (foreground, faster parallax) ===
  renderZone(ctx, scrollOffset, {
    parallax: LOWER_PARALLAX,
    slotSpacing: LOWER_SLOT_SPACING,
    yMin: ROAD_BOTTOM + 4,
    yMax: ROAD_BOTTOM + 40,
    numTypes: LOWER_TYPES,
    drawObject: drawLowerObject,
  });
}

interface ZoneConfig {
  parallax: number;
  slotSpacing: number;
  yMin: number;
  yMax: number;
  numTypes: number;
  drawObject: (ctx: CanvasRenderingContext2D, type: number, x: number, y: number, seed: number) => void;
}

function renderZone(
  ctx: CanvasRenderingContext2D,
  scrollOffset: number,
  config: ZoneConfig,
): void {
  const worldOffset = scrollOffset * config.parallax;
  // Find first slot that could be visible
  const firstSlot = Math.floor((worldOffset - MARGIN) / config.slotSpacing);
  const lastSlot = Math.ceil((worldOffset + CANVAS_WIDTH + MARGIN) / config.slotSpacing);

  for (let slot = firstSlot; slot <= lastSlot; slot++) {
    const h = hash(slot * 7919); // prime multiplier for variety
    // ~40% of slots have objects (skip empty slots for natural spacing)
    if (h % 10 > 3) continue;

    const type = h % config.numTypes;
    const worldX = slot * config.slotSpacing + (hash(slot * 1301) % 20) - 10; // X jitter +-10
    const screenX = Math.round(worldX - worldOffset);

    if (screenX < -MARGIN || screenX > CANVAS_WIDTH + MARGIN) continue;

    // Y jitter within grass band
    const yRange = config.yMax - config.yMin;
    const baseY = config.yMin + (hash(slot * 3571) % Math.max(yRange, 1));

    config.drawObject(ctx, type, screenX, baseY, h);
  }
}

function drawUpperObject(
  ctx: CanvasRenderingContext2D,
  type: number,
  x: number,
  y: number,
  seed: number,
): void {
  switch (type) {
    case 0: drawSmallPine(ctx, x, y); break;
    case 1: drawDeciduousTree(ctx, x, y); break;
    case 2: drawHouse(ctx, x, y, seed); break;
  }
}

function drawLowerObject(
  ctx: CanvasRenderingContext2D,
  type: number,
  x: number,
  y: number,
  _seed: number,
): void {
  switch (type) {
    case 0: drawLargePine(ctx, x, y); break;
    case 1: drawKmMarker(ctx, x, y); break;
    case 2: drawRoadSign(ctx, x, y); break;
  }
}
