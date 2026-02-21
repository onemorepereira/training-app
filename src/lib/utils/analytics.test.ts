import { describe, it, expect } from 'vitest';
import type { SessionSummary } from '$lib/tauri';
import {
  computePmc,
  computeRampRate,
  computeWeeklyTrends,
  extractFtpProgression,
  toDateKey,
  weekMonday,
} from './analytics';
import type { PmcDay } from './analytics';

/** Helper to build a minimal SessionSummary for testing. */
function session(
  overrides: Partial<SessionSummary> & { start_time: string },
): SessionSummary {
  return {
    id: 'test',
    duration_secs: 3600,
    ftp: null,
    avg_power: null,
    max_power: null,
    normalized_power: null,
    tss: null,
    intensity_factor: null,
    avg_hr: null,
    max_hr: null,
    avg_cadence: null,
    avg_speed: null,
    ...overrides,
  };
}

function assertApprox(actual: number, expected: number, epsilon: number, msg?: string) {
  expect(
    Math.abs(actual - expected),
    `${msg ?? ''} expected ≈${expected}, got ${actual}`,
  ).toBeLessThanOrEqual(epsilon);
}

describe('toDateKey', () => {
  it('extracts date from ISO datetime', () => {
    expect(toDateKey('2025-03-15T14:30:00Z')).toBe('2025-03-15');
  });

  it('extracts date from ISO with timezone offset', () => {
    expect(toDateKey('2025-03-15T14:30:00+02:00')).toBe('2025-03-15');
  });
});

describe('weekMonday', () => {
  it('returns same day for a Monday', () => {
    // 2025-01-06 is a Monday
    const result = weekMonday(new Date('2025-01-06T12:00:00Z'));
    expect(toDateKey(result.toISOString())).toBe('2025-01-06');
  });

  it('returns previous Monday for a Wednesday', () => {
    // 2025-01-08 is a Wednesday → Monday is 2025-01-06
    const result = weekMonday(new Date('2025-01-08T12:00:00Z'));
    expect(toDateKey(result.toISOString())).toBe('2025-01-06');
  });

  it('returns previous Monday for a Saturday', () => {
    // 2025-01-11 is a Saturday → Monday is 2025-01-06
    const result = weekMonday(new Date('2025-01-11T12:00:00Z'));
    expect(toDateKey(result.toISOString())).toBe('2025-01-06');
  });

  it('returns previous Monday for a Sunday', () => {
    // 2025-01-12 is a Sunday → Monday is 2025-01-06
    const result = weekMonday(new Date('2025-01-12T12:00:00Z'));
    expect(toDateKey(result.toISOString())).toBe('2025-01-06');
  });
});

describe('computePmc', () => {
  it('returns empty for no sessions', () => {
    expect(computePmc([])).toEqual([]);
  });

  it('computes single-session PMC correctly', () => {
    // TSS=100 on day 0
    // CTL = 0 + (100 - 0) / 42 = 100/42 ≈ 2.381
    // ATL = 0 + (100 - 0) / 7  = 100/7  ≈ 14.286
    // TSB = CTL - ATL ≈ 2.381 - 14.286 ≈ -11.905
    const sessions = [session({ start_time: new Date().toISOString(), tss: 100 })];
    const pmc = computePmc(sessions);
    const last = pmc[pmc.length - 1];

    assertApprox(last.ctl, 100 / 42, 0.01, 'CTL');
    assertApprox(last.atl, 100 / 7, 0.01, 'ATL');
    assertApprox(last.tsb, 100 / 42 - 100 / 7, 0.01, 'TSB');
  });

  it('sums TSS from two sessions on the same day', () => {
    const today = new Date().toISOString();
    const sessions = [
      session({ id: 'a', start_time: today, tss: 60 }),
      session({ id: 'b', start_time: today, tss: 40 }),
    ];
    const pmc = computePmc(sessions);
    const last = pmc[pmc.length - 1];

    // Combined TSS = 100, same result as single 100-TSS session
    assertApprox(last.ctl, 100 / 42, 0.01, 'CTL');
    assertApprox(last.atl, 100 / 7, 0.01, 'ATL');
  });

  it('all-null TSS keeps CTL/ATL/TSB at zero', () => {
    const sessions = [
      session({ start_time: new Date().toISOString(), tss: null }),
    ];
    const pmc = computePmc(sessions);
    const last = pmc[pmc.length - 1];

    expect(last.ctl).toBe(0);
    expect(last.atl).toBe(0);
    expect(last.tsb).toBe(0);
  });

  it('multi-day PMC accumulates correctly', () => {
    // Day 0: TSS=100 → CTL = 100/42, ATL = 100/7
    // Day 1: TSS=0   → CTL = CTL_0 + (0 - CTL_0)/42 = CTL_0 * (41/42)
    //                   ATL = ATL_0 + (0 - ATL_0)/7  = ATL_0 * (6/7)
    const yesterday = new Date();
    yesterday.setUTCDate(yesterday.getUTCDate() - 1);
    const sessions = [
      session({ start_time: yesterday.toISOString(), tss: 100 }),
    ];
    const pmc = computePmc(sessions);

    // Should have 2 days
    expect(pmc.length).toBe(2);

    const ctl0 = 100 / 42;
    const atl0 = 100 / 7;
    const ctl1 = ctl0 * (41 / 42);
    const atl1 = atl0 * (6 / 7);

    assertApprox(pmc[0].ctl, ctl0, 0.01, 'day 0 CTL');
    assertApprox(pmc[0].atl, atl0, 0.01, 'day 0 ATL');
    assertApprox(pmc[1].ctl, ctl1, 0.01, 'day 1 CTL');
    assertApprox(pmc[1].atl, atl1, 0.01, 'day 1 ATL');
    assertApprox(pmc[1].tsb, ctl1 - atl1, 0.01, 'day 1 TSB');
  });
});

describe('computeWeeklyTrends', () => {
  it('returns empty for no sessions', () => {
    expect(computeWeeklyTrends([])).toEqual([]);
  });

  it('groups sessions into correct week buckets', () => {
    // Week 1: Mon 2025-01-06
    //   Session A: TSS=50, power=200, hr=140, duration=3600
    //   Session B: TSS=70, power=220, hr=null, duration=5400
    // Week 2: Mon 2025-01-13
    //   Session C: TSS=80, power=null, hr=150, duration=7200
    const sessions = [
      session({
        id: 'a',
        start_time: '2025-01-07T10:00:00Z', // Tuesday wk1
        tss: 50,
        avg_power: 200,
        avg_hr: 140,
        duration_secs: 3600,
      }),
      session({
        id: 'b',
        start_time: '2025-01-09T10:00:00Z', // Thursday wk1
        tss: 70,
        avg_power: 220,
        avg_hr: null,
        duration_secs: 5400,
      }),
      session({
        id: 'c',
        start_time: '2025-01-14T10:00:00Z', // Tuesday wk2
        tss: 80,
        avg_power: null,
        avg_hr: 150,
        duration_secs: 7200,
      }),
    ];

    const weeks = computeWeeklyTrends(sessions);
    expect(weeks.length).toBe(2);

    // Week 1
    expect(weeks[0].weekStart).toBe('2025-01-06');
    expect(weeks[0].totalTss).toBe(120); // 50+70
    assertApprox(weeks[0].avgPower!, 210, 0.1, 'wk1 avgPower'); // (200+220)/2
    assertApprox(weeks[0].avgHr!, 140, 0.1, 'wk1 avgHr'); // only 1 non-null
    expect(weeks[0].sessionCount).toBe(2);
    expect(weeks[0].totalDurationSecs).toBe(9000); // 3600+5400

    // Week 2
    expect(weeks[1].weekStart).toBe('2025-01-13');
    expect(weeks[1].totalTss).toBe(80);
    expect(weeks[1].avgPower).toBeNull();
    assertApprox(weeks[1].avgHr!, 150, 0.1, 'wk2 avgHr');
    expect(weeks[1].sessionCount).toBe(1);
    expect(weeks[1].totalDurationSecs).toBe(7200);
  });

  it('excludes null metrics from averages (not counted as 0)', () => {
    const sessions = [
      session({
        id: 'a',
        start_time: '2025-01-07T10:00:00Z',
        avg_power: 200,
        avg_hr: null,
      }),
      session({
        id: 'b',
        start_time: '2025-01-08T10:00:00Z',
        avg_power: null,
        avg_hr: null,
      }),
    ];

    const weeks = computeWeeklyTrends(sessions);
    // avg_power should be 200 (only one valid), not 100 (200+0)/2
    assertApprox(weeks[0].avgPower!, 200, 0.1, 'power average excludes nulls');
    expect(weeks[0].avgHr).toBeNull();
  });
});

describe('extractFtpProgression', () => {
  it('returns empty for no sessions', () => {
    expect(extractFtpProgression([])).toEqual([]);
  });

  it('returns empty when all FTP are null', () => {
    const sessions = [
      session({ start_time: '2025-01-01T00:00:00Z', ftp: null }),
    ];
    expect(extractFtpProgression(sessions)).toEqual([]);
  });

  it('emits change points only', () => {
    const sessions = [
      session({ id: 'a', start_time: '2025-01-01T00:00:00Z', ftp: 200 }),
      session({ id: 'b', start_time: '2025-02-01T00:00:00Z', ftp: 200 }),
      session({ id: 'c', start_time: '2025-03-01T00:00:00Z', ftp: 220 }),
      session({ id: 'd', start_time: '2025-04-01T00:00:00Z', ftp: 220 }),
      session({ id: 'e', start_time: '2025-05-01T00:00:00Z', ftp: 210 }),
    ];

    const points = extractFtpProgression(sessions);
    // Change points: 200 (first), 220 (change), 210 (change)
    // Last session (210) is already a change point, so 3 total
    expect(points).toEqual([
      { date: '2025-01-01', ftp: 200 },
      { date: '2025-03-01', ftp: 220 },
      { date: '2025-05-01', ftp: 210 },
    ]);
  });

  it('always includes last session even if FTP unchanged', () => {
    const sessions = [
      session({ id: 'a', start_time: '2025-01-01T00:00:00Z', ftp: 200 }),
      session({ id: 'b', start_time: '2025-06-01T00:00:00Z', ftp: 200 }),
    ];

    const points = extractFtpProgression(sessions);
    // First is a change point, last has same FTP but different date → should be emitted
    expect(points).toEqual([
      { date: '2025-01-01', ftp: 200 },
      { date: '2025-06-01', ftp: 200 },
    ]);
  });
});

describe('computeRampRate', () => {
  function makePmcDays(ctlValues: number[]): PmcDay[] {
    return ctlValues.map((ctl, i) => ({
      date: `2025-01-${String(i + 1).padStart(2, '0')}`,
      tss: 0,
      ctl,
      atl: 0,
      tsb: 0,
    }));
  }

  it('returns null with fewer than 8 PMC days', () => {
    expect(computeRampRate(makePmcDays([10, 11, 12, 13, 14, 15, 16]))).toBeNull();
  });

  it('returns zero ramp for flat CTL', () => {
    const result = computeRampRate(makePmcDays([30, 30, 30, 30, 30, 30, 30, 30]));
    expect(result).not.toBeNull();
    assertApprox(result!.current, 0, 0.01, 'flat CTL');
    expect(result!.classification).toBe('maintenance');
  });

  it('classifies moderate build correctly', () => {
    // CTL goes from 20 to 24 over 7 days → delta = 4.0
    const result = computeRampRate(makePmcDays([20, 20.5, 21, 21.5, 22, 22.5, 23, 24]));
    expect(result).not.toBeNull();
    assertApprox(result!.current, 4.0, 0.01, 'moderate delta');
    expect(result!.classification).toBe('moderate');
  });

  it('classifies excessive build correctly', () => {
    // CTL goes from 20 to 30 over 7 days → delta = 10.0
    const result = computeRampRate(makePmcDays([20, 21, 22, 23, 24, 25, 26, 30]));
    expect(result).not.toBeNull();
    assertApprox(result!.current, 10.0, 0.01, 'excessive delta');
    expect(result!.classification).toBe('excessive');
  });

  it('classifies recovery correctly', () => {
    // CTL goes from 40 to 35 over 7 days → delta = -5.0
    const result = computeRampRate(makePmcDays([40, 39, 38, 37, 36, 36, 35, 35]));
    expect(result).not.toBeNull();
    assertApprox(result!.current, -5.0, 0.01, 'recovery delta');
    expect(result!.classification).toBe('recovery');
  });
});
