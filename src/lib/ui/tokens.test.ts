import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { contrastRatio, over, parseColor } from './contrast';

const css = readFileSync(fileURLToPath(new URL('../../app.css', import.meta.url)), 'utf8');
const tokens = Object.fromEntries([...css.matchAll(/--([\w-]+):\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]));
const tok = (name: string) => {
  const v = tokens[name];
  if (!v) throw new Error(`token --${name} missing from app.css`);
  return v;
};

const SURFACES = ['bg-body', 'bg-surface', 'bg-elevated', 'bg-hover'];
const PANELS = ['panel-time', 'panel-effort', 'panel-power', 'panel-hr', 'panel-move'];
const TEXT = ['text-primary', 'text-secondary', 'text-muted', 'danger-text', 'accent', 'success', 'warning', 'info'];

/** Every background text can sit on: the opaque surfaces plus tinted panels composited over the body. */
const backgrounds = [
  ...SURFACES.map((s) => ({ name: s, color: parseColor(tok(s)) })),
  ...PANELS.map((p) => ({ name: `${p} over bg-body`, color: over(parseColor(tok(p)), parseColor(tok('bg-body'))) })),
];

describe('design tokens', () => {
  for (const t of TEXT) {
    it(`--${t} is >= 4.5:1 on every surface and panel`, () => {
      for (const bg of backgrounds) {
        const r = contrastRatio(tok(t), bg.color);
        expect(r, `--${t} on ${bg.name} = ${r.toFixed(2)}`).toBeGreaterThanOrEqual(4.5);
      }
    });
  }

  it('--text-faint (disabled/decorative only) is >= 3:1 on every surface', () => {
    for (const bg of backgrounds) {
      const r = contrastRatio(tok('text-faint'), bg.color);
      expect(r, `on ${bg.name} = ${r.toFixed(2)}`).toBeGreaterThanOrEqual(3);
    }
  });

  it('white button text is >= 4.5:1 on every strong fill', () => {
    for (const fill of ['accent-strong', 'success-strong', 'danger-strong']) {
      const r = contrastRatio('#ffffff', tok(fill));
      expect(r, `white on --${fill} = ${r.toFixed(2)}`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it('the smallest type step is 12px', () => {
    expect(tok('text-xs')).toBe('0.75rem');
  });
});

describe('contrastRatio', () => {
  it('black on white is 21:1', () => {
    expect(contrastRatio('#000000', '#ffffff')).toBeCloseTo(21, 2);
  });

  it('identical colours are 1:1', () => {
    expect(contrastRatio('#8c8ca6', '#8c8ca6')).toBeCloseTo(1, 5);
  });

  it('a fully transparent panel composites to the background itself', () => {
    const bg = parseColor('#0f0f1a');
    expect(over(parseColor('rgba(255, 0, 0, 0)'), bg)).toEqual(bg);
  });
});
