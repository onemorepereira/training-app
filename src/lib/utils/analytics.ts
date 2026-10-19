import type { SessionSummary } from '$lib/tauri';

export interface PmcDay {
  date: string;
  tss: number;
  ctl: number;
  atl: number;
  tsb: number;
}

export interface WeekBucket {
  weekStart: string;
  totalTss: number;
  avgPower: number | null;
  avgHr: number | null;
  sessionCount: number;
  totalDurationSecs: number;
}

export interface FtpPoint {
  date: string;
  ftp: number;
}

/*
 * Calendar keys ("YYYY-MM-DD") are computed in the user's local time zone.
 * Day/week arithmetic is done on keys via Date.UTC, which has no DST, so a
 * 23- or 25-hour local day can never skip or duplicate a key.
 */

const pad2 = (n: number) => String(n).padStart(2, '0');

/** Local calendar date of an instant (ISO string or Date). */
export function localDateKey(input: string | Date): string {
  const d = typeof input === 'string' ? new Date(input) : input;
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}

function keyToUtc(key: string): Date {
  const [y, m, d] = key.split('-').map(Number);
  return new Date(Date.UTC(y, m - 1, d));
}

function utcToKey(d: Date): string {
  return `${d.getUTCFullYear()}-${pad2(d.getUTCMonth() + 1)}-${pad2(d.getUTCDate())}`;
}

/** Calendar key `n` days after `key`. */
export function addDaysToKey(key: string, n: number): string {
  const d = keyToUtc(key);
  d.setUTCDate(d.getUTCDate() + n);
  return utcToKey(d);
}

/** Monday of the week containing calendar day `key`. */
export function weekStartKey(key: string): string {
  const day = keyToUtc(key).getUTCDay();
  // Sunday=0 → offset 6, Monday=1 → 0, Tuesday=2 → 1, etc.
  return addDaysToKey(key, day === 0 ? -6 : 1 - day);
}

/**
 * Compute a Performance Management Chart from session data.
 * Walks each local calendar day from the first session to today, applying
 * exponential moving averages for CTL (42-day) and ATL (7-day).
 */
export function computePmc(sessions: SessionSummary[], now: Date = new Date()): PmcDay[] {
  if (sessions.length === 0) return [];

  const tssByDate = new Map<string, number>();
  let firstDate: string | null = null;
  for (const s of sessions) {
    const key = localDateKey(s.start_time);
    tssByDate.set(key, (tssByDate.get(key) ?? 0) + (s.tss ?? 0));
    if (firstDate === null || key < firstDate) firstDate = key;
  }

  const today = localDateKey(now);
  const result: PmcDay[] = [];
  let ctl = 0;
  let atl = 0;

  for (let key = firstDate!; key <= today; key = addDaysToKey(key, 1)) {
    const tss = tssByDate.get(key) ?? 0;
    ctl = ctl + (tss - ctl) / 42;
    atl = atl + (tss - atl) / 7;
    result.push({ date: key, tss, ctl, atl, tsb: ctl - atl });
  }

  return result;
}

/**
 * Group sessions by local week (Monday start), from the first ride's week to the
 * current week; weeks without rides appear with zero volume. Average power/HR
 * are weighted by session duration.
 */
export function computeWeeklyTrends(sessions: SessionSummary[], now: Date = new Date()): WeekBucket[] {
  if (sessions.length === 0) return [];

  interface Acc {
    totalTss: number;
    powerSum: number;
    powerSecs: number;
    hrSum: number;
    hrSecs: number;
    count: number;
    totalDuration: number;
  }
  const buckets = new Map<string, Acc>();

  for (const s of sessions) {
    const key = weekStartKey(localDateKey(s.start_time));
    let b = buckets.get(key);
    if (!b) {
      b = { totalTss: 0, powerSum: 0, powerSecs: 0, hrSum: 0, hrSecs: 0, count: 0, totalDuration: 0 };
      buckets.set(key, b);
    }
    const secs = s.duration_secs > 0 ? s.duration_secs : 0;
    b.totalTss += s.tss ?? 0;
    if (s.avg_power != null) { b.powerSum += s.avg_power * secs; b.powerSecs += secs; }
    if (s.avg_hr != null) { b.hrSum += s.avg_hr * secs; b.hrSecs += secs; }
    b.count += 1;
    b.totalDuration += s.duration_secs;
  }

  const keys = [...buckets.keys()].sort();
  const thisWeek = weekStartKey(localDateKey(now));
  const last = keys[keys.length - 1] > thisWeek ? keys[keys.length - 1] : thisWeek;

  const result: WeekBucket[] = [];
  for (let key = keys[0]; key <= last; key = addDaysToKey(key, 7)) {
    const b = buckets.get(key);
    result.push({
      weekStart: key,
      totalTss: b?.totalTss ?? 0,
      avgPower: b && b.powerSecs > 0 ? b.powerSum / b.powerSecs : null,
      avgHr: b && b.hrSecs > 0 ? b.hrSum / b.hrSecs : null,
      sessionCount: b?.count ?? 0,
      totalDurationSecs: b?.totalDuration ?? 0,
    });
  }
  return result;
}

/** Rounded TSS of the current local week, 0 if no ride this week, null without data. */
export function currentWeekTss(weekly: WeekBucket[], now: Date = new Date()): number | null {
  if (weekly.length === 0) return null;
  const thisWeek = weekStartKey(localDateKey(now));
  const bucket = weekly.find((w) => w.weekStart === thisWeek);
  return bucket ? Math.round(bucket.totalTss) : 0;
}

export interface RampRate {
  current: number; // CTL delta over 7 days
  classification:
    | 'recovery'
    | 'maintenance'
    | 'moderate'
    | 'aggressive'
    | 'excessive';
}

export function computeRampRate(pmcData: PmcDay[]): RampRate | null {
  if (pmcData.length < 8) return null;
  const current = pmcData[pmcData.length - 1].ctl - pmcData[pmcData.length - 8].ctl;
  let classification: RampRate['classification'];
  if (current < -2) classification = 'recovery';
  else if (current < 2) classification = 'maintenance';
  else if (current < 5) classification = 'moderate';
  else if (current < 8) classification = 'aggressive';
  else classification = 'excessive';
  return { current, classification };
}

/** Extract FTP change points: emit when FTP value changes from previous. Always emit first and last. */
export function extractFtpProgression(sessions: SessionSummary[]): FtpPoint[] {
  const sorted = [...sessions]
    .filter((s) => s.ftp != null)
    .sort((a, b) => a.start_time.localeCompare(b.start_time));

  if (sorted.length === 0) return [];

  const result: FtpPoint[] = [];
  let prevFtp: number | null = null;

  for (let i = 0; i < sorted.length; i++) {
    const ftp = sorted[i].ftp!;
    if (ftp !== prevFtp) {
      result.push({ date: localDateKey(sorted[i].start_time), ftp });
      prevFtp = ftp;
    }
  }

  // Always include the last point if it wasn't already added as a change
  const lastSession = sorted[sorted.length - 1];
  const lastResult = result[result.length - 1];
  if (lastResult.date !== localDateKey(lastSession.start_time)) {
    result.push({
      date: localDateKey(lastSession.start_time),
      ftp: lastSession.ftp!,
    });
  }

  return result;
}
