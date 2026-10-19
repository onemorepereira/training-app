import { describe, it, expect } from 'vitest';
import { rideLayout, MIN_POWER_PX, MIN_VALUE_PX, MIN_SMALL_VALUE_PX } from './rideLayout';

// Stage sizes from the design's height budget (docs/design/ui-polish/design.md, ADR-3).
const WIN_800 = { w: 704, h: 380 };
const WIN_800_NOTICE = { w: 704, h: 328 };
const WIN_1280 = { w: 1048, h: 580 };
const WIN_1920 = { w: 1688, h: 912 };
const FULL_1920 = { w: 1888, h: 912 };

describe('rideLayout', () => {
  it('800x600 window: wide layout meets the minimums with the secondary strip shown', () => {
    const l = rideLayout(WIN_800.w, WIN_800.h);
    expect(l.mode).toBe('wide');
    expect(l.showSecondary).toBe(true);
    expect(l.powerPx).toBeGreaterThanOrEqual(MIN_POWER_PX);
    expect(l.valuePx).toBeGreaterThanOrEqual(MIN_VALUE_PX);
    expect(l.smallValuePx).toBeGreaterThanOrEqual(MIN_SMALL_VALUE_PX);
  });

  it('800x600 with a notice: numbers still meet the minimums', () => {
    const l = rideLayout(WIN_800_NOTICE.w, WIN_800_NOTICE.h);
    expect(l.powerPx).toBeGreaterThanOrEqual(MIN_POWER_PX);
    expect(l.valuePx).toBeGreaterThanOrEqual(MIN_VALUE_PX);
  });

  it('when the strip would push primaries below their minimum, the strip hides first', () => {
    const l = rideLayout(704, 240);
    expect(l.showSecondary).toBe(false);
    expect(l.stripPx).toBe(0);
    expect(l.valuePx).toBeGreaterThan(rideLayout(704, 240 - 72).valuePx);
  });

  it('1280x800 window: numbers grow beyond the minimums', () => {
    const l = rideLayout(WIN_1280.w, WIN_1280.h);
    expect(l.valuePx).toBeGreaterThan(55);
    expect(l.powerPx).toBeGreaterThan(70);
  });

  it('1920x1080 window: power >= 112px and primaries >= 88px', () => {
    const l = rideLayout(WIN_1920.w, WIN_1920.h);
    expect(l.powerPx).toBeGreaterThanOrEqual(112);
    expect(l.valuePx).toBeGreaterThanOrEqual(88);
  });

  it('fullscreen at 1920x1080 is at least as large as windowed', () => {
    const win = rideLayout(WIN_1920.w, WIN_1920.h);
    const full = rideLayout(FULL_1920.w, FULL_1920.h);
    expect(full.valuePx).toBeGreaterThanOrEqual(win.valuePx);
    expect(full.powerPx).toBeGreaterThanOrEqual(win.powerPx);
  });

  it('numbers never shrink when the stage grows', () => {
    let prev = rideLayout(704, 380);
    for (const [w, h] of [[900, 480], [1048, 580], [1400, 760], [1688, 912], [1888, 912]]) {
      const l = rideLayout(w, h);
      expect(l.valuePx).toBeGreaterThanOrEqual(prev.valuePx);
      expect(l.powerPx).toBeGreaterThanOrEqual(prev.powerPx);
      prev = l;
    }
  });

  it('a tall, narrow stage (e.g. an 800x1000 window) stacks and still meets the minimums', () => {
    const l = rideLayout(704, 780);
    expect(l.mode).toBe('stacked');
    expect(l.powerPx).toBeGreaterThanOrEqual(MIN_POWER_PX);
    expect(l.valuePx).toBeGreaterThanOrEqual(MIN_VALUE_PX);
  });

  it('secondary values are never below 28px, so their 0.45em unit stays >= 12px', () => {
    for (const [w, h] of [[704, 380], [1048, 580], [1688, 912]]) {
      expect(rideLayout(w, h).smallValuePx * 0.45).toBeGreaterThanOrEqual(12);
    }
  });

  it('a zero-size stage (hidden, first paint) returns the minimums, never NaN', () => {
    const l = rideLayout(0, 0);
    expect(l.valuePx).toBe(MIN_VALUE_PX);
    expect(l.powerPx).toBe(MIN_POWER_PX);
    expect(Number.isNaN(l.smallValuePx)).toBe(false);
  });
});
