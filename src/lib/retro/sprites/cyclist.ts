/**
 * Cyclist sprite data: 4 pedal frames at 24x32.
 * Rendered at 2x scale. Side-view racing cyclist on a road bike.
 *
 * Generated programmatically with layered rendering:
 *   Layer 1 (back): Wheels — circles for tires/rims, cross spokes
 *   Layer 2: Frame — top tube, seat tube, down tube, stays, fork
 *   Layer 3: Legs — thick lines from hip to pedal positions
 *   Layer 4 (front): Rider body — helmet, jersey polygon, arms
 *
 * Character key:
 *   . = transparent
 *   H = hair / S = skin / s = skin shadow
 *   J = jersey / j = jersey dark (also shorts)
 *   B = bike frame (red) / b = bike dark
 *   W = wheel rim / w = tire
 *   A = accent (helmet, shoes)
 *   R = wheel spoke highlight
 */

import type { EffortLevel } from '../types.js';
import { ZONE_PALETTES, type ZonePalette } from './palette.js';

export const SPRITE_W = 24;
export const SPRITE_H = 32;
export const SPRITE_SCALE = 2;

type SpriteData = string[];

/** Pad each row to exactly SPRITE_W with transparent pixels. */
function padFrame(rows: string[]): string[] {
  return rows.map(r => r.padEnd(SPRITE_W, '.'));
}

const FRAMES: SpriteData[] = [
  // Frame 0: pedal angle 0° (right pedal forward)
  padFrame([
    '........................', // 0
    '........................', // 1
    '........................', // 2
    '..........AAAA..........', // 3
    '........HAAAAAA.........', // 4
    '...........SSS..........', // 5
    '........jjjJJJJ.........', // 6
    '........jjjJJJJJ........', // 7
    '........jjjJJJJJJ.......', // 8
    '......bbjjjJJJJ..ss.....', // 9
    '.......BBBBBBBBBBB......', // 10
    '.......BjjBBBBBBBB......', // 11
    '........j.j.....Bb......', // 12
    '.......bjB.j...B.b......', // 13
    '...www.b.j.jj.B..bwww...', // 14
    '..wWWWb..jB.jj...wbWWw..', // 15
    '.wWWWWbwbAbbbjA.wWbWWWw.', // 16
    '.wRWbbbb........wRbbWRw.', // 17
    '.wWWWWWw........wWWWWWw.', // 18
    '..wWWWw..........wWWWw..', // 19
    '...www............www...', // 20
    '........................', // 21
    '........................', // 22
    '........................', // 23
    '........................', // 24
    '........................', // 25
    '........................', // 26
    '........................', // 27
    '........................', // 28
    '........................', // 29
    '........................', // 30
    '........................', // 31
  ]),
  // Frame 1: pedal angle 90° (right pedal down)
  padFrame([
    '........................', // 0
    '........................', // 1
    '........................', // 2
    '..........AAAA..........', // 3
    '........HAAAAAA.........', // 4
    '...........SSS..........', // 5
    '........jjjJJJJ.........', // 6
    '........jjjJJJJJ........', // 7
    '........jjjJJJJJJ.......', // 8
    '......bbjjjJJJJ..ss.....', // 9
    '.......BBBBBBBBBBB......', // 10
    '.......BjjBBBBBBBB......', // 11
    '........bj......Bb......', // 12
    '.......b.jj....B.b......', // 13
    '...www.b.BjA..B..bwww...', // 14
    '..wWWWb...jb.B...wbWWw..', // 15
    '.wWWWWbwbbjbB...wWbWWWw.', // 16
    '.wWWbbbb..jb....wWbbWWw.', // 17
    '.wWWWWWw...j....wWWWWWw.', // 18
    '..wWWWw....A.....wWWWw..', // 19
    '...www............www...', // 20
    '........................', // 21
    '........................', // 22
    '........................', // 23
    '........................', // 24
    '........................', // 25
    '........................', // 26
    '........................', // 27
    '........................', // 28
    '........................', // 29
    '........................', // 30
    '........................', // 31
  ]),
  // Frame 2: pedal angle 180° (right pedal back)
  padFrame([
    '........................', // 0
    '........................', // 1
    '........................', // 2
    '..........AAAA..........', // 3
    '........HAAAAAA.........', // 4
    '...........SSS..........', // 5
    '........jjjJJJJ.........', // 6
    '........jjjJJJJJ........', // 7
    '........jjjJJJJJJ.......', // 8
    '......bbjjjJJJJ..ss.....', // 9
    '.......BBBBBBBBBBB......', // 10
    '.......BjjBBBBBBBB......', // 11
    '........bj......Bb......', // 12
    '.......b.jjj...B.b......', // 13
    '...www.bjj..j.B..bwww...', // 14
    '..wWWWb.jjB..j...wbWWw..', // 15
    '.wWWWWbwjAbbbbA.wWbWWWw.', // 16
    '.wRWbbbb........wRbbWRw.', // 17
    '.wWWWWWw........wWWWWWw.', // 18
    '..wWWWw..........wWWWw..', // 19
    '...www............www...', // 20
    '........................', // 21
    '........................', // 22
    '........................', // 23
    '........................', // 24
    '........................', // 25
    '........................', // 26
    '........................', // 27
    '........................', // 28
    '........................', // 29
    '........................', // 30
    '........................', // 31
  ]),
  // Frame 3: pedal angle 270° (right pedal up)
  padFrame([
    '........................', // 0
    '........................', // 1
    '........................', // 2
    '..........AAAA..........', // 3
    '........HAAAAAA.........', // 4
    '...........SSS..........', // 5
    '........jjjJJJJ.........', // 6
    '........jjjJJJJJ........', // 7
    '........jjjJJJJJJ.......', // 8
    '......bbjjjJJJJ..ss.....', // 9
    '.......BBBBBBBBBBB......', // 10
    '.......BjjBBBBBBBB......', // 11
    '........j.j.....Bb......', // 12
    '.......b.jj....B.b......', // 13
    '...www.b.j.A..B..bwww...', // 14
    '..wWWWb..jBb.B...wbWWw..', // 15
    '.wWWWWbwbbjbB...wWbWWWw.', // 16
    '.wWWbbbb..jb....wWbbWWw.', // 17
    '.wWWWWWw...j....wWWWWWw.', // 18
    '..wWWWw....A.....wWWWw..', // 19
    '...www............www...', // 20
    '........................', // 21
    '........................', // 22
    '........................', // 23
    '........................', // 24
    '........................', // 25
    '........................', // 26
    '........................', // 27
    '........................', // 28
    '........................', // 29
    '........................', // 30
    '........................', // 31
  ]),
];

/**
 * Effort-specific sprite adjustments.
 * Higher effort = more forward lean.
 */
export interface EffortAdjustment {
  torsoShift: number;
  extraBob: number;
}

export const EFFORT_ADJUSTMENTS: Record<EffortLevel, EffortAdjustment> = {
  idle:      { torsoShift: 0, extraBob: 0 },
  easy:      { torsoShift: 0, extraBob: 0 },
  tempo:     { torsoShift: -1, extraBob: 0 },
  threshold: { torsoShift: -1, extraBob: 0.5 },
  vo2:       { torsoShift: -2, extraBob: 1 },
  anaerobic: { torsoShift: -2, extraBob: 1.5 },
  sprint:    { torsoShift: -3, extraBob: 2 },
};

/** Map sprite character to palette color. Returns null for transparent. */
function charToColor(ch: string, palette: ZonePalette): string | null {
  switch (ch) {
    case '.': return null;
    case 'H': return palette.hair;
    case 'S': return palette.skin;
    case 's': return palette.skinDark;
    case 'J': return palette.jersey;
    case 'j': return palette.jerseyDark;
    case 'B': return palette.bike;
    case 'b': return palette.bikeDark;
    case 'W': return palette.wheel;
    case 'w': return palette.outline;   // tire = dark outline color
    case 'A': return palette.accent;
    case 'R': return '#bbbbbb';          // spoke highlight
    default: return null;
  }
}

/**
 * Draw a cyclist sprite at SPRITE_SCALE.
 */
export function drawCyclist(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  frame: number,
  zone: number,
  effort: EffortLevel,
): void {
  const frameIdx = Math.abs(frame) % 4;
  const spriteRows = FRAMES[frameIdx];
  const palette = ZONE_PALETTES[Math.min(Math.max(zone, 0), 7)];
  const adj = EFFORT_ADJUSTMENTS[effort];
  const s = SPRITE_SCALE;

  for (let row = 0; row < SPRITE_H; row++) {
    const line = spriteRows[row];
    const shift = row <= 9 ? adj.torsoShift : 0;

    for (let col = 0; col < SPRITE_W; col++) {
      const ch = line[col];
      const color = charToColor(ch, palette);
      if (color) {
        ctx.fillStyle = color;
        ctx.fillRect(
          Math.round(x + (col + shift) * s),
          Math.round(y + row * s),
          s,
          s,
        );
      }
    }
  }
}
