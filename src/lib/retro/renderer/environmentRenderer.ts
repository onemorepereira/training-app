/**
 * Environment renderer — time-of-day lighting and weather particles.
 *
 * Time-of-day: driven by elapsed_secs (1h = full cycle)
 *   0.00-0.15: night (dark overlay, stars)
 *   0.15-0.30: sunrise (warm orange tint fading in)
 *   0.30-0.70: day (no overlay)
 *   0.70-0.85: sunset (warm orange/purple tint)
 *   0.85-1.00: dusk->night (dark overlay increasing)
 *
 * Weather: driven by cumulative TSS
 *   0-30:   clear
 *   30-60:  clouds (grey patches)
 *   60-100: rain (diagonal streaks)
 *   100+:   storm (rain + lightning flashes)
 */

import { CANVAS_WIDTH, CANVAS_HEIGHT, type GameState } from '../types.js';

/** Sky bottom to match roadRenderer */
const HORIZON_Y = 96;

/** Simple pseudo-random from seed for deterministic particles */
function seededRandom(seed: number): number {
  const x = Math.sin(seed * 12.9898 + seed * 78.233) * 43758.5453;
  return x - Math.floor(x);
}

/**
 * Render environment overlays after the main scene.
 */
export function renderEnvironment(ctx: CanvasRenderingContext2D, state: GameState): void {
  renderTimeOfDay(ctx, state.timeOfDay, state.frameCount);
  renderWeather(ctx, state.sensors.tss ?? 0, state.frameCount);
}

function renderTimeOfDay(ctx: CanvasRenderingContext2D, tod: number, frame: number): void {
  // Night overlay
  if (tod < 0.15 || tod > 0.85) {
    const nightIntensity = tod < 0.15
      ? 1 - tod / 0.15
      : (tod - 0.85) / 0.15;
    const alpha = 0.45 * nightIntensity;

    ctx.fillStyle = `rgba(8, 8, 32, ${alpha})`;
    ctx.fillRect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT);

    // Stars (only in sky area)
    if (nightIntensity > 0.3) {
      for (let i = 0; i < 40; i++) {
        const sx = Math.floor(seededRandom(i * 7 + 1) * CANVAS_WIDTH);
        const sy = Math.floor(seededRandom(i * 7 + 2) * (HORIZON_Y - 8));
        // Twinkling
        const twinkle = seededRandom(i * 7 + frame * 0.008);
        if (twinkle > 0.25) {
          const brightness = 0.5 + twinkle * 0.5;
          ctx.fillStyle = `rgba(255, 255, 240, ${nightIntensity * brightness * 0.9})`;
          ctx.fillRect(sx, sy, 1, 1);
          // Brighter stars get a + shape
          if (twinkle > 0.8) {
            ctx.fillRect(sx - 1, sy, 1, 1);
            ctx.fillRect(sx + 1, sy, 1, 1);
          }
        }
      }
    }
  }

  // Sunrise warm tint
  if (tod >= 0.15 && tod < 0.30) {
    const intensity = 1 - (tod - 0.15) / 0.15;
    ctx.fillStyle = `rgba(255, 140, 50, ${0.12 * intensity})`;
    ctx.fillRect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT);
  }

  // Sunset warm tint
  if (tod >= 0.70 && tod < 0.85) {
    const intensity = (tod - 0.70) / 0.15;
    ctx.fillStyle = `rgba(255, 100, 60, ${0.15 * intensity})`;
    ctx.fillRect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT);
    ctx.fillStyle = `rgba(80, 20, 120, ${0.08 * intensity})`;
    ctx.fillRect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT);
  }
}

function renderWeather(ctx: CanvasRenderingContext2D, tss: number, frame: number): void {
  if (tss < 30) return;

  // Clouds: grey overlay patches in sky area (TSS 30+)
  if (tss >= 30) {
    const cloudAlpha = Math.min(0.2, (tss - 30) / 150);
    ctx.fillStyle = `rgba(100, 105, 120, ${cloudAlpha})`;

    for (let i = 0; i < 5; i++) {
      const cx = ((seededRandom(i * 13) * CANVAS_WIDTH + frame * 0.15 * (i % 2 === 0 ? 1 : 0.5)) % (CANVAS_WIDTH + 80)) - 40;
      const cy = seededRandom(i * 13 + 3) * (HORIZON_Y - 20) + 6;
      const cw = 25 + seededRandom(i * 13 + 5) * 50;
      const ch = 6 + seededRandom(i * 13 + 7) * 10;
      ctx.fillRect(Math.round(cx), Math.round(cy), Math.round(cw), Math.round(ch));
    }
  }

  // Rain: diagonal streaks (TSS 60+)
  if (tss >= 60) {
    const rainIntensity = Math.min(1, (tss - 60) / 80);
    const dropCount = Math.round(25 + rainIntensity * 50);

    ctx.fillStyle = `rgba(170, 190, 210, ${0.25 + rainIntensity * 0.35})`;

    for (let i = 0; i < dropCount; i++) {
      const baseX = seededRandom(i * 17 + 1) * CANVAS_WIDTH;
      const baseY = seededRandom(i * 17 + 2) * CANVAS_HEIGHT;
      const x = (baseX + frame * 1.2) % CANVAS_WIDTH;
      const y = (baseY + frame * 2.5) % CANVAS_HEIGHT;
      const len = 2 + Math.round(rainIntensity * 2);
      ctx.fillRect(Math.round(x), Math.round(y), 1, len);
    }

    ctx.fillStyle = `rgba(15, 20, 40, ${0.04 + rainIntensity * 0.08})`;
    ctx.fillRect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT);
  }

  // Storm: lightning (TSS 100+)
  if (tss >= 100) {
    const flashCycle = frame % 240;
    if (flashCycle < 3) {
      const flashAlpha = flashCycle === 0 ? 0.35 : flashCycle === 1 ? 0.15 : 0.04;
      ctx.fillStyle = `rgba(255, 255, 240, ${flashAlpha})`;
      ctx.fillRect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT);
    }
  }
}
