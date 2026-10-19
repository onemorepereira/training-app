<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import type { SessionConfig } from '$lib/tauri';
  import { api, extractError } from '$lib/tauri';
  import { unitSystem, displayWeight, toStorageWeight, convertDisplayedWeight } from '$lib/stores/units';
  import { configEquals } from '$lib/utils/dirty';
  import { POWER_ZONE_COLORS, HR_ZONE_COLORS, POWER_ZONE_NAMES, HR_ZONE_NAMES } from '$lib/charts/theme';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import StateView from '$lib/ui/StateView.svelte';
  import Notice from '$lib/ui/Notice.svelte';

  const clone = (c: SessionConfig): SessionConfig => ({ ...c, hr_zones: [...c.hr_zones], power_zones: [...c.power_zones] });

  let config = $state<SessionConfig>({
    ftp: 200,
    weight_kg: 75.0,
    hr_zones: [120, 140, 160, 175, 190],
    units: 'metric',
    power_zones: [55, 75, 90, 105, 120, 150],
    date_of_birth: null,
    sex: null,
    resting_hr: null,
    max_hr: null,
  });
  /** The last loaded or saved values; the form is dirty when it differs. */
  let savedConfig = $state.raw<SessionConfig | null>(null);
  let weightDisplay = $state(75.0);
  let loading = $state(true);
  let loadError = $state('');
  let saveError = $state('');
  let saving = $state(false);
  let savedNotice = $state(false);
  let savedTimer: ReturnType<typeof setTimeout> | null = null;

  let dirty = $derived(savedConfig != null && !configEquals(savedConfig, config, weightDisplay));

  const UNITS = [
    { value: 'metric', label: 'Metric' },
    { value: 'imperial', label: 'Imperial' },
  ] as const;
  const SEXES = [
    { value: 'male', label: 'Male' },
    { value: 'female', label: 'Female' },
  ] as const;

  async function load() {
    loading = true;
    loadError = '';
    try {
      const c = await api.getUserConfig();
      config = clone(c);
      savedConfig = clone(c);
      weightDisplay = displayWeight(c.weight_kg, c.units);
      unitSystem.set(c.units);
    } catch (e) {
      loadError = extractError(e);
    } finally {
      loading = false;
    }
  }

  onMount(load);
  onDestroy(() => {
    if (savedTimer) clearTimeout(savedTimer);
  });

  // Converts the typed (possibly unsaved) weight; the app-wide unit changes only on Save.
  function onUnitsChange(units: 'metric' | 'imperial') {
    weightDisplay = convertDisplayedWeight(weightDisplay, config.units, units);
    config.units = units;
  }

  async function save() {
    if (saving) return;
    saveError = '';
    saving = true;
    try {
      const next = clone(config);
      next.weight_kg = toStorageWeight(weightDisplay, config.units);
      await api.saveUserConfig(next);
      config.weight_kg = next.weight_kg;
      savedConfig = next;
      unitSystem.set(next.units);
      savedNotice = true;
      if (savedTimer) clearTimeout(savedTimer);
      savedTimer = setTimeout(() => (savedNotice = false), 3000);
    } catch (e) {
      saveError = extractError(e);
    } finally {
      saving = false;
    }
  }

  function discard() {
    if (!savedConfig) return;
    config = clone(savedConfig);
    weightDisplay = displayWeight(savedConfig.weight_kg, savedConfig.units);
    saveError = '';
  }

  function powerWatts(zonePercent: number): number {
    return Math.round((zonePercent / 100) * config.ftp);
  }

  let canEstimateHrZones = $derived(config.resting_hr != null && config.max_hr != null);

  // Karvonen formula: zone upper bound = %HRR * (MaxHR - rHR) + rHR
  // Z1: 60%, Z2: 70%, Z3: 80%, Z4: 90%, Z5: 100%
  function estimateHrZones() {
    if (config.resting_hr == null || config.max_hr == null) return;
    const rhr = config.resting_hr;
    const hrr = config.max_hr - rhr;
    const pcts = [0.60, 0.70, 0.80, 0.90, 1.00];
    config.hr_zones = pcts.map((p) => Math.round(p * hrr + rhr)) as typeof config.hr_zones;
  }
</script>

<div class="page" data-page="settings">
  <header class="page-header">
    <h1 class="page-title">Settings</h1>
  </header>

  {#if loading}
    <StateView kind="loading" title="Loading your settings…" />
  {:else if loadError}
    <StateView
      kind="error"
      title="Couldn't load your settings"
      message="Nothing was changed. Try again; if it keeps failing, restart the app."
      details={loadError}
      action={{ label: 'Retry', onclick: load }}
    />
  {:else}
    {#if savedNotice}
      <div class="notice-slot"><Notice tone="success" ondismiss={() => (savedNotice = false)}>Settings saved</Notice></div>
    {/if}

    <div class="columns">
      <section class="card" aria-labelledby="profile-h">
        <h2 class="section-title" id="profile-h">Athlete profile</h2>

        <div class="fields">
          <div class="field">
            <span class="field-label" id="units-l">Units</span>
            <SegmentedControl label="Units" options={UNITS} value={config.units} onchange={(u) => onUnitsChange(u)} />
          </div>

          <label class="field">
            <span class="field-label">FTP</span>
            <span class="input-wrap">
              <input type="number" class="data" bind:value={config.ftp} min="50" max="500" />
              <span class="input-unit">W</span>
            </span>
          </label>

          <label class="field">
            <span class="field-label">Weight</span>
            <span class="input-wrap">
              <input type="number" class="data" bind:value={weightDisplay} min="30" max={config.units === 'imperial' ? 440 : 200} step="0.1" />
              <span class="input-unit">{config.units === 'imperial' ? 'lbs' : 'kg'}</span>
            </span>
          </label>

          <label class="field">
            <span class="field-label">Date of birth</span>
            <input type="date" class="date" bind:value={config.date_of_birth} />
          </label>

          <div class="field">
            <span class="field-label">Sex</span>
            <SegmentedControl label="Sex" options={SEXES} value={config.sex ?? ''} onchange={(v) => (config.sex = v)} />
          </div>

          <label class="field">
            <span class="field-label">Resting heart rate</span>
            <span class="input-wrap">
              <input type="number" class="data" bind:value={config.resting_hr} min="30" max="120" />
              <span class="input-unit">bpm</span>
            </span>
          </label>

          <label class="field">
            <span class="field-label">Max heart rate</span>
            <span class="input-wrap">
              <input type="number" class="data" bind:value={config.max_hr} min="120" max="230" />
              <span class="input-unit">bpm</span>
            </span>
          </label>
        </div>
      </section>

      <div class="zones-col">
        <section class="card" aria-labelledby="pz-h">
          <h2 class="section-title" id="pz-h">Power zones</h2>
          <p class="hint">Top of each zone as a % of FTP (<span class="data">{config.ftp} W</span>).</p>
          <ul class="zones">
            {#each config.power_zones as _, i (i)}
              <li class="zone-row">
                <span class="swatch" style="background: {POWER_ZONE_COLORS[i]}" aria-hidden="true"></span>
                <label class="zone-name" for="pz-{i}">Z{i + 1} · {POWER_ZONE_NAMES[i]}</label>
                <span class="input-wrap zone-input">
                  <input id="pz-{i}" type="number" class="data" bind:value={config.power_zones[i]} min="1" max="300" />
                  <span class="input-unit">%</span>
                </span>
                <span class="zone-watts data">≤ {powerWatts(config.power_zones[i])} W</span>
              </li>
            {/each}
            <li class="zone-row">
              <span class="swatch" style="background: {POWER_ZONE_COLORS[6]}" aria-hidden="true"></span>
              <span class="zone-name">Z7 · {POWER_ZONE_NAMES[6]}</span>
              <span class="zone-input"></span>
              <span class="zone-watts data">&gt; {powerWatts(config.power_zones[5])} W</span>
            </li>
          </ul>
        </section>

        <section class="card" aria-labelledby="hz-h">
          <div class="card-head">
            <div>
              <h2 class="section-title" id="hz-h">Heart rate zones</h2>
              <p class="hint">Top of each zone in bpm.</p>
            </div>
            <button
              type="button"
              class="btn btn-secondary"
              disabled={!canEstimateHrZones}
              title={canEstimateHrZones ? undefined : 'Enter resting and max heart rate first'}
              onclick={estimateHrZones}
            >Estimate from resting/max HR</button>
          </div>
          <ul class="zones">
            {#each config.hr_zones as _, i (i)}
              <li class="zone-row">
                <span class="swatch" style="background: {HR_ZONE_COLORS[i]}" aria-hidden="true"></span>
                <label class="zone-name" for="hz-{i}">Z{i + 1} · {HR_ZONE_NAMES[i]}</label>
                <span class="input-wrap zone-input">
                  <input id="hz-{i}" type="number" class="data" bind:value={config.hr_zones[i]} min="60" max="220" />
                  <span class="input-unit">bpm</span>
                </span>
                <span class="zone-watts"></span>
              </li>
            {/each}
          </ul>
        </section>
      </div>
    </div>

    {#if dirty || saveError}
      <div class="save-bar" role="region" aria-label="Unsaved changes">
        {#if saveError}
          <Notice tone="error" title="Couldn't save settings" ondismiss={() => (saveError = '')}>{saveError} Your edits are kept.</Notice>
        {/if}
        <div class="save-row">
          <span class="save-text">{dirty ? 'Unsaved changes' : 'No changes'}</span>
          <button type="button" class="btn btn-ghost" disabled={!dirty || saving} onclick={discard}>Discard</button>
          <button type="button" class="btn btn-primary" disabled={!dirty || saving} onclick={save}>{saving ? 'Saving…' : 'Save'}</button>
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .notice-slot {
    margin-bottom: var(--space-lg);
  }

  .columns {
    display: grid;
    grid-template-columns: 1fr;
    gap: var(--space-xl);
    align-items: start;
  }

  @media (min-width: 1280px) {
    .columns {
      grid-template-columns: minmax(320px, 2fr) 3fr;
    }
  }

  .zones-col {
    display: flex;
    flex-direction: column;
    gap: var(--space-xl);
  }

  .card {
    padding: var(--space-xl);
    border-radius: var(--radius-lg);
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
  }

  .card-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: var(--space-md);
  }

  .hint {
    margin: calc(-1 * var(--space-sm)) 0 var(--space-lg);
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }

  .fields {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: var(--space-lg);
  }

  .input-wrap {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }

  .input-wrap input {
    width: 100%;
    min-width: 0;
  }

  .input-unit {
    min-width: 2.5em;
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }

  .date {
    width: 100%;
  }

  .zones {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }

  .zone-row {
    display: grid;
    grid-template-columns: 12px minmax(0, 1fr) 128px 96px;
    align-items: center;
    gap: var(--space-md);
    min-height: var(--control-h);
  }

  .swatch {
    width: 12px;
    height: 12px;
    border-radius: 3px;
  }

  .zone-name {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-primary);
  }

  .zone-watts {
    font-size: var(--text-sm);
    color: var(--text-secondary);
    text-align: right;
  }

  .save-bar {
    position: sticky;
    bottom: 0;
    z-index: 10;
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    margin-top: var(--space-xl);
    padding: var(--space-md) var(--space-lg);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-lg);
    background: var(--bg-elevated);
    box-shadow: var(--shadow-lg);
    animation: slide-up 160ms ease;
  }

  .save-row {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }

  .save-text {
    margin-right: auto;
    font-weight: 600;
  }
</style>
