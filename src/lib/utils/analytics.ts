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

/** Extract YYYY-MM-DD from an ISO string via slicing (avoids timezone issues). */
export function toDateKey(iso: string): string {
  return iso.slice(0, 10);
}

/** Get the Monday of the ISO week for a given Date. */
export function weekMonday(date: Date): Date {
  const d = new Date(date);
  const day = d.getUTCDay();
  // Sunday=0 → offset 6, Monday=1 → 0, Tuesday=2 → 1, etc.
  const offset = day === 0 ? 6 : day - 1;
  d.setUTCDate(d.getUTCDate() - offset);
  d.setUTCHours(0, 0, 0, 0);
  return d;
}

/**
 * Compute a Performance Management Chart from session data.
 * Walks each calendar day from the first session to today, applying
 * exponential moving averages for CTL (42-day) and ATL (7-day).
 */
export function computePmc(sessions: SessionSummary[]): PmcDay[] {
  if (sessions.length === 0) return [];

  // Sort ascending by start_time
  const sorted = [...sessions].sort(
    (a, b) => a.start_time.localeCompare(b.start_time),
  );

  // Group TSS by date key
  const tssByDate = new Map<string, number>();
  for (const s of sorted) {
    const key = toDateKey(s.start_time);
    tssByDate.set(key, (tssByDate.get(key) ?? 0) + (s.tss ?? 0));
  }

  const firstDate = toDateKey(sorted[0].start_time);
  const today = toDateKey(new Date().toISOString());

  const result: PmcDay[] = [];
  let ctl = 0;
  let atl = 0;

  // Walk each day from first session to today
  const current = new Date(firstDate + 'T00:00:00Z');
  const end = new Date(today + 'T00:00:00Z');

  while (current <= end) {
    const key = toDateKey(current.toISOString());
    const tss = tssByDate.get(key) ?? 0;

    ctl = ctl + (tss - ctl) / 42;
    atl = atl + (tss - atl) / 7;
    const tsb = ctl - atl;

    result.push({ date: key, tss, ctl, atl, tsb });

    current.setUTCDate(current.getUTCDate() + 1);
  }

  return result;
}

/** Group sessions by ISO week (Monday start) and compute aggregates. */
export function computeWeeklyTrends(sessions: SessionSummary[]): WeekBucket[] {
  if (sessions.length === 0) return [];

  const buckets = new Map<
    string,
    {
      totalTss: number;
      powers: number[];
      hrs: number[];
      count: number;
      totalDuration: number;
    }
  >();

  for (const s of sessions) {
    const date = new Date(s.start_time);
    const monday = weekMonday(date);
    const key = toDateKey(monday.toISOString());

    let bucket = buckets.get(key);
    if (!bucket) {
      bucket = { totalTss: 0, powers: [], hrs: [], count: 0, totalDuration: 0 };
      buckets.set(key, bucket);
    }

    bucket.totalTss += s.tss ?? 0;
    if (s.avg_power != null) bucket.powers.push(s.avg_power);
    if (s.avg_hr != null) bucket.hrs.push(s.avg_hr);
    bucket.count += 1;
    bucket.totalDuration += s.duration_secs;
  }

  return [...buckets.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([weekStart, b]) => ({
      weekStart,
      totalTss: b.totalTss,
      avgPower:
        b.powers.length > 0
          ? b.powers.reduce((a, v) => a + v, 0) / b.powers.length
          : null,
      avgHr:
        b.hrs.length > 0
          ? b.hrs.reduce((a, v) => a + v, 0) / b.hrs.length
          : null,
      sessionCount: b.count,
      totalDurationSecs: b.totalDuration,
    }));
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
      result.push({ date: toDateKey(sorted[i].start_time), ftp });
      prevFtp = ftp;
    }
  }

  // Always include the last point if it wasn't already added as a change
  const lastSession = sorted[sorted.length - 1];
  const lastResult = result[result.length - 1];
  if (lastResult.date !== toDateKey(lastSession.start_time)) {
    result.push({
      date: toDateKey(lastSession.start_time),
      ftp: lastSession.ftp!,
    });
  }

  return result;
}
