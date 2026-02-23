/**
 * NPC renderer — draws NPC cyclists in a side-scrolling view.
 * NPCs are positioned horizontally relative to the player.
 * All NPCs render at 2× scale on the same road surface (matching player sprite scale).
 */

import { CANVAS_WIDTH } from '../types.js';
import type { NpcState } from '../types.js';
import { drawNpc, NPC_W, NPC_H } from '../sprites/npc.js';
import { getRoadY, getPlayerX } from './roadRenderer.js';

/** NPC draw scale — matches player SPRITE_SCALE */
const NPC_SCALE = 2;
const DRAWN_W = NPC_W * NPC_SCALE;
const DRAWN_H = NPC_H * NPC_SCALE;

/**
 * Render all NPCs sorted by distance (furthest first so closer ones overlap).
 */
export function renderNpcs(ctx: CanvasRenderingContext2D, npcs: NpcState[]): void {
  const playerX = getPlayerX();
  const roadY = getRoadY();

  // Sort by relativePos descending — furthest ahead drawn first (behind closer NPCs)
  const sorted = [...npcs].sort((a, b) => b.relativePos - a.relativePos);

  for (const npc of sorted) {
    const screenX = Math.round(playerX + npc.relativePos);

    // Cull NPCs off-screen
    if (screenX + DRAWN_W < 0 || screenX - DRAWN_W > CANVAS_WIDTH) continue;

    // Small lane offset per NPC for visual variety (±2px)
    const laneOffset = ((npc.id * 7) % 5) - 2;
    const x = screenX - Math.round(DRAWN_W / 2);
    const y = Math.round(roadY - DRAWN_H + laneOffset);

    const pedalFrame = Math.floor(npc.pedalAngle / Math.PI) % 2;
    drawNpc(ctx, x, y, pedalFrame, npc.id, NPC_SCALE);
  }
}
