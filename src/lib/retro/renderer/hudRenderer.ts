/**
 * HUD renderer — pixel-font overlays for power, HR, cadence, zone meter, score, time.
 * Drawn with shadow text for readability against the scene.
 */

import { CANVAS_WIDTH, CANVAS_HEIGHT, type GameState } from '../types.js';
import { ZONE_HUD_COLORS } from '../sprites/palette.js';
import { drawText, measureText } from '../pixelFont.js';

const PAD = 6;
const ZONE_BAR_W = 10;
const ZONE_BAR_H = 5;
const ZONE_BAR_GAP = 2;

/** Draw text with a 1px dark shadow for readability */
function drawShadowText(
  ctx: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  color: string,
  scale: number = 1,
): void {
  drawText(ctx, text, x + 1, y + 1, 'rgba(0,0,0,0.6)', scale);
  drawText(ctx, text, x, y, color, scale);
}

/**
 * Render the full HUD overlay.
 */
export function renderHud(ctx: CanvasRenderingContext2D, state: GameState): void {
  const { sensors, score, streak, comboMultiplier } = state;
  const zone = sensors.powerZone ?? 0;
  const zoneColor = ZONE_HUD_COLORS[Math.min(Math.max(zone, 0), 7)];

  // === Top-left: Power (large) ===
  const powerStr = sensors.power != null ? Math.round(sensors.power).toString() : '--';
  drawShadowText(ctx, powerStr, PAD, PAD, zoneColor, 2);
  const powerW = measureText(powerStr, 2);
  drawShadowText(ctx, 'W', PAD + powerW + 3, PAD + 8, zoneColor, 1);

  // === Top-right: HR and Cadence ===
  const hrStr = sensors.hr != null ? Math.round(sensors.hr).toString() : '--';
  const cadStr = sensors.cadence != null ? Math.round(sensors.cadence).toString() : '--';

  const hrLabel = hrStr + ' BPM';
  const cadLabel = cadStr + ' RPM';
  const hrW = measureText(hrLabel, 1);
  const cadW = measureText(cadLabel, 1);

  drawShadowText(ctx, hrLabel, CANVAS_WIDTH - PAD - hrW, PAD, '#ee5555', 1);
  drawShadowText(ctx, cadLabel, CANVAS_WIDTH - PAD - cadW, PAD + 10, '#55bbee', 1);

  // === Bottom-left: Score + combo ===
  const scoreStr = 'SCORE ' + Math.floor(score).toString();
  drawShadowText(ctx, scoreStr, PAD, CANVAS_HEIGHT - PAD - 8, '#ffffff', 1);
  if (comboMultiplier > 1) {
    const comboStr = 'x' + comboMultiplier;
    const scoreW = measureText(scoreStr, 1);
    drawShadowText(ctx, comboStr, PAD + scoreW + 4, CANVAS_HEIGHT - PAD - 8, '#ffcc44', 1);
  }

  // === Bottom-center: Elapsed time ===
  const elapsed = sensors.elapsedSecs;
  const mins = Math.floor(elapsed / 60);
  const secs = Math.floor(elapsed % 60);
  const timeStr = mins.toString().padStart(2, '0') + ':' + secs.toString().padStart(2, '0');
  const timeW = measureText(timeStr, 1);
  drawShadowText(ctx, timeStr, Math.round((CANVAS_WIDTH - timeW) / 2), CANVAS_HEIGHT - PAD - 8, '#cccccc', 1);

  // === Zone streak indicator (above time) ===
  if (streak && streak.duration >= 30) {
    const streakMins = Math.floor(streak.duration / 60);
    const streakSecs = Math.floor(streak.duration % 60);
    const streakStr = 'ZONE ' + streak.zone + ' x ' +
      streakMins.toString() + ':' + streakSecs.toString().padStart(2, '0');
    const streakW = measureText(streakStr, 1);
    const streakColor = ZONE_HUD_COLORS[Math.min(streak.zone, 7)];
    drawShadowText(ctx, streakStr, Math.round((CANVAS_WIDTH - streakW) / 2), CANVAS_HEIGHT - PAD - 18, streakColor, 1);
  }

  // === Bottom-right: Zone meter ===
  renderZoneMeter(ctx, zone, state.frameCount);
}

/**
 * Draw a 7-segment zone meter in the bottom-right corner.
 * Active zone is bright + pulsing, others are dimmed.
 */
function renderZoneMeter(ctx: CanvasRenderingContext2D, activeZone: number, frame: number): void {
  const totalW = 7 * (ZONE_BAR_W + ZONE_BAR_GAP) - ZONE_BAR_GAP;
  const startX = CANVAS_WIDTH - PAD - totalW;
  const y = CANVAS_HEIGHT - PAD - ZONE_BAR_H;

  // Background strip for contrast
  ctx.fillStyle = 'rgba(0, 0, 0, 0.3)';
  ctx.fillRect(startX - 2, y - 2, totalW + 4, ZONE_BAR_H + 4);

  for (let z = 1; z <= 7; z++) {
    const x = startX + (z - 1) * (ZONE_BAR_W + ZONE_BAR_GAP);
    const isActive = z === activeZone;

    // Pulse active zone
    const pulse = isActive ? 0.85 + Math.sin(frame * 0.1) * 0.15 : 0.25;
    ctx.globalAlpha = pulse;
    ctx.fillStyle = ZONE_HUD_COLORS[z];
    ctx.fillRect(x, y, ZONE_BAR_W, ZONE_BAR_H);
  }

  ctx.globalAlpha = 1.0;
}
