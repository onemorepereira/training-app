import { describe, it, expect } from 'vitest';
import type { SessionSummary } from '$lib/tauri';
import {
  computePmc,
  computeRampRate,
  computeWeeklyTrends,
  extractFtpProgression,
  localDateKey,
  addDaysToKey,
  weekStartKey,
  currentWeekTss,
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

// Tests run with TZ=America/Chicago (vite.config.ts): CST = UTC−6, CDT = UTC−5.
// DST 2025: starts Sun 2025-03-09, ends Sun 2025-11-02.

describe('localDateKey', () => {
  it('local_date_key_uses_local_calendar_day', () => {
    // 2025-06-16T02:00Z = Sun 2025-06-15 21:00 CDT
    expect(localDateKey('2025-06-16T02:00:00Z')).toBe('2025-06-15');
  });

  it('local_date_key_midday_utc_is_same_day', () => {
    expect(localDateKey('2025-03-15T14:30:00Z')).toBe('2025-03-15');
  });

  it('local_date_key_accepts_offset_iso', () => {
    // 2025-03-15T01:30+02:00 = 2025-03-14T23:30Z = 2025-03-14 18:30 CDT
    expect(localDateKey('2025-03-15T01:30:00+02:00')).toBe('2025-03-14');
  });

  it('today_key_is_local', () => {
    // 03:00Z on Jan 6 is still Jan 5 21:00 CST
    expect(localDateKey(new Date('2025-01-06T03:00:00Z'))).toBe('2025-01-05');
  });
});

describe('addDaysToKey', () => {
  it('add_days_crosses_month_and_year', () => {
    expect(addDaysToKey('2024-12-31', 1)).toBe('2025-01-01');
    expect(addDaysToKey('2024-02-28', 1)).toBe('2024-02-29');
  });

  it('add_days_across_spring_forward_is_one_calendar_day', () => {
    expect(addDaysToKey('2025-03-08', 1)).toBe('2025-03-09');
    expect(addDaysToKey('2025-03-09', 1)).toBe('2025-03-10');
  });
});

describe('weekStartKey', () => {
  it('monday_is_its_own_week_start', () => {
    expect(weekStartKey('2025-01-06')).toBe('2025-01-06');
  });

  it('wednesday_maps_to_previous_monday', () => {
    expect(weekStartKey('2025-01-08')).toBe('2025-01-06');
  });

  it('sunday_maps_to_previous_monday', () => {
    expect(weekStartKey('2025-01-12')).toBe('2025-01-06');
  });
});

describe('computePmc', () => {
  it('returns empty for no sessions', () => {
    expect(computePmc([])).toEqual([]);
  });

  it('computes single-session PMC correctly', () => {
    // TSS=100 on day 0 → CTL = 100/42, ATL = 100/7, TSB = CTL − ATL
    const now = new Date('2025-06-15T18:00:00Z');
    const pmc = computePmc([session({ start_time: '2025-06-15T15:00:00Z', tss: 100 })], now);
    const last = pmc[pmc.length - 1];
    expect(pmc).toHaveLength(1);
    assertApprox(last.ctl, 100 / 42, 0.01, 'CTL');
    assertApprox(last.atl, 100 / 7, 0.01, 'ATL');
    assertApprox(last.tsb, 100 / 42 - 100 / 7, 0.01, 'TSB');
  });

  it('sums TSS from two sessions on the same day', () => {
    const now = new Date('2025-06-15T23:00:00Z');
    const pmc = computePmc([
      session({ id: 'a', start_time: '2025-06-15T13:00:00Z', tss: 60 }),
      session({ id: 'b', start_time: '2025-06-15T22:00:00Z', tss: 40 }),
    ], now);
    expect(pmc).toHaveLength(1);
    assertApprox(pmc[0].ctl, 100 / 42, 0.01, 'CTL');
    assertApprox(pmc[0].atl, 100 / 7, 0.01, 'ATL');
  });

  it('all-null TSS keeps CTL/ATL/TSB at zero', () => {
    const now = new Date('2025-06-15T18:00:00Z');
    const pmc = computePmc([session({ start_time: '2025-06-15T15:00:00Z', tss: null })], now);
    expect(pmc[0].ctl).toBe(0);
    expect(pmc[0].atl).toBe(0);
    expect(pmc[0].tsb).toBe(0);
  });

  it('multi-day PMC accumulates correctly', () => {
    // Day 0: TSS=100 → CTL0 = 100/42, ATL0 = 100/7; Day 1: TSS=0 → CTL0·41/42, ATL0·6/7
    const now = new Date('2025-06-16T18:00:00Z');
    const pmc = computePmc([session({ start_time: '2025-06-15T15:00:00Z', tss: 100 })], now);
    expect(pmc.map((d) => d.date)).toEqual(['2025-06-15', '2025-06-16']);
    const ctl1 = (100 / 42) * (41 / 42);
    const atl1 = (100 / 7) * (6 / 7);
    assertApprox(pmc[1].ctl, ctl1, 0.01, 'day 1 CTL');
    assertApprox(pmc[1].atl, atl1, 0.01, 'day 1 ATL');
    assertApprox(pmc[1].tsb, ctl1 - atl1, 0.01, 'day 1 TSB');
  });

  it('evening_ride_counts_on_local_day', () => {
    // Sun 2025-06-15 21:00 CDT = 2025-06-16T02:00Z
    const now = new Date('2025-06-16T03:00:00Z'); // still Sun 22:00 local
    const pmc = computePmc([session({ start_time: '2025-06-16T02:00:00Z', tss: 80 })], now);
    expect(pmc).toEqual([expect.objectContaining({ date: '2025-06-15', tss: 80 })]);
  });

  it('pmc_spring_forward_has_no_skipped_day', () => {
    const now = new Date('2025-03-10T18:00:00Z');
    const pmc = computePmc([session({ start_time: '2025-03-08T18:00:00Z', tss: 50 })], now);
    expect(pmc.map((d) => d.date)).toEqual(['2025-03-08', '2025-03-09', '2025-03-10']);
  });

  it('pmc_fall_back_has_no_duplicate_day', () => {
    const now = new Date('2025-11-03T18:00:00Z');
    const pmc = computePmc([session({ start_time: '2025-11-01T18:00:00Z', tss: 50 })], now);
    expect(pmc.map((d) => d.date)).toEqual(['2025-11-01', '2025-11-02', '2025-11-03']);
  });
});

describe('computeWeeklyTrends', () => {
  it('returns empty for no sessions', () => {
    expect(computeWeeklyTrends([])).toEqual([]);
  });

  it('groups sessions into correct week buckets', () => {
    const sessions = [
      session({ id: 'a', start_time: '2025-01-07T16:00:00Z', tss: 50, avg_power: 200, avg_hr: 140, duration_secs: 3600 }),
      session({ id: 'b', start_time: '2025-01-09T16:00:00Z', tss: 70, avg_power: 220, avg_hr: null, duration_secs: 5400 }),
      session({ id: 'c', start_time: '2025-01-14T16:00:00Z', tss: 80, avg_power: null, avg_hr: 150, duration_secs: 7200 }),
    ];
    const weeks = computeWeeklyTrends(sessions, new Date('2025-01-15T18:00:00Z'));
    expect(weeks.map((w) => w.weekStart)).toEqual(['2025-01-06', '2025-01-13']);
    expect(weeks[0].totalTss).toBe(120);
    // Duration-weighted: (200·3600 + 220·5400) / 9000 = 212.0
    assertApprox(weeks[0].avgPower!, 212, 0.1, 'wk1 avgPower');
    assertApprox(weeks[0].avgHr!, 140, 0.1, 'wk1 avgHr');
    expect(weeks[0].sessionCount).toBe(2);
    expect(weeks[0].totalDurationSecs).toBe(9000);
    expect(weeks[1].totalTss).toBe(80);
    expect(weeks[1].avgPower).toBeNull();
    assertApprox(weeks[1].avgHr!, 150, 0.1, 'wk2 avgHr');
  });

  it('sunday_2100_cdt_ride_counts_toward_that_sunday_week', () => {
    // Sun 2025-06-15 21:00 CDT = Mon 2025-06-16T02:00Z (UTC would say next week)
    const weeks = computeWeeklyTrends(
      [session({ start_time: '2025-06-16T02:00:00Z', tss: 90 })],
      new Date('2025-06-16T03:00:00Z'),
    );
    expect(weeks).toHaveLength(1);
    expect(weeks[0].weekStart).toBe('2025-06-09');
    expect(weeks[0].totalTss).toBe(90);
  });

  it('weekly_avg_power_is_duration_weighted', () => {
    // (200·3600 + 110·1800) / 5400 = 170.0 (unweighted mean would be 155)
    const weeks = computeWeeklyTrends([
      session({ id: 'a', start_time: '2025-06-10T16:00:00Z', avg_power: 200, duration_secs: 3600 }),
      session({ id: 'b', start_time: '2025-06-11T16:00:00Z', avg_power: 110, duration_secs: 1800 }),
    ], new Date('2025-06-12T16:00:00Z'));
    assertApprox(weeks[0].avgPower!, 170, 0.1, 'weighted power');
  });

  it('weekly_avg_hr_is_duration_weighted', () => {
    // (150·1200 + 130·3600) / 4800 = 135.0
    const weeks = computeWeeklyTrends([
      session({ id: 'a', start_time: '2025-06-10T16:00:00Z', avg_hr: 150, duration_secs: 1200 }),
      session({ id: 'b', start_time: '2025-06-11T16:00:00Z', avg_hr: 130, duration_secs: 3600 }),
    ], new Date('2025-06-12T16:00:00Z'));
    assertApprox(weeks[0].avgHr!, 135, 0.1, 'weighted hr');
  });

  it('excludes null metrics from averages (not counted as 0)', () => {
    const weeks = computeWeeklyTrends([
      session({ id: 'a', start_time: '2025-01-07T16:00:00Z', avg_power: 200, avg_hr: null }),
      session({ id: 'b', start_time: '2025-01-08T16:00:00Z', avg_power: null, avg_hr: null }),
    ], new Date('2025-01-08T18:00:00Z'));
    assertApprox(weeks[0].avgPower!, 200, 0.1, 'power average excludes nulls');
    expect(weeks[0].avgHr).toBeNull();
  });

  it('zero_duration_sessions_give_null_average', () => {
    const weeks = computeWeeklyTrends(
      [session({ start_time: '2025-06-10T16:00:00Z', avg_power: 250, avg_hr: 160, duration_secs: 0 })],
      new Date('2025-06-10T18:00:00Z'),
    );
    expect(weeks[0].avgPower).toBeNull();
    expect(weeks[0].avgHr).toBeNull();
  });

  it('empty_week_between_rides_has_zero_volume', () => {
    const weeks = computeWeeklyTrends([
      session({ id: 'a', start_time: '2025-06-03T16:00:00Z', tss: 40, avg_power: 180 }),
      session({ id: 'b', start_time: '2025-06-17T16:00:00Z', tss: 60, avg_power: 190 }),
    ], new Date('2025-06-18T16:00:00Z'));
    expect(weeks.map((w) => w.weekStart)).toEqual(['2025-06-02', '2025-06-09', '2025-06-16']);
    expect(weeks[1]).toEqual({
      weekStart: '2025-06-09',
      totalTss: 0,
      avgPower: null,
      avgHr: null,
      sessionCount: 0,
      totalDurationSecs: 0,
    });
  });

  it('weeks_filled_through_current_week', () => {
    const weeks = computeWeeklyTrends(
      [session({ start_time: '2025-06-03T16:00:00Z', tss: 40 })],
      new Date('2025-06-25T16:00:00Z'), // Wed of week 2025-06-23
    );
    expect(weeks.map((w) => w.weekStart)).toEqual(['2025-06-02', '2025-06-09', '2025-06-16', '2025-06-23']);
    expect(weeks[3].sessionCount).toBe(0);
  });
});

describe('currentWeekTss', () => {
  it('current_week_tss_uses_local_monday', () => {
    // Ride Mon 2025-06-16 08:00 CDT; "now" Sun 2025-06-22 22:00 CDT = 2025-06-23T03:00Z
    // (in UTC that is already Monday of the next week, which would report 0)
    const now = new Date('2025-06-23T03:00:00Z');
    const weeks = computeWeeklyTrends([session({ start_time: '2025-06-16T13:00:00Z', tss: 75 })], now);
    expect(currentWeekTss(weeks, now)).toBe(75);
  });

  it('current_week_tss_zero_when_no_ride_this_week', () => {
    const now = new Date('2025-06-25T16:00:00Z');
    const weeks = computeWeeklyTrends([session({ start_time: '2025-06-03T16:00:00Z', tss: 40 })], now);
    expect(currentWeekTss(weeks, now)).toBe(0);
  });

  it('current_week_tss_null_without_data', () => {
    expect(currentWeekTss([], new Date())).toBeNull();
  });
});

describe('extractFtpProgression', () => {
  it('returns empty for no sessions', () => {
    expect(extractFtpProgression([])).toEqual([]);
  });

  it('returns empty when all FTP are null', () => {
    const sessions = [
      session({ start_time: '2025-01-01T17:00:00Z', ftp: null }),
    ];
    expect(extractFtpProgression(sessions)).toEqual([]);
  });

  it('emits change points only', () => {
    const sessions = [
      session({ id: 'a', start_time: '2025-01-01T17:00:00Z', ftp: 200 }),
      session({ id: 'b', start_time: '2025-02-01T17:00:00Z', ftp: 200 }),
      session({ id: 'c', start_time: '2025-03-01T17:00:00Z', ftp: 220 }),
      session({ id: 'd', start_time: '2025-04-01T17:00:00Z', ftp: 220 }),
      session({ id: 'e', start_time: '2025-05-01T17:00:00Z', ftp: 210 }),
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
      session({ id: 'a', start_time: '2025-01-01T17:00:00Z', ftp: 200 }),
      session({ id: 'b', start_time: '2025-06-01T17:00:00Z', ftp: 200 }),
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
