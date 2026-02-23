/**
 * NPC cyclist sprite — simpler than the player sprite.
 * 16x24px, 2 pedal frames, single color palette per NPC.
 *
 * No heavy outlines — uses the same clean style as the player sprite.
 * Bike is grey (not red like player) to visually distinguish NPCs.
 */

const NPC_W = 16;
const NPC_H = 24;

/**
 * NPC color palettes — varied jersey colors.
 * Index chosen by npc.id % NPC_COLORS.length
 */
const NPC_COLORS = [
  { jersey: '#ee7733', dark: '#cc5511' },
  { jersey: '#33aaee', dark: '#1188cc' },
  { jersey: '#44cc44', dark: '#22aa22' },
  { jersey: '#ee44aa', dark: '#cc2288' },
  { jersey: '#aaaa44', dark: '#888822' },
];

// Shared NPC colors
const NPC_SKIN = '#e8c8a0';
const NPC_SKIN_DARK = '#c8a880';
const NPC_HAIR = '#554433';
const NPC_HELMET = '#ddddee';
const NPC_BIKE = '#667788';
const NPC_BIKE_DARK = '#556677';
const NPC_WHEEL = '#99aabb';
const NPC_TIRE = '#444455';

/**
 * NPC frames — 2 pedal positions.
 * Character key same as player: A=helmet, H=hair, S/s=skin, J/j=jersey,
 * B/b=bike, W=wheel, w=tire, R=spoke
 */
const NPC_FRAMES: string[][] = [
  // Frame 0: pedals level
  [
    '................', // 0
    '.....AA.........', // 1  helmet
    '....AAHH........', // 2  helmet+hair
    '.....SSs........', // 3  face
    '....JJJJ........', // 4  jersey
    '....JjjJ........', // 5  jersey dark
    '.....SsB........', // 6  arm + handlebar
    '....Ss.BBB......', // 7  arm + top tube
    '....S...BbBB....', // 8  thigh + frame
    '...Ss....BbB....', // 9  leg + down tube
    '...sA.BBBBbB....', // 10 foot + chainstay
    '.wwW.bB...Bb....', // 11 rear wheel + stay
    'wWWWwB.....Bww..', // 12 rear wheel + front
    'wWRWw.....wWWWw.', // 13 wheels
    'wWWWw.....wWRWw.', // 14 wheels
    '.wWWw......wWWw.', // 15 wheels
    '..ww........ww..', // 16 tires
    '................', // 17
    '................', // 18
    '................', // 19
    '................', // 20
    '................', // 21
    '................', // 22
    '................', // 23
  ],
  // Frame 1: pedals vertical
  [
    '................', // 0
    '.....AA.........', // 1  helmet
    '....AAHH........', // 2  helmet+hair
    '.....SSs........', // 3  face
    '....JJJJ........', // 4  jersey
    '....JjjJ........', // 5  jersey dark
    '.....SsB........', // 6  arm + handlebar
    '....Ss.BBB......', // 7  arm + top tube
    '...SS...BbBB....', // 8  knee up + frame
    '...sSs...BbB....', // 9  shin up + tube
    '....sA.BBBBbB...', // 10 foot + chainstay
    '.wwW.bB...Bb....', // 11 rear wheel + stay
    'wWWWwB.....Bww..', // 12 rear wheel + front
    'wWWRw.....wWWWw.', // 13 wheels
    'wWWWw.....wWWRw.', // 14 wheels
    '.wWWw..S...wWWw.', // 15 wheels + leg down
    '..ww..sA....ww..', // 16 tires + foot
    '................', // 17
    '................', // 18
    '................', // 19
    '................', // 20
    '................', // 21
    '................', // 22
    '................', // 23
  ],
];

/** Map NPC sprite character to color */
function npcCharColor(ch: string, colorIdx: number): string | null {
  const c = NPC_COLORS[colorIdx % NPC_COLORS.length];
  switch (ch) {
    case '.': return null;
    case 'A': return NPC_HELMET;
    case 'H': return NPC_HAIR;
    case 'S': return NPC_SKIN;
    case 's': return NPC_SKIN_DARK;
    case 'J': return c.jersey;
    case 'j': return c.dark;
    case 'B': return NPC_BIKE;
    case 'b': return NPC_BIKE_DARK;
    case 'W': return NPC_WHEEL;
    case 'w': return NPC_TIRE;
    case 'R': return '#aabbcc';
    default: return null;
  }
}

/**
 * Draw an NPC cyclist sprite.
 * @param scale - Scale factor (0.5 for distant, 1 for close)
 */
export function drawNpc(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  frame: number,
  colorIdx: number,
  scale: number = 1,
): void {
  const frameIdx = Math.abs(frame) % 2;
  const rows = NPC_FRAMES[frameIdx];

  for (let row = 0; row < NPC_H; row++) {
    const line = rows[row];
    for (let col = 0; col < NPC_W; col++) {
      const color = npcCharColor(line[col], colorIdx);
      if (color) {
        ctx.fillStyle = color;
        if (scale === 1) {
          ctx.fillRect(Math.round(x + col), Math.round(y + row), 1, 1);
        } else {
          ctx.fillRect(
            Math.round(x + col * scale),
            Math.round(y + row * scale),
            Math.max(1, Math.round(scale)),
            Math.max(1, Math.round(scale)),
          );
        }
      }
    }
  }
}

export { NPC_W, NPC_H };
