<script lang="ts">
  import type { ZoneRideConfig } from '$lib/tauri';
  import { computeDecoupling, computeTimeInZone, computeTimeToZone } from '$lib/utils/zoneAnalysis';
  import { formatDuration } from '$lib/utils/format';

  let {
    config,
    timeseries,
  }: {
    config: ZoneRideConfig;
    timeseries: { power: number | null; heart_rate: number | null }[];
  } = $props();

  let modeLabel = $derived(config.mode === 'HeartRate' ? 'HR' : 'Power');
  let unit = $derived(config.mode === 'HeartRate' ? 'bpm' : 'W');
  let zoneLabel = $derived(config.zone > 0 ? `Z${config.zone}` : 'Custom');

  // Build value series from the appropriate channel
  let valueSeries = $derived.by(() => {
    return timeseries.map((pt) => ({
      value: config.mode === 'HeartRate' ? (pt.heart_rate as number | null) : (pt.power as number | null),
    }));
  });

  let timeInZone = $derived(
    computeTimeInZone(valueSeries, config.lower_bound, config.upper_bound, 1),
  );

  let timeToZone = $derived(
    computeTimeToZone(valueSeries, config.lower_bound, config.upper_bound, 1),
  );

  let decoupling = $derived(computeDecoupling(timeseries));

  let totalTracked = $derived(timeInZone.inZoneSecs + timeInZone.belowSecs + timeInZone.aboveSecs);
  let inZonePct = $derived(totalTracked > 0 ? (timeInZone.inZoneSecs / totalTracked) * 100 : 0);
  let belowPct = $derived(totalTracked > 0 ? (timeInZone.belowSecs / totalTracked) * 100 : 0);
  let abovePct = $derived(totalTracked > 0 ? (timeInZone.aboveSecs / totalTracked) * 100 : 0);

  let decouplingLabel = $derived.by(() => {
    if (decoupling == null) return null;
    const abs = Math.abs(decoupling);
    if (abs < 3) return 'Excellent';
    if (abs < 5) return 'Good';
    if (abs < 8) return 'Moderate drift';
    return 'Significant drift';
  });

  let decouplingColor = $derived.by(() => {
    if (decoupling == null) return 'var(--text-muted)';
    const abs = Math.abs(decoupling);
    if (abs < 5) return 'var(--success)';
    if (abs < 8) return 'var(--warning)';
    return 'var(--danger)';
  });
</script>

<section class="zone-analysis">
  <h2>Zone Ride Analysis</h2>

  <div class="zone-summary">
    <div class="summary-badge">
      <span class="badge-mode">{zoneLabel} {modeLabel}</span>
      <span class="badge-range">{config.lower_bound}&ndash;{config.upper_bound} {unit}</span>
    </div>

    <div class="summary-stats">
      <div class="stat">
        <span class="stat-label">Time in Zone</span>
        <span class="stat-value">{formatDuration(config.time_in_zone_secs)}</span>
        <span class="stat-sub">{inZonePct.toFixed(0)}%</span>
      </div>

      {#if timeToZone != null}
        <div class="stat">
          <span class="stat-label">Time to Zone</span>
          <span class="stat-value">{formatDuration(timeToZone)}</span>
        </div>
      {/if}

      {#if config.duration_secs != null}
        <div class="stat">
          <span class="stat-label">Target Duration</span>
          <span class="stat-value">{formatDuration(config.duration_secs)}</span>
        </div>
      {/if}

      {#if decoupling != null}
        <div class="stat">
          <span class="stat-label">Decoupling</span>
          <span class="stat-value" style="color: {decouplingColor}">
            {decoupling.toFixed(1)}%
          </span>
          <span class="stat-sub" style="color: {decouplingColor}">{decouplingLabel}</span>
        </div>
      {/if}
    </div>
  </div>

  <!-- Time in zone bar -->
  <div class="zone-bar-wrap">
    <div class="zone-bar">
      {#if belowPct > 0}
        <div class="bar-segment below" style="width: {belowPct}%" title="Below: {formatDuration(timeInZone.belowSecs)}"></div>
      {/if}
      {#if inZonePct > 0}
        <div class="bar-segment in-zone" style="width: {inZonePct}%" title="In zone: {formatDuration(timeInZone.inZoneSecs)}"></div>
      {/if}
      {#if abovePct > 0}
        <div class="bar-segment above" style="width: {abovePct}%" title="Above: {formatDuration(timeInZone.aboveSecs)}"></div>
      {/if}
    </div>
    <div class="bar-legend">
      <span class="legend-item"><span class="dot below"></span> Below</span>
      <span class="legend-item"><span class="dot in-zone"></span> In Zone</span>
      <span class="legend-item"><span class="dot above"></span> Above</span>
    </div>
  </div>
</section>

<style>
  .zone-analysis {
    margin-bottom: var(--space-xl);
  }

  h2 {
    margin: 0 0 var(--space-md);
    font-size: var(--text-base);
    font-weight: 700;
    color: var(--text-primary);
  }

  .zone-summary {
    display: flex;
    align-items: flex-start;
    gap: var(--space-lg);
    flex-wrap: wrap;
    margin-bottom: var(--space-md);
  }

  .summary-badge {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: var(--space-sm) var(--space-md);
    background: var(--accent-soft);
    border-radius: var(--radius-md);
    flex-shrink: 0;
  }

  .badge-mode {
    font-size: var(--text-sm);
    font-weight: 700;
    color: var(--accent);
    letter-spacing: 0.04em;
  }

  .badge-range {
    font-family: var(--font-data);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: var(--text-secondary);
  }

  .summary-stats {
    display: flex;
    gap: var(--space-lg);
    flex-wrap: wrap;
  }

  .stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .stat-label {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-weight: 500;
  }

  .stat-value {
    font-family: var(--font-data);
    font-size: var(--text-lg);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    color: var(--text-primary);
  }

  .stat-sub {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-muted);
  }

  /* Zone bar */
  .zone-bar-wrap {
    margin-top: var(--space-sm);
  }

  .zone-bar {
    display: flex;
    height: 12px;
    border-radius: var(--radius-full);
    overflow: hidden;
    background: var(--bg-body);
  }

  .bar-segment {
    transition: width 300ms ease;
    min-width: 2px;
  }

  .bar-segment.below {
    background: var(--info);
  }

  .bar-segment.in-zone {
    background: var(--success);
  }

  .bar-segment.above {
    background: var(--danger);
  }

  .bar-legend {
    display: flex;
    gap: var(--space-md);
    margin-top: var(--space-xs);
  }

  .legend-item {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .dot.below {
    background: var(--info);
  }

  .dot.in-zone {
    background: var(--success);
  }

  .dot.above {
    background: var(--danger);
  }
</style>
