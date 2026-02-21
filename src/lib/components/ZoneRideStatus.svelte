<script lang="ts">
  import { zoneStatus } from '$lib/stores/zoneRide';
  import { formatDuration } from '$lib/utils/format';

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

  let modeLabel = $derived.by(() => {
    if (!$zoneStatus) return '';
    const z = $zoneStatus.target_zone;
    const m = $zoneStatus.mode === 'HeartRate' ? 'HR' : 'PWR';
    return z ? `Z${z} ${m}` : `${m}`;
  });

  let remaining = $derived.by(() => {
    if (!$zoneStatus?.duration_secs) return null;
    const left = $zoneStatus.duration_secs - $zoneStatus.elapsed_secs;
    return Math.max(0, left);
  });
</script>

{#if $zoneStatus?.active}
  <div class="zone-status">
    <span class="zone-badge">{modeLabel}</span>

    {#if $zoneStatus.lower_bound != null && $zoneStatus.upper_bound != null}
      <span class="target-range">
        {$zoneStatus.lower_bound}&ndash;{$zoneStatus.upper_bound}
        <span class="range-unit">{$zoneStatus.mode === 'HeartRate' ? 'bpm' : 'W'}</span>
      </span>
    {/if}

    <span class="phase-dot" style="background: {phaseColor}"></span>
    <span class="phase-label">{$zoneStatus.phase}</span>

    <span class="status-divider"></span>

    <span class="time-label">
      In zone: <strong>{formatDuration($zoneStatus.time_in_zone_secs)}</strong>
    </span>

    {#if remaining != null}
      <span class="time-label">
        Left: <strong>{formatDuration(remaining)}</strong>
      </span>
    {/if}

    {#if $zoneStatus.commanded_power != null}
      <span class="cmd-power">{$zoneStatus.commanded_power}<span class="cmd-unit">W</span></span>
    {/if}

    {#if $zoneStatus.safety_note}
      <span class="safety-note">{$zoneStatus.safety_note}</span>
    {/if}

    <button class="btn-stop-zone" onclick={onStop}>Stop</button>
  </div>
{/if}

<style>
  .zone-status {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    flex-wrap: wrap;
  }

  .zone-badge {
    padding: var(--space-xs) var(--space-sm);
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: white;
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.04em;
    white-space: nowrap;
  }

  .target-range {
    font-family: var(--font-data);
    font-size: var(--text-sm);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--text-primary);
  }

  .range-unit {
    font-size: 0.8em;
    color: var(--text-muted);
    font-weight: 500;
  }

  .phase-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .phase-label {
    font-size: var(--text-xs);
    color: var(--text-muted);
    text-transform: capitalize;
  }

  .status-divider {
    width: 1px;
    height: 18px;
    background: var(--border-default);
    flex-shrink: 0;
  }

  .time-label {
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .time-label strong {
    font-family: var(--font-data);
    font-variant-numeric: tabular-nums;
    color: var(--text-primary);
  }

  .cmd-power {
    font-family: var(--font-data);
    font-size: var(--text-sm);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    color: var(--info);
  }

  .cmd-unit {
    font-size: 0.7em;
    font-weight: 500;
    color: var(--text-muted);
    margin-left: 1px;
  }

  .safety-note {
    font-size: var(--text-xs);
    color: var(--warning);
    font-weight: 500;
  }

  .btn-stop-zone {
    padding: var(--space-xs) var(--space-md);
    border: 1px solid var(--danger);
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--danger);
    font-size: var(--text-xs);
    font-weight: 700;
    cursor: pointer;
    transition: all var(--transition-fast);
    white-space: nowrap;
  }

  .btn-stop-zone:hover {
    background: var(--danger);
    color: white;
  }
</style>
