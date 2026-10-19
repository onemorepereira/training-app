<script lang="ts">
  import { zoneStatus } from '$lib/stores/zoneRide';
  import { formatDuration } from '$lib/utils/format';
  import { POWER_ZONE_COLORS, HR_ZONE_COLORS } from '$lib/charts/theme';

  let {
    onStop,
  }: {
    onStop: () => void;
  } = $props();


  let phaseColor = $derived.by(() => {
    const phase = $zoneStatus?.phase;
    if (phase === 'in_zone') return 'var(--success)';
    if (phase === 'ramping' || phase === 'adjusting') return 'var(--warning)';
    return 'var(--text-muted)';
  });

  let zoneColor = $derived.by(() => {
    if (!$zoneStatus) return 'var(--accent)';
    const z = $zoneStatus.target_zone;
    if (!z) return 'var(--accent)';
    const colors: readonly string[] = $zoneStatus.mode === 'HeartRate' ? HR_ZONE_COLORS : POWER_ZONE_COLORS;
    return colors[z - 1] ?? 'var(--accent)';
  });

  let modeDescription = $derived.by(() => {
    if (!$zoneStatus) return '';
    const z = $zoneStatus.target_zone;
    const m = $zoneStatus.mode === 'HeartRate' ? 'HR' : 'Power';
    return z ? `${m} Zone ${z}` : m;
  });

  let remaining = $derived.by(() => {
    if (!$zoneStatus?.duration_secs) return null;
    const left = $zoneStatus.duration_secs - $zoneStatus.elapsed_secs;
    return Math.max(0, left);
  });

  let progress = $derived.by(() => {
    if (!$zoneStatus?.duration_secs || !$zoneStatus.duration_secs) return null;
    return Math.min(100, ($zoneStatus.elapsed_secs / $zoneStatus.duration_secs) * 100);
  });
</script>

{#if $zoneStatus?.active}
  <div class="zone-status" style="--zone-color: {zoneColor}">
    <span class="zone-badge">
      <span class="zone-swatch" aria-hidden="true"></span>{modeDescription}
    </span>

    {#if $zoneStatus.lower_bound != null && $zoneStatus.upper_bound != null}
      <span class="stat">
        <span class="stat-label">Target</span>
        <span class="stat-value data">{$zoneStatus.lower_bound}–{$zoneStatus.upper_bound}<span class="stat-unit">{$zoneStatus.mode === 'HeartRate' ? 'bpm' : 'W'}</span></span>
      </span>
    {/if}

    <span class="stat">
      <span class="stat-label">Status</span>
      <span class="stat-value phase"><span class="phase-dot" style="background: {phaseColor}" aria-hidden="true"></span>{$zoneStatus.phase.replace('_', ' ')}</span>
    </span>

    <span class="stat">
      <span class="stat-label">In zone</span>
      <span class="stat-value data">{formatDuration($zoneStatus.time_in_zone_secs)}</span>
    </span>

    {#if remaining != null && $zoneStatus.duration_secs}
      <span class="stat">
        <span class="stat-label">Left</span>
        <span class="stat-value data">{formatDuration(remaining)}<span class="stat-unit">/ {formatDuration($zoneStatus.duration_secs)}</span></span>
      </span>
    {/if}

    {#if $zoneStatus.commanded_power != null}
      <span class="stat">
        <span class="stat-label">Trainer</span>
        <span class="stat-value data">{$zoneStatus.commanded_power}<span class="stat-unit">W</span></span>
      </span>
    {/if}

    {#if $zoneStatus.safety_note}
      <span class="safety-badge">{$zoneStatus.safety_note}</span>
    {/if}

    <button type="button" class="btn btn-lg btn-secondary stop" data-ride-control onclick={onStop}>Stop zone ride</button>

    {#if progress != null}
      <div class="progress-track" role="progressbar" aria-label="Zone ride progress" aria-valuenow={Math.round(progress)} aria-valuemin={0} aria-valuemax={100}>
        <div class="progress-fill" style="width: {progress}%"></div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .zone-status {
    position: relative;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-sm) var(--space-xl);
    padding: var(--space-sm) var(--space-md) var(--space-md);
    border: 1px solid var(--border-default);
    border-left: 4px solid var(--zone-color);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    overflow: hidden;
    animation: slide-up 200ms ease;
  }

  .zone-badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    font-size: var(--text-lg);
    font-weight: 700;
    color: var(--text-primary);
  }

  .zone-swatch {
    width: 12px;
    height: 12px;
    border-radius: 3px;
    background: var(--zone-color);
  }

  .stat {
    display: flex;
    flex-direction: column;
    line-height: 1.2;
  }

  .stat-label {
    font-size: var(--text-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
  }

  .stat-value {
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--text-primary);
  }

  .stat-unit {
    margin-left: 4px;
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--text-muted);
  }

  .phase {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    text-transform: capitalize;
    font-family: var(--font-sans);
  }

  .phase-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .safety-badge {
    padding: 4px 10px;
    border-radius: var(--radius-sm);
    border: 1px solid rgba(255, 183, 77, 0.4);
    background: rgba(255, 183, 77, 0.1);
    color: var(--warning);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .stop {
    margin-left: auto;
  }

  .progress-track {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 4px;
    background: var(--border-subtle);
  }

  .progress-fill {
    height: 100%;
    background: var(--zone-color);
    transition: width 1s linear;
  }
</style>
