/**
 * Post-ride zone analysis utilities.
 * Pure functions for computing decoupling, time-in-zone, and time-to-zone.
 */

interface HrPowerPoint {
  power: number | null;
  heart_rate: number | null;
}

interface ValuePoint {
  value: number | null;
}

/**
 * Compute HR:Power decoupling percentage.
 *
 * Splits paired (HR, power) data into first and second halves,
 * computes the HR/power ratio for each half, and returns the
 * percentage change. Higher decoupling indicates cardiac drift.
 *
 * Returns null if fewer than 20 paired data points.
 */
export function computeDecoupling(timeseries: HrPowerPoint[]): number | null {
  const paired = timeseries.filter(
    (pt): pt is { power: number; heart_rate: number } =>
      pt.power != null && pt.heart_rate != null && pt.power > 0,
  );

  if (paired.length < 20) return null;

  const mid = Math.floor(paired.length / 2);
  const firstHalf = paired.slice(0, mid);
  const secondHalf = paired.slice(mid);

  const avgHr1 = firstHalf.reduce((s, p) => s + p.heart_rate, 0) / firstHalf.length;
  const avgPw1 = firstHalf.reduce((s, p) => s + p.power, 0) / firstHalf.length;
  const avgHr2 = secondHalf.reduce((s, p) => s + p.heart_rate, 0) / secondHalf.length;
  const avgPw2 = secondHalf.reduce((s, p) => s + p.power, 0) / secondHalf.length;

  if (avgPw1 === 0 || avgPw2 === 0) return null;

  const ratio1 = avgHr1 / avgPw1;
  const ratio2 = avgHr2 / avgPw2;

  return ((ratio2 - ratio1) / ratio1) * 100;
}

/**
 * Compute time spent in/below/above a zone from a value series.
 *
 * Each point represents one `intervalSecs` of data.
 * Null values are skipped (not counted in any bucket).
 */
export function computeTimeInZone(
  timeseries: ValuePoint[],
  lower: number,
  upper: number,
  intervalSecs: number,
): { inZoneSecs: number; belowSecs: number; aboveSecs: number } {
  let inZoneSecs = 0;
  let belowSecs = 0;
  let aboveSecs = 0;

  for (const pt of timeseries) {
    if (pt.value == null) continue;
    if (pt.value < lower) belowSecs += intervalSecs;
    else if (pt.value > upper) aboveSecs += intervalSecs;
    else inZoneSecs += intervalSecs;
  }

  return { inZoneSecs, belowSecs, aboveSecs };
}

/**
 * Compute the time (in seconds) until the first in-zone reading.
 *
 * Returns null if the value never enters the zone.
 */
export function computeTimeToZone(
  timeseries: ValuePoint[],
  lower: number,
  upper: number,
  intervalSecs: number,
): number | null {
  for (let i = 0; i < timeseries.length; i++) {
    const v = timeseries[i].value;
    if (v != null && v >= lower && v <= upper) {
      return i * intervalSecs;
    }
  }
  return null;
}
