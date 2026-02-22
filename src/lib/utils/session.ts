import type { SessionSummary } from '$lib/tauri';
import { autoTitle } from './format';

export const TYPE_LABELS: Record<string, string> = {
  endurance: 'Endurance',
  intervals: 'Intervals',
  threshold: 'Threshold',
  sweet_spot: 'Sweet Spot',
  vo2max: 'VO2max',
  sprint: 'Sprint',
  tempo: 'Tempo',
  recovery: 'Recovery',
  race: 'Race',
  test: 'Test',
  warmup: 'Warmup',
  group_ride: 'Group Ride',
  free_ride: 'Free Ride',
  other: 'Other',
};

export function displayTitle(session: SessionSummary): string {
  return session.title ?? autoTitle(session.start_time);
}

/** RPE gradient: green (1) -> yellow (5) -> red (10) */
export function rpeColor(value: number): string {
  if (value <= 5) {
    const ratio = (value - 1) / 4;
    const r = Math.round(76 + ratio * (255 - 76));
    const g = Math.round(175 + ratio * (255 - 175));
    const b = Math.round(80 + ratio * (77 - 80));
    return `rgb(${r}, ${g}, ${b})`;
  }
  const ratio = (value - 5) / 5;
  const r = Math.round(255 - ratio * (255 - 244));
  const g = Math.round(255 - ratio * (255 - 67));
  const b = Math.round(77 - ratio * (77 - 54));
  return `rgb(${r}, ${g}, ${b})`;
}
