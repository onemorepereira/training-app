/**
 * HR zone (1–5) for a heart rate, using the backend's inclusive-upper rule
 * (Rust `MetricsCalculator::hr_zone`): the first zone whose upper bound is >= bpm;
 * above every bound is zone 5.
 */
export function hrZoneForBpm(bpm: number, zones: readonly number[]): number {
  for (let i = 0; i < zones.length; i++) {
    if (bpm <= zones[i]) return i + 1;
  }
  return 5;
}
