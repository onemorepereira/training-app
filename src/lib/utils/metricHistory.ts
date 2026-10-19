export interface MetricHistoryEntry {
  t: number;
  power: number | null;
  hr: number | null;
  cadence: number | null;
  speed: number | null;
}

type Channels = Pick<MetricHistoryEntry, 'power' | 'hr' | 'cadence' | 'speed'>;
type LastSeen = Record<keyof Channels, number>;

/** Append one sample, keeping at most `max` entries (oldest dropped). */
export function appendSample(
  history: MetricHistoryEntry[],
  sample: MetricHistoryEntry,
  max: number,
): MetricHistoryEntry[] {
  const next = [...history, sample];
  return next.length > max ? next.slice(next.length - max) : next;
}

/**
 * Build a chart sample from the latest values. A channel whose last reading
 * (`lastSeen`, ms; 0 = never) is older than `freshnessMs` is recorded as null,
 * so a dropped sensor shows a gap rather than a flat line.
 */
export function buildSample(
  latest: Channels,
  lastSeen: LastSeen,
  now: number,
  freshnessMs: number,
): MetricHistoryEntry {
  const pick = (k: keyof Channels) =>
    lastSeen[k] > 0 && now - lastSeen[k] <= freshnessMs ? latest[k] : null;
  return { t: now, power: pick('power'), hr: pick('hr'), cadence: pick('cadence'), speed: pick('speed') };
}
