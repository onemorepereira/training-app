<script lang="ts">
  import type { SessionConfig, ZoneTarget, ZoneMode } from '$lib/tauri';
  import { resolveZoneBounds } from '$lib/stores/zoneRide';

  let {
    config,
    onStart,
    onClose,
    trainerConnected = false,
  }: {
    config: SessionConfig;
    onStart: (target: ZoneTarget) => void;
    onClose: () => void;
    trainerConnected?: boolean;
  } = $props();

  let mode: ZoneMode = $state('Power');
  let selectedZone = $state(3);
  let customLower = $state(150);
  let customUpper = $state(200);
  let durationMin = $state(20);
  let useCustom = $state(false);

  const POWER_COLORS = ['#78909c', '#4caf50', '#8bc34a', '#ffeb3b', '#ff9800', '#f44336', '#9c27b0'];
  const HR_COLORS = ['#4caf50', '#8bc34a', '#ffeb3b', '#ff9800', '#f44336'];

  let zoneCount = $derived(mode === 'Power' ? 7 : 5);
  let unit = $derived(mode === 'Power' ? 'W' : 'bpm');
  let colors = $derived(mode === 'Power' ? POWER_COLORS : HR_COLORS);

  let resolved = $derived.by(() => {
    if (useCustom) return { lower: customLower, upper: customUpper };
    return resolveZoneBounds(mode, selectedZone, config);
  });

  let zoneColor = $derived(useCustom ? 'var(--accent)' : (colors[selectedZone - 1] ?? 'var(--accent)'));

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
  <!-- Row 1: mode + zones + close -->
  <div class="builder-top">
    <div class="mode-group">
      <button class="mode-btn" class:active={mode === 'Power'} onclick={() => switchMode('Power')}>Power</button>
      <button class="mode-btn" class:active={mode === 'HeartRate'} onclick={() => switchMode('HeartRate')}>HR</button>
    </div>

    <div class="zone-selector">
      {#each Array.from({ length: zoneCount }, (_, i) => i + 1) as z}
        <button
          class="zone-chip"
          class:active={!useCustom && selectedZone === z}
          style="--zone-color: {colors[z - 1]}"
          onclick={() => selectZone(z)}
        >
          <span class="zone-dot" style="background: {colors[z - 1]}"></span>
          Z{z}
        </button>
      {/each}
      <button
        class="zone-chip custom-chip"
        class:active={useCustom}
        onclick={() => { useCustom = true; }}
      >Custom</button>
    </div>

    <button class="btn-close" onclick={onClose} aria-label="Close">
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
    </button>
  </div>

  <!-- Row 2: target + duration + start -->
  <div class="builder-bottom">
    {#if useCustom}
      <div class="target-custom">
        <input type="number" bind:value={customLower} min="0" max="500" class="range-input" />
        <span class="range-dash">&ndash;</span>
        <input type="number" bind:value={customUpper} min="0" max="500" class="range-input" />
        <span class="target-unit">{unit}</span>
      </div>
    {:else}
      <div class="target-resolved">
        <span class="target-value" style="color: {zoneColor}">{resolved.lower}<span class="target-dash">&ndash;</span>{resolved.upper}</span>
        <span class="target-unit">{unit}</span>
      </div>
    {/if}

    <div class="builder-spacer"></div>

    <div class="duration-group">
      <svg class="duration-icon" viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>
      <input type="number" bind:value={durationMin} min="0" max="240" class="duration-input" />
      <span class="duration-label">min</span>
    </div>

    <button
      class="btn-start"
      disabled={!trainerConnected || resolved.lower >= resolved.upper}
      style="--btn-color: {zoneColor}"
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
    border-top: 1px solid var(--border-subtle);
    animation: fade-in 150ms ease;
  }

  @keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  /* Row 1 */
  .builder-top {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: var(--space-sm) var(--space-lg);
    flex-wrap: wrap;
  }

  .mode-group {
    display: flex;
    gap: 2px;
    background: var(--bg-body);
    border-radius: var(--radius-md);
    padding: 2px;
    flex-shrink: 0;
  }

  .mode-btn {
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

  .mode-btn:hover { color: var(--text-secondary); }

  .mode-btn.active {
    background: var(--bg-elevated);
    color: var(--accent);
    box-shadow: var(--shadow-sm);
  }

  .zone-selector {
    display: flex;
    gap: 3px;
    flex-wrap: wrap;
  }

  .zone-chip {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 2px var(--space-sm);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-full);
    background: var(--bg-body);
    color: var(--text-secondary);
    font-family: var(--font-data);
    font-size: var(--text-xs);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .zone-chip:hover {
    border-color: var(--zone-color, var(--accent));
    color: var(--text-primary);
  }

  .zone-chip.active {
    border-color: var(--zone-color, var(--accent));
    background: color-mix(in srgb, var(--zone-color, var(--accent)) 15%, transparent);
    color: var(--text-primary);
    box-shadow: 0 0 6px color-mix(in srgb, var(--zone-color, var(--accent)) 25%, transparent);
  }

  .zone-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .custom-chip {
    font-family: var(--font-sans);
    letter-spacing: 0.02em;
  }

  .custom-chip.active {
    border-color: var(--accent);
    background: var(--accent-soft);
    box-shadow: 0 0 6px var(--accent-glow);
  }

  .btn-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    margin-left: auto;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    transition: all var(--transition-fast);
    flex-shrink: 0;
  }

  .btn-close:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  /* Row 2 */
  .builder-bottom {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: var(--space-sm) var(--space-lg);
    border-top: 1px solid var(--border-subtle);
    background: var(--bg-body);
    flex-wrap: wrap;
  }

  .target-resolved {
    display: flex;
    align-items: baseline;
    gap: var(--space-xs);
  }

  .target-value {
    font-family: var(--font-data);
    font-size: var(--text-lg);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    letter-spacing: -0.02em;
  }

  .target-dash {
    color: var(--text-muted);
    margin: 0 1px;
  }

  .target-unit {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-weight: 500;
  }

  .target-custom {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
  }

  .range-input {
    width: 52px;
    padding: 2px var(--space-xs);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-sm);
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-family: var(--font-data);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    text-align: center;
  }

  .range-input:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }

  .range-dash {
    color: var(--text-muted);
    font-weight: 600;
  }

  .builder-spacer {
    flex: 1;
  }

  .duration-group {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }

  .duration-icon {
    color: var(--text-muted);
    flex-shrink: 0;
  }

  .duration-input {
    width: 40px;
    padding: 2px var(--space-xs);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-sm);
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-family: var(--font-data);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    text-align: center;
  }

  .duration-input:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }

  .duration-label {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-weight: 500;
  }

  .btn-start {
    padding: var(--space-xs) var(--space-lg);
    border: none;
    border-radius: var(--radius-md);
    background: var(--btn-color, var(--accent));
    color: white;
    font-size: var(--text-xs);
    font-weight: 700;
    cursor: pointer;
    transition: all var(--transition-fast);
    white-space: nowrap;
    letter-spacing: 0.02em;
    box-shadow: 0 2px 6px color-mix(in srgb, var(--btn-color, var(--accent)) 30%, transparent);
    flex-shrink: 0;
  }

  .btn-start:hover:not(:disabled) {
    transform: translateY(-1px);
    box-shadow: 0 3px 10px color-mix(in srgb, var(--btn-color, var(--accent)) 40%, transparent);
  }

  .btn-start:active:not(:disabled) {
    transform: translateY(0);
  }

  .btn-start:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }
</style>
