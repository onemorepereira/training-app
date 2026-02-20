<script lang="ts">
  import { onMount } from 'svelte';
  import type { SessionConfig } from '$lib/tauri';
  import { api, extractError } from '$lib/tauri';
  import { unitSystem, displayWeight, toStorageWeight } from '$lib/stores/units';

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
  let weightDisplay = $state(75.0);
  let saved = $state(false);
  let error = $state('');

  onMount(async () => {
    try {
      config = await api.getUserConfig();
      weightDisplay = displayWeight(config.weight_kg, config.units);
      unitSystem.set(config.units);
    } catch (e) {
      error = extractError(e);
    }
  });

  function onUnitsChange(units: 'metric' | 'imperial') {
    config.units = units;
    unitSystem.set(units);
    weightDisplay = displayWeight(config.weight_kg, units);
  }

  async function save() {
    error = '';
    saved = false;
    try {
      config.weight_kg = toStorageWeight(weightDisplay, config.units);
      await api.saveUserConfig(config);
      saved = true;
      setTimeout(() => (saved = false), 2000);
    } catch (e) {
      error = extractError(e);
    }
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

  const hrZoneColors = ['#4caf50', '#8bc34a', '#ffeb3b', '#ff9800', '#f44336'];
  const powerZoneColors = ['#78909c', '#4caf50', '#8bc34a', '#ffeb3b', '#ff9800', '#f44336', '#9c27b0'];
  const powerZoneLabels = ['Active Recovery', 'Endurance', 'Tempo', 'Threshold', 'VO2max', 'Anaerobic', 'Neuromuscular'];
</script>

<div class="page">
  <h1>Settings</h1>

  {#if error}
    <div class="error">{error}</div>
  {/if}

  <div class="settings-form">
    <section class="section">
      <h2 class="section-title">Units</h2>
      <div class="unit-toggle">
        <button
          class="unit-btn"
          class:active={config.units === 'metric'}
          onclick={() => onUnitsChange('metric')}
        >Metric</button>
        <button
          class="unit-btn"
          class:active={config.units === 'imperial'}
          onclick={() => onUnitsChange('imperial')}
        >Imperial</button>
      </div>
    </section>

    <section class="section">
      <h2 class="section-title">Athlete Profile</h2>
      <div class="field-group">
        <div class="field">
          <label for="ftp">FTP</label>
          <div class="input-wrap">
            <input id="ftp" type="number" bind:value={config.ftp} min="50" max="500" />
            <span class="input-unit">W</span>
          </div>
        </div>
        <div class="field">
          <label for="weight">Weight</label>
          <div class="input-wrap">
            <input id="weight" type="number" bind:value={weightDisplay} min="30" max={config.units === 'imperial' ? 440 : 200} step="0.1" />
            <span class="input-unit">{config.units === 'imperial' ? 'lbs' : 'kg'}</span>
          </div>
        </div>
        <div class="field">
          <label for="dob">Date of Birth</label>
          <input id="dob" type="date" bind:value={config.date_of_birth} class="date-input" />
        </div>
        <div class="field">
          <label for="sex">Sex</label>
          <div class="sex-toggle">
            <button class="sex-btn" class:active={config.sex === 'male'} onclick={() => config.sex = 'male'}>Male</button>
            <button class="sex-btn" class:active={config.sex === 'female'} onclick={() => config.sex = 'female'}>Female</button>
          </div>
        </div>
        <div class="field">
          <label for="resting-hr">Resting HR</label>
          <div class="input-wrap">
            <input id="resting-hr" type="number" bind:value={config.resting_hr} min="30" max="120" />
            <span class="input-unit">bpm</span>
          </div>
        </div>
        <div class="field">
          <label for="max-hr">Max HR</label>
          <div class="input-wrap">
            <input id="max-hr" type="number" bind:value={config.max_hr} min="120" max="230" />
            <span class="input-unit">bpm</span>
          </div>
        </div>
      </div>
    </section>

    <div class="zones-grid">
      <section class="section">
        <div class="section-header">
          <div>
            <h2 class="section-title">Heart Rate Zones</h2>
            <p class="section-hint">Upper bound of each zone in bpm</p>
          </div>
          {#if canEstimateHrZones}
            <button class="estimate-btn" onclick={estimateHrZones}>Estimate from rHR/Max</button>
          {/if}
        </div>
        <div class="zones">
          {#each config.hr_zones as zone, i}
            <div class="zone-field">
              <div class="zone-indicator" style="background: {hrZoneColors[i]}"></div>
              <label for="hr-zone-{i}">Z{i + 1}</label>
              <input id="hr-zone-{i}" type="number" bind:value={config.hr_zones[i]} min="60" max="220" />
            </div>
          {/each}
        </div>
      </section>

      <section class="section">
        <h2 class="section-title">Power Zones</h2>
        <p class="section-hint">Upper bound as % of FTP ({config.ftp}W)</p>
        <div class="zones">
          {#each config.power_zones as pz, i}
            <div class="zone-field">
              <div class="zone-indicator" style="background: {powerZoneColors[i]}"></div>
              <label for="pz-{i}">Z{i + 1}</label>
              <span class="zone-name">{powerZoneLabels[i]}</span>
              <div class="zone-input-group">
                <input id="pz-{i}" type="number" bind:value={config.power_zones[i]} min="1" max="300" />
                <span class="zone-pct">%</span>
                <span class="zone-watts">{powerWatts(config.power_zones[i])}W</span>
              </div>
            </div>
          {/each}
          <div class="zone-field zone-7">
            <div class="zone-indicator" style="background: {powerZoneColors[6]}"></div>
            <span class="zone-label">Z7</span>
            <span class="zone-name">{powerZoneLabels[6]}</span>
            <span class="zone-watts-hint">&gt; {powerWatts(config.power_zones[5])}W</span>
          </div>
        </div>
      </section>
    </div>

    <button class="save-btn" class:saved onclick={save}>
      {saved ? 'Saved' : 'Save Settings'}
    </button>
  </div>
</div>

<style>
  .page {
    max-width: 960px;
  }

  h1 {
    margin: 0 0 var(--space-xl);
    font-size: var(--text-2xl);
    font-weight: 800;
  }

  .error {
    margin-bottom: var(--space-lg);
    padding: var(--space-md);
    background: rgba(244, 67, 54, 0.08);
    border: 1px solid rgba(244, 67, 54, 0.3);
    border-radius: var(--radius-md);
    color: var(--danger);
    font-size: var(--text-base);
    animation: slide-up 200ms ease;
  }

  .section {
    margin-bottom: var(--space-3xl);
  }

  .section-title {
    font-size: var(--text-sm);
    font-weight: 700;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin: 0 0 var(--space-md);
  }

  .section-hint {
    font-size: var(--text-sm);
    color: var(--text-faint);
    margin: -0.25rem 0 var(--space-md);
  }

  .unit-toggle {
    display: flex;
    gap: 2px;
    background: var(--bg-body);
    border-radius: var(--radius-md);
    padding: 3px;
    width: fit-content;
  }

  .unit-btn {
    padding: var(--space-sm) var(--space-lg);
    border: none;
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
    border-radius: 6px;
    transition: all var(--transition-fast);
  }

  .unit-btn:hover {
    color: var(--text-secondary);
  }

  .unit-btn.active {
    background: var(--bg-elevated);
    color: var(--accent);
    box-shadow: var(--shadow-sm);
  }

  .field-group {
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    max-width: 480px;
  }

  .field {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-md);
    background: var(--bg-surface);
    border-radius: var(--radius-md);
    border: 1px solid var(--border-subtle);
  }

  .field label {
    color: var(--text-secondary);
    font-size: var(--text-base);
    font-weight: 500;
  }

  .input-wrap {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }

  .input-unit {
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-weight: 500;
    min-width: 20px;
  }

  .date-input {
    width: auto;
    text-align: left;
    color-scheme: dark;
  }

  .sex-toggle {
    display: flex;
    gap: 2px;
    background: var(--bg-body);
    border-radius: var(--radius-md);
    padding: 3px;
  }

  .sex-btn {
    padding: var(--space-xs) var(--space-md);
    border: none;
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
    border-radius: 5px;
    transition: all var(--transition-fast);
  }

  .sex-btn:hover {
    color: var(--text-secondary);
  }

  .sex-btn.active {
    background: var(--bg-elevated);
    color: var(--accent);
    box-shadow: var(--shadow-sm);
  }

  input {
    width: 80px;
    padding: var(--space-sm);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-md);
    background: var(--bg-input);
    color: var(--text-primary);
    font-family: var(--font-data);
    font-size: var(--text-base);
    font-weight: 600;
    text-align: right;
    transition: border-color var(--transition-fast);
    -moz-appearance: textfield;
    appearance: textfield;
  }

  input::-webkit-outer-spin-button,
  input::-webkit-inner-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }

  input:focus {
    border-color: var(--accent);
    outline: none;
  }

  .section-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-md);
  }

  .estimate-btn {
    padding: var(--space-xs) var(--space-md);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
    color: var(--text-secondary);
    font-size: var(--text-xs);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--transition-fast);
    white-space: nowrap;
    flex-shrink: 0;
    margin-top: 2px;
  }

  .estimate-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
    background: var(--accent-soft);
  }

  .zones-grid {
    display: grid;
    grid-template-columns: 1fr;
    gap: var(--space-lg);
  }

  @media (min-width: 700px) {
    .zones-grid {
      grid-template-columns: 1fr 1fr;
    }

    .zones-grid .section {
      margin-bottom: 0;
    }
  }

  .zones {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }

  .zone-field {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: var(--space-sm) var(--space-md);
    background: var(--bg-surface);
    border-radius: var(--radius-md);
    border: 1px solid var(--border-subtle);
  }

  .zone-indicator {
    width: 4px;
    height: 20px;
    border-radius: var(--radius-full);
    flex-shrink: 0;
  }

  .zone-field label,
  .zone-label {
    color: var(--text-secondary);
    font-size: var(--text-sm);
    font-weight: 600;
    min-width: 24px;
  }

  .zone-field input {
    width: 70px;
    margin-left: auto;
  }

  .zone-name {
    font-size: var(--text-xs);
    color: var(--text-faint);
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .zone-input-group {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    margin-left: auto;
    flex-shrink: 0;
  }

  .zone-input-group input {
    margin-left: 0;
  }

  .zone-pct {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-weight: 500;
  }

  .zone-watts {
    font-family: var(--font-data);
    font-size: var(--text-xs);
    color: var(--text-faint);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    min-width: 40px;
    text-align: right;
  }

  .zone-7 {
    opacity: 0.7;
  }

  .zone-watts-hint {
    font-family: var(--font-data);
    font-size: var(--text-xs);
    color: var(--text-faint);
    font-weight: 600;
    margin-left: auto;
  }

  .save-btn {
    padding: var(--space-md) var(--space-2xl);
    border: none;
    border-radius: var(--radius-lg);
    font-size: var(--text-base);
    font-weight: 600;
    cursor: pointer;
    background: var(--accent);
    color: white;
    transition: all var(--transition-fast);
    box-shadow: 0 2px 8px var(--accent-glow);
  }

  .save-btn:hover {
    transform: translateY(-1px);
    box-shadow: 0 4px 12px var(--accent-glow);
  }

  .save-btn:active {
    transform: translateY(0);
  }

  .save-btn.saved {
    background: var(--success);
    box-shadow: 0 2px 8px rgba(76, 175, 80, 0.3);
  }
</style>
