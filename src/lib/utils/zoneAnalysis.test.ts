import { describe, it, expect } from 'vitest';
import { computeDecoupling, computeTimeInZone, computeTimeToZone } from './zoneAnalysis';

describe('computeDecoupling', () => {
  it('constant HR/power ratio yields ~0% decoupling', () => {
    // 40 points: constant 150bpm / 200W throughout
    const ts = Array.from({ length: 40 }, () => ({ power: 200, heart_rate: 150 }));
    const d = computeDecoupling(ts);
    expect(d).not.toBeNull();
    expect(Math.abs(d!)).toBeLessThan(0.01);
  });

  it('HR drifts up 10% in second half yields ~10% decoupling', () => {
    // First 20: 150bpm / 200W → ratio = 0.75
    // Second 20: 165bpm / 200W → ratio = 0.825
    // decoupling = (0.825 - 0.75) / 0.75 * 100 = 10%
    const first = Array.from({ length: 20 }, () => ({ power: 200, heart_rate: 150 }));
    const second = Array.from({ length: 20 }, () => ({ power: 200, heart_rate: 165 }));
    const d = computeDecoupling([...first, ...second]);
    expect(d).not.toBeNull();
    expect(d!).toBeCloseTo(10.0, 0);
  });

  it('returns null with fewer than 20 paired points', () => {
    const ts = Array.from({ length: 15 }, () => ({ power: 200, heart_rate: 150 }));
    expect(computeDecoupling(ts)).toBeNull();
  });

  it('skips null values when counting pairs', () => {
    // 25 total but 10 have null power → only 15 paired → null
    const ts = [
      ...Array.from({ length: 10 }, () => ({ power: null as number | null, heart_rate: 150 })),
      ...Array.from({ length: 15 }, () => ({ power: 200 as number | null, heart_rate: 150 })),
    ];
    expect(computeDecoupling(ts)).toBeNull();
  });

  it('skips zero-power points', () => {
    // 25 total but 10 have power=0 → only 15 paired → null
    const ts = [
      ...Array.from({ length: 10 }, () => ({ power: 0, heart_rate: 150 })),
      ...Array.from({ length: 15 }, () => ({ power: 200, heart_rate: 150 })),
    ];
    expect(computeDecoupling(ts)).toBeNull();
  });
});

describe('computeTimeInZone', () => {
  it('counts points in each bucket correctly', () => {
    // Zone: 100-200. 10 points at 1s intervals.
    // 3 below (50, 80, 90), 4 in zone (100, 150, 175, 200), 3 above (210, 250, 300)
    const ts = [50, 80, 90, 100, 150, 175, 200, 210, 250, 300].map((v) => ({ value: v }));
    const result = computeTimeInZone(ts, 100, 200, 1);
    expect(result.belowSecs).toBe(3);
    expect(result.inZoneSecs).toBe(4);
    expect(result.aboveSecs).toBe(3);
  });

  it('respects intervalSecs multiplier', () => {
    const ts = [50, 150, 250].map((v) => ({ value: v }));
    const result = computeTimeInZone(ts, 100, 200, 5);
    expect(result.belowSecs).toBe(5);
    expect(result.inZoneSecs).toBe(5);
    expect(result.aboveSecs).toBe(5);
  });

  it('skips null values', () => {
    const ts = [{ value: null }, { value: 150 }, { value: null }];
    const result = computeTimeInZone(ts, 100, 200, 1);
    expect(result.inZoneSecs).toBe(1);
    expect(result.belowSecs).toBe(0);
    expect(result.aboveSecs).toBe(0);
  });
});

describe('computeTimeToZone', () => {
  it('returns seconds until first in-zone reading', () => {
    // First 3 out of zone, then in zone
    const ts = [50, 60, 70, 150, 160].map((v) => ({ value: v }));
    expect(computeTimeToZone(ts, 100, 200, 1)).toBe(3);
  });

  it('returns 0 when first reading is in zone', () => {
    const ts = [150, 160, 170].map((v) => ({ value: v }));
    expect(computeTimeToZone(ts, 100, 200, 1)).toBe(0);
  });

  it('returns null when never reaching zone', () => {
    const ts = [50, 60, 70].map((v) => ({ value: v }));
    expect(computeTimeToZone(ts, 100, 200, 1)).toBeNull();
  });

  it('respects intervalSecs', () => {
    const ts = [50, 60, 150].map((v) => ({ value: v }));
    expect(computeTimeToZone(ts, 100, 200, 5)).toBe(10);
  });

  it('skips null values (does not count as in-zone)', () => {
    const ts = [{ value: null }, { value: null }, { value: 150 }];
    expect(computeTimeToZone(ts, 100, 200, 1)).toBe(2);
  });
});
