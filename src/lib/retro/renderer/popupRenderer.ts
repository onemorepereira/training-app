/**
 * Pop-up renderer — milestone/PR/streak text pop-ups with rise+fade animation.
 */

import { CANVAS_WIDTH, CANVAS_HEIGHT, type PopupMessage } from '../types.js';
import { drawText, measureText } from '../pixelFont.js';

/** How far pop-ups rise in pixels over their lifetime */
const RISE_DISTANCE = 20;

/**
 * Update and render all active pop-ups.
 * Modifies the popups array in place (decrements timeLeft, removes expired).
 */
export function renderPopups(
  ctx: CanvasRenderingContext2D,
  popups: PopupMessage[],
  dt: number,
): void {
  // Update timers and remove expired
  for (let i = popups.length - 1; i >= 0; i--) {
    popups[i].timeLeft -= dt;
    if (popups[i].timeLeft <= 0) {
      popups.splice(i, 1);
    }
  }

  // Render active pop-ups stacked from center
  const centerX = CANVAS_WIDTH / 2;
  const baseY = CANVAS_HEIGHT * 0.35;

  for (let i = 0; i < popups.length; i++) {
    const popup = popups[i];
    const progress = 1 - popup.timeLeft / popup.totalDuration; // 0→1

    // Rise: moves up over time
    const riseY = progress * RISE_DISTANCE;
    // Fade: full opacity first 70%, then fade out
    const alpha = progress < 0.7 ? 1 : 1 - (progress - 0.7) / 0.3;

    // Scale: start at 2x, settle to 1x in first 20%
    const scale = progress < 0.2 ? 2 : 1;

    const textW = measureText(popup.text, scale);
    const x = Math.round(centerX - textW / 2);
    const y = Math.round(baseY - riseY + i * 14);

    ctx.globalAlpha = alpha;

    // Shadow for readability
    drawText(ctx, popup.text, x + 1, y + 1, '#000000', scale);
    drawText(ctx, popup.text, x, y, popup.color, scale);
  }

  ctx.globalAlpha = 1;
}
