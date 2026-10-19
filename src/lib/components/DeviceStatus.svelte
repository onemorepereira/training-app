<script lang="ts">
  import { activeDevices } from '$lib/stores/devices';
  import { currentPower, currentHR, currentCadence, currentSpeed } from '$lib/stores/sensor';
  import { unitSystem, formatSpeed, speedUnit } from '$lib/stores/units';
  import { SERIES } from '$lib/charts/theme';

  /** Connected-devices summary at the bottom of the nav: full in the sidebar, count + tooltip in the rail. */
  let { expanded = false }: { expanded?: boolean } = $props();

  let count = $derived($activeDevices.length);
  let summary = $derived(count === 0 ? 'None connected' : `${count} connected`);
  let accessibleName = $derived(count === 0 ? 'No devices connected' : `${count} device${count === 1 ? '' : 's'} connected`);

  let channels = $derived(
    [
      { key: 'power', label: 'Power', color: SERIES.power, value: $currentPower != null ? `${$currentPower} W` : null },
      { key: 'hr', label: 'Heart rate', color: SERIES.hr, value: $currentHR != null ? `${$currentHR} bpm` : null },
      { key: 'cadence', label: 'Cadence', color: SERIES.cadence, value: $currentCadence != null ? `${Math.round($currentCadence)} rpm` : null },
      { key: 'speed', label: 'Speed', color: SERIES.speed, value: $currentSpeed != null ? `${formatSpeed($currentSpeed, $unitSystem)} ${$speedUnit}` : null },
    ].filter((c) => c.value != null)
  );
</script>

<a href="/devices" class="device-status" class:expanded aria-label={accessibleName}>
  <span class="status-head">
    <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M8.5 8.5a5 5 0 0 0 0 7M15.5 8.5a5 5 0 0 1 0 7M5.6 5.6a9 9 0 0 0 0 12.8M18.4 5.6a9 9 0 0 1 0 12.8" />
      <circle cx="12" cy="12" r="1.5" />
    </svg>
    <span class="status-dot" class:on={count > 0} aria-hidden="true"></span>
    {#if expanded}
      <span class="status-text">{summary}</span>
    {:else}
      <span class="status-count" aria-hidden="true">{count}</span>
    {/if}
  </span>

  {#if expanded && channels.length}
    <span class="channels" aria-hidden="true">
      {#each channels as c (c.key)}
        <span class="channel"><span class="channel-dot" style="background: {c.color}"></span>{c.label}<span class="channel-value data">{c.value}</span></span>
      {/each}
    </span>
  {/if}

  {#if !expanded}
    <span class="tip" role="tooltip" aria-hidden="true">
      <strong>{summary}</strong>
      {#each $activeDevices as d (d.id)}<span>{d.name ?? d.id}</span>{/each}
    </span>
  {/if}
</a>

<style>
  .device-status {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    min-height: 44px;
    margin-top: auto;
    padding: var(--space-sm);
    border-radius: var(--radius-md);
    color: var(--text-secondary);
    text-decoration: none;
  }

  .device-status:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .device-status:not(.expanded) {
    align-items: center;
    width: 48px;
    margin-inline: auto;
  }

  .status-head {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-height: 28px;
  }

  .device-status:not(.expanded) .status-head {
    flex-direction: column;
    gap: 2px;
  }

  .status-dot {
    position: absolute;
    top: 0;
    left: 14px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-faint);
    border: 2px solid var(--bg-surface);
  }

  .status-dot.on {
    background: var(--success);
  }

  .status-text {
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .status-count {
    font-family: var(--font-data);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .channels {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .channel {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .channel-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .channel-value {
    margin-left: auto;
    color: var(--text-secondary);
  }

  .tip {
    position: absolute;
    left: calc(100% + 10px);
    bottom: 0;
    z-index: 200;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 10px;
    border: 1px solid var(--border-default);
    border-radius: var(--radius-sm);
    background: var(--bg-elevated);
    color: var(--text-secondary);
    font-size: var(--text-xs);
    white-space: nowrap;
    pointer-events: none;
    opacity: 0;
    transition: opacity var(--transition-fast);
    box-shadow: var(--shadow-md);
  }

  .tip strong {
    color: var(--text-primary);
  }

  .device-status:hover .tip,
  .device-status:focus-visible .tip {
    opacity: 1;
  }
</style>
