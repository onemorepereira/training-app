<script lang="ts">
  import type { SessionConfig, ZoneTarget, ZoneMode } from '$lib/tauri';
  import { resolveZoneBounds } from '$lib/stores/zoneRide';
  import { POWER_ZONE_COLORS, HR_ZONE_COLORS } from '$lib/charts/theme';
  import { nextIndex } from '$lib/ui/segmented';
  import Dialog from '$lib/ui/Dialog.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';

  /** "Start a zone ride" dialog. Its controls are ride controls, so they're 44px. */
  let {
    open = $bindable(false),
    config,
    onStart,
    onClose,
    trainerConnected = false,
  }: {
    open?: boolean;
    config: SessionConfig;
    onStart: (target: ZoneTarget) => void;
    onClose: () => void;
    trainerConnected?: boolean;
  } = $props();

  const MODES = [
    { value: 'Power', label: 'Power' },
    { value: 'HeartRate', label: 'Heart rate' },
  ] as const;

  let mode: ZoneMode = $state('Power');
  let selectedZone = $state(3);
  let customLower = $state(150);
  let customUpper = $state(200);
  let durationMin = $state(20);
  let useCustom = $state(false);


  let zoneCount = $derived(mode === 'Power' ? 7 : 5);
  let unit = $derived(mode === 'Power' ? 'W' : 'bpm');
  let colors: readonly string[] = $derived(mode === 'Power' ? POWER_ZONE_COLORS : HR_ZONE_COLORS);
  /** Zone options plus Custom (index zoneCount). */
  let choice = $derived(useCustom ? zoneCount : selectedZone - 1);
  let chipEls: HTMLButtonElement[] = $state([]);

  function boundsFor(z: number) {
    return resolveZoneBounds(mode, z, config);
  }

  function choose(i: number) {
    if (i === zoneCount) useCustom = true;
    else selectZone(i + 1);
  }

  function onChipKey(e: KeyboardEvent) {
    const i = nextIndex(e.key, choice, zoneCount + 1);
    if (i === null) return;
    e.preventDefault();
    choose(i);
    chipEls[i]?.focus();
  }

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

<Dialog bind:open title="Start a zone ride" description="The trainer adjusts to keep you in the zone you pick." onclose={onClose} width="lg">
  <div class="builder">
    <div class="field">
      <span class="field-label" id="zr-mode">Hold</span>
      <SegmentedControl size="lg" label="Zone type" options={MODES} value={mode} onchange={(m) => switchMode(m)} data-ride-control />
    </div>

    <div class="field">
      <span class="field-label" id="zr-zone">Zone</span>
      <div class="zone-grid" role="radiogroup" aria-labelledby="zr-zone" tabindex="-1" onkeydown={onChipKey}>
        {#each Array.from({ length: zoneCount }, (_, i) => i + 1) as z, i (z)}
          {@const b = boundsFor(z)}
          <button
            bind:this={chipEls[i]}
            type="button"
            class="btn btn-lg zone-chip"
            role="radio"
            aria-checked={choice === i}
            tabindex={choice === i ? 0 : -1}
            data-ride-control
            style="--zone-color: {colors[i]}"
            onclick={() => choose(i)}
          >
            <span class="zone-dot" aria-hidden="true"></span>
            <span class="zone-name">Z{z}</span>
            <span class="zone-range data">{b.lower}–{b.upper} {unit}</span>
          </button>
        {/each}
        <button
          bind:this={chipEls[zoneCount]}
          type="button"
          class="btn btn-lg zone-chip"
          role="radio"
          aria-checked={useCustom}
          tabindex={useCustom ? 0 : -1}
          data-ride-control
          onclick={() => choose(zoneCount)}
        >Custom</button>
      </div>
    </div>

    <div class="row">
      {#if useCustom}
        <div class="field">
          <span class="field-label">Range ({unit})</span>
          <div class="range">
            <input type="number" bind:value={customLower} min="0" max="2000" class="num data" aria-label="Lower bound" data-ride-control />
            <span aria-hidden="true">–</span>
            <input type="number" bind:value={customUpper} min="0" max="2000" class="num data" aria-label="Upper bound" data-ride-control />
          </div>
        </div>
      {:else}
        <div class="field">
          <span class="field-label">Target</span>
          <span class="target data" style="color: {zoneColor}">{resolved.lower}–{resolved.upper}<span class="target-unit">{unit}</span></span>
        </div>
      {/if}

      <label class="field">
        <span class="field-label">Duration (min, 0 = open-ended)</span>
        <input type="number" bind:value={durationMin} min="0" max="240" class="num data" data-ride-control />
      </label>
    </div>

    {#if resolved.lower >= resolved.upper}
      <p class="hint error">The lower bound must be below the upper bound.</p>
    {:else if !trainerConnected}
      <p class="hint">Connect a smart trainer to start a zone ride.</p>
    {/if}
  </div>

  {#snippet footer()}
    <button type="button" class="btn btn-lg btn-ghost" data-ride-control onclick={() => (open = false)}>Cancel</button>
    <button
      type="button"
      class="btn btn-lg btn-primary"
      data-ride-control
      disabled={!trainerConnected || resolved.lower >= resolved.upper}
      onclick={handleStart}
    >Start zone ride</button>
  {/snippet}
</Dialog>

<style>
  .builder {
    display: flex;
    flex-direction: column;
    gap: var(--space-lg);
  }

  .zone-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: var(--space-sm);
  }

  .zone-grid:focus {
    outline: none;
  }

  .zone-chip {
    justify-content: flex-start;
    padding: 0 var(--space-md);
    font-size: var(--text-sm);
  }

  .zone-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--zone-color, var(--accent));
    flex-shrink: 0;
  }

  .zone-name {
    font-weight: 700;
  }

  .zone-range {
    margin-left: auto;
    color: var(--text-secondary);
    font-size: var(--text-xs);
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xl);
    align-items: flex-end;
  }

  .range {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }

  .num {
    width: 96px;
    min-height: var(--control-h-lg);
  }

  .target {
    font-size: 1.75rem;
    font-weight: 700;
    line-height: var(--control-h-lg);
  }

  .target-unit {
    margin-left: 6px;
    font-size: 0.5em;
    color: var(--text-muted);
  }

  .hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }

  .hint.error {
    color: var(--danger-text);
  }
</style>
