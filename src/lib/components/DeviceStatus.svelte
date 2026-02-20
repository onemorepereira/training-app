<script lang="ts">
  import { activeDevices } from '$lib/stores/devices';
  import { currentPower, currentHR, currentCadence, currentSpeed } from '$lib/stores/sensor';
  import { unitSystem, formatSpeed, speedUnit } from '$lib/stores/units';

  let { compact = false }: { compact?: boolean } = $props();
</script>

{#if compact}
  <div class="compact-status">
    <div class="status-dots">
      {#if $currentPower != null}
        <div class="status-dot power" title="Power: {$currentPower}W"></div>
      {/if}
      {#if $currentHR != null}
        <div class="status-dot hr" title="HR: {$currentHR}bpm"></div>
      {/if}
      {#if $currentCadence != null}
        <div class="status-dot cadence" title="Cadence: {Math.round($currentCadence)}rpm"></div>
      {/if}
      {#if $currentSpeed != null}
        <div class="status-dot speed" title="Speed: {formatSpeed($currentSpeed, $unitSystem)}{$speedUnit}"></div>
      {/if}
    </div>
    {#if $activeDevices.length > 0}
      <span class="compact-count">{$activeDevices.length}</span>
    {/if}
  </div>
{:else}
  <div class="device-status">
    <div class="header">
      <span class="dot" class:active={$activeDevices.length > 0}></span>
      <span class="count">{$activeDevices.length} device{$activeDevices.length !== 1 ? 's' : ''}</span>
    </div>
    <div class="readings">
      {#if $currentPower != null}
        <div class="reading">
          <span class="reading-label">PWR</span>
          <span class="reading-value">{$currentPower}<span class="reading-unit">W</span></span>
        </div>
      {/if}
      {#if $currentHR != null}
        <div class="reading">
          <span class="reading-label">HR</span>
          <span class="reading-value">{$currentHR}<span class="reading-unit">bpm</span></span>
        </div>
      {/if}
      {#if $currentCadence != null}
        <div class="reading">
          <span class="reading-label">CAD</span>
          <span class="reading-value">{Math.round($currentCadence)}<span class="reading-unit">rpm</span></span>
        </div>
      {/if}
      {#if $currentSpeed != null}
        <div class="reading">
          <span class="reading-label">SPD</span>
          <span class="reading-value">{formatSpeed($currentSpeed, $unitSystem)}<span class="reading-unit">{$speedUnit}</span></span>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  /* ── Compact mode (nav rail) ── */
  .compact-status {
    margin-top: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-md) 0;
  }

  .status-dots {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }

  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: var(--radius-full);
    transition: all var(--transition-base);
  }

  .status-dot.power {
    background: var(--accent);
    box-shadow: 0 0 8px var(--accent-glow);
  }

  .status-dot.hr {
    background: var(--danger);
    box-shadow: 0 0 8px rgba(244, 67, 54, 0.4);
  }

  .status-dot.cadence {
    background: var(--info);
    box-shadow: 0 0 8px rgba(100, 181, 246, 0.4);
  }

  .status-dot.speed {
    background: var(--success);
    box-shadow: 0 0 8px var(--success-glow);
  }

  .compact-count {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-muted);
    font-family: var(--font-data);
  }

  /* ── Full mode (legacy) ── */
  .device-status {
    margin-top: auto;
    padding: var(--space-lg);
    border-top: 1px solid var(--border-subtle);
  }

  .header {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    margin-bottom: var(--space-sm);
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: var(--radius-full);
    background: var(--text-faint);
    flex-shrink: 0;
    transition: all var(--transition-base);
  }

  .dot.active {
    background: var(--success);
    box-shadow: 0 0 6px var(--success-glow);
    animation: pulse-glow 2s ease-in-out infinite;
  }

  .count {
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-weight: 500;
  }

  .readings {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .reading {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    padding: 3px 0;
  }

  .reading-label {
    font-size: var(--text-xs);
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-weight: 600;
  }

  .reading-value {
    font-size: var(--text-base);
    font-family: var(--font-data);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    color: var(--text-primary);
  }

  .reading-unit {
    font-size: var(--text-xs);
    font-weight: 400;
    color: var(--text-muted);
    margin-left: 2px;
  }
</style>
