<script lang="ts">
  import { zoneStatus } from '$lib/stores/zoneRide';
  import { formatDuration } from '$lib/utils/format';

  let {
    onStop,
  }: {
    onStop: () => void;
  } = $props();

  const POWER_COLORS = ['#78909c', '#4caf50', '#8bc34a', '#ffeb3b', '#ff9800', '#f44336', '#9c27b0'];
  const HR_COLORS = ['#4caf50', '#8bc34a', '#ffeb3b', '#ff9800', '#f44336'];

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
    const colors = $zoneStatus.mode === 'HeartRate' ? HR_COLORS : POWER_COLORS;
    return colors[z - 1] ?? 'var(--accent)';
  });

  let modeLabel = $derived.by(() => {
    if (!$zoneStatus) return '';
    const z = $zoneStatus.target_zone;
    const m = $zoneStatus.mode === 'HeartRate' ? 'HR' : 'PWR';
    return z ? `Z${z} ${m}` : m;
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
    {#if progress != null}
      <div class="progress-track">
        <div class="progress-fill" style="width: {progress}%"></div>
      </div>
    {/if}

    <div class="status-content">
      <span class="zone-badge">{modeLabel}</span>

      {#if $zoneStatus.lower_bound != null && $zoneStatus.upper_bound != null}
        <span class="target-range">
          {$zoneStatus.lower_bound}&ndash;{$zoneStatus.upper_bound}
          <span class="range-unit">{$zoneStatus.mode === 'HeartRate' ? 'bpm' : 'W'}</span>
        </span>
      {/if}

      <span class="status-sep"></span>

      <span class="phase-indicator">
        <span class="phase-dot" style="background: {phaseColor}"></span>
        <span class="phase-text">{$zoneStatus.phase.replace('_', ' ')}</span>
      </span>

      <span class="status-sep"></span>

      <span class="time-stat">
        <span class="time-label">In zone</span>
        <span class="time-value">{formatDuration($zoneStatus.time_in_zone_secs)}</span>
      </span>

      {#if remaining != null}
        <span class="time-stat">
          <span class="time-label">Left</span>
          <span class="time-value">{formatDuration(remaining)}</span>
        </span>
      {/if}

      {#if $zoneStatus.commanded_power != null}
        <span class="cmd-power">
          {$zoneStatus.commanded_power}<span class="cmd-unit">W</span>
        </span>
      {/if}

      {#if $zoneStatus.safety_note}
        <span class="safety-badge">{$zoneStatus.safety_note}</span>
      {/if}

      <button class="btn-stop" onclick={onStop}>Stop</button>
    </div>
  </div>
{/if}

<style>
  .zone-status {
    position: relative;
    background: var(--bg-surface);
    border: 1px solid color-mix(in srgb, var(--zone-color) 30%, transparent);
    border-radius: var(--radius-md);
    overflow: hidden;
    animation: slide-up 200ms ease;
  }

  @keyframes slide-up {
    from { opacity: 0; transform: translateY(4px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .progress-track {
    height: 3px;
    background: var(--bg-body);
  }

  .progress-fill {
    height: 100%;
    background: var(--zone-color);
    transition: width 1s linear;
    border-radius: 0 2px 2px 0;
  }

  .status-content {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: var(--space-sm) var(--space-md);
    flex-wrap: wrap;
  }

  .zone-badge {
    padding: 2px var(--space-sm);
    border-radius: var(--radius-sm);
    background: var(--zone-color);
    color: white;
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.04em;
    white-space: nowrap;
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
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

  .status-sep {
    width: 1px;
    height: 16px;
    background: var(--border-default);
    flex-shrink: 0;
  }

  .phase-indicator {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .phase-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
    animation: pulse-dot 2s ease-in-out infinite;
  }

  @keyframes pulse-dot {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  .phase-text {
    font-size: var(--text-xs);
    color: var(--text-muted);
    text-transform: capitalize;
    font-weight: 500;
  }

  .time-stat {
    display: flex;
    align-items: baseline;
    gap: 4px;
  }

  .time-label {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-weight: 500;
  }

  .time-value {
    font-family: var(--font-data);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    font-weight: 700;
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

  .safety-badge {
    font-size: var(--text-xs);
    color: var(--warning);
    font-weight: 600;
    padding: 2px var(--space-sm);
    background: rgba(255, 183, 77, 0.08);
    border: 1px solid rgba(255, 183, 77, 0.2);
    border-radius: var(--radius-sm);
    animation: pulse-dot 2s ease-in-out infinite;
  }

  .btn-stop {
    margin-left: auto;
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

  .btn-stop:hover {
    background: var(--danger);
    color: white;
    box-shadow: 0 2px 8px rgba(244, 67, 54, 0.3);
  }
</style>
