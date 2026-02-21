<script lang="ts">
  import type { SessionConfig, ZoneTarget, ZoneMode } from '$lib/tauri';
  import { resolveZoneBounds } from '$lib/stores/zoneRide';

  let {
    config,
    onStart,
    trainerConnected = false,
  }: {
    config: SessionConfig;
    onStart: (target: ZoneTarget) => void;
    trainerConnected?: boolean;
  } = $props();

  let mode: ZoneMode = $state('Power');
  let selectedZone = $state(3);
  let customLower = $state(150);
  let customUpper = $state(200);
  let durationMin = $state(20);
  let useCustom = $state(false);

  const powerZoneCount = 7;
  const hrZoneCount = 5;

  let resolved = $derived.by(() => {
    if (useCustom) return { lower: customLower, upper: customUpper };
    return resolveZoneBounds(mode, selectedZone, config);
  });

  let zoneCount = $derived(mode === 'Power' ? powerZoneCount : hrZoneCount);
  let unit = $derived(mode === 'Power' ? 'W' : 'bpm');

  function switchMode(m: ZoneMode) {
    mode = m;
    selectedZone = 3;
    useCustom = false;
  }

  function selectZone(z: number) {
    selectedZone = z;
    useCustom = false;
  }

  function handleStart() {
    onStart({
      mode,
      zone: useCustom ? 0 : selectedZone,
      lower_bound: resolved.lower,
      upper_bound: resolved.upper,
      duration_secs: durationMin > 0 ? durationMin * 60 : null,
    });
  }
</script>

<div class="zone-builder">
  <div class="builder-row">
    <div class="mode-tabs">
      <button class="tab" class:active={mode === 'Power'} onclick={() => switchMode('Power')}>Power</button>
      <button class="tab" class:active={mode === 'HeartRate'} onclick={() => switchMode('HeartRate')}>HR</button>
    </div>

    <div class="zone-btns">
      {#each Array.from({ length: zoneCount }, (_, i) => i + 1) as z}
        <button
          class="zone-btn"
          class:active={!useCustom && selectedZone === z}
          onclick={() => selectZone(z)}
        >Z{z}</button>
      {/each}
      <button class="zone-btn" class:active={useCustom} onclick={() => { useCustom = true; }}>Custom</button>
    </div>
  </div>

  <div class="builder-row">
    {#if useCustom}
      <div class="custom-inputs">
        <label class="custom-field">
          <span class="field-label">Min</span>
          <input type="number" bind:value={customLower} min="0" max="500" class="num-input" />
          <span class="field-unit">{unit}</span>
        </label>
        <span class="range-sep">&ndash;</span>
        <label class="custom-field">
          <span class="field-label">Max</span>
          <input type="number" bind:value={customUpper} min="0" max="500" class="num-input" />
          <span class="field-unit">{unit}</span>
        </label>
      </div>
    {:else}
      <div class="resolved-bounds">
        <span class="bounds-value">{resolved.lower}&ndash;{resolved.upper}</span>
        <span class="bounds-unit">{unit}</span>
      </div>
    {/if}

    <div class="duration-field">
      <input type="number" bind:value={durationMin} min="0" max="240" class="num-input dur" />
      <span class="field-unit">min</span>
    </div>

    <button
      class="btn-start-zone"
      disabled={!trainerConnected || resolved.lower >= resolved.upper}
      onclick={handleStart}
    >
      Start Zone
    </button>
  </div>
</div>

<style>
  .zone-builder {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-sm) 0;
  }

  .builder-row {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    flex-wrap: wrap;
  }

  .mode-tabs {
    display: flex;
    gap: 2px;
    background: var(--bg-body);
    border-radius: var(--radius-md);
    padding: 2px;
    flex-shrink: 0;
  }

  .tab {
    padding: var(--space-xs) var(--space-md);
    border: none;
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-xs);
    font-weight: 700;
    cursor: pointer;
    border-radius: 5px;
    transition: all var(--transition-fast);
    letter-spacing: 0.04em;
  }

  .tab:hover {
    color: var(--text-secondary);
  }

  .tab.active {
    background: var(--bg-elevated);
    color: var(--accent);
    box-shadow: var(--shadow-sm);
  }

  .zone-btns {
    display: flex;
    gap: 3px;
  }

  .zone-btn {
    padding: var(--space-xs) var(--space-sm);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-sm);
    background: var(--bg-body);
    color: var(--text-secondary);
    font-family: var(--font-data);
    font-size: var(--text-xs);
    font-weight: 600;
    cursor: pointer;
    min-width: 32px;
    text-align: center;
    transition: all var(--transition-fast);
  }

  .zone-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }

  .zone-btn.active {
    background: var(--accent);
    color: white;
    border-color: var(--accent);
  }

  .resolved-bounds {
    display: flex;
    align-items: baseline;
    gap: var(--space-xs);
  }

  .bounds-value {
    font-family: var(--font-data);
    font-size: var(--text-lg);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    color: var(--text-primary);
  }

  .bounds-unit {
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-weight: 500;
  }

  .custom-inputs {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
  }

  .custom-field {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .field-label {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-weight: 500;
  }

  .field-unit {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-weight: 500;
  }

  .range-sep {
    color: var(--text-muted);
    font-weight: 600;
  }

  .num-input {
    width: 56px;
    padding: var(--space-xs) var(--space-sm);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-sm);
    background: var(--bg-body);
    color: var(--text-primary);
    font-family: var(--font-data);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    text-align: center;
  }

  .num-input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .num-input.dur {
    width: 48px;
  }

  .duration-field {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .btn-start-zone {
    padding: var(--space-sm) var(--space-lg);
    border: none;
    border-radius: var(--radius-md);
    background: var(--accent);
    color: white;
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--transition-fast);
    white-space: nowrap;
  }

  .btn-start-zone:hover:not(:disabled) {
    transform: translateY(-1px);
    box-shadow: 0 2px 8px rgba(var(--accent-rgb, 33, 150, 243), 0.3);
  }

  .btn-start-zone:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
