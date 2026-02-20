<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { trainerState, trainerError, type TrainerMode, type TrainerState } from '$lib/stores/trainer';
  import { currentPower } from '$lib/stores/sensor';
  import { api, extractError } from '$lib/tauri';

  let local: TrainerState = $state({
    mode: 'erg',
    ergTarget: 150,
    resistanceLevel: 50,
    simGrade: 0,
    simCrr: 0.004,
    simCw: 0.51,
  });
  let sending = $state(false);
  let sendTimer: ReturnType<typeof setTimeout> | null = null;
  let errorTimer: ReturnType<typeof setTimeout> | null = null;

  const unsubscribe = trainerState.subscribe((v) => { local = { ...v }; });
  onDestroy(() => {
    unsubscribe();
    if (sendTimer) clearTimeout(sendTimer);
    if (errorTimer) clearTimeout(errorTimer);
  });

  function setError(msg: string) {
    trainerError.set(msg);
    if (errorTimer) clearTimeout(errorTimer);
    errorTimer = setTimeout(() => { trainerError.set(''); }, 5000);
  }

  function save() {
    trainerState.set({ ...local });
  }

  function setMode(mode: TrainerMode) {
    local.mode = mode;
    save();
    if (mode === 'erg') sendErg();
    else if (mode === 'resistance') sendResistance();
    else sendSimulation();
  }

  async function sendErg() {
    if (sending) return;
    sending = true;
    trainerError.set('');
    try {
      await api.setTrainerPower(local.ergTarget);
    } catch (e) {
      setError(extractError(e));
    } finally {
      sending = false;
    }
  }

  async function sendResistance() {
    if (sending) return;
    sending = true;
    trainerError.set('');
    try {
      await api.setTrainerResistance(local.resistanceLevel);
    } catch (e) {
      setError(extractError(e));
    } finally {
      sending = false;
    }
  }

  async function sendSimulation() {
    if (sending) return;
    sending = true;
    trainerError.set('');
    try {
      await api.setTrainerSimulation(local.simGrade, local.simCrr, local.simCw);
    } catch (e) {
      setError(extractError(e));
    } finally {
      sending = false;
    }
  }

  function throttledSend(fn: () => void) {
    save();
    if (sendTimer) return;
    fn();
    sendTimer = setTimeout(() => { sendTimer = null; }, 150);
  }

  function adjustErg(delta: number) {
    local.ergTarget = Math.max(0, Math.min(2000, local.ergTarget + delta));
    save();
    sendErg();
  }

  function adjustResistance(delta: number) {
    local.resistanceLevel = Math.max(0, Math.min(100, local.resistanceLevel + delta));
    save();
    sendResistance();
  }

  function adjustGrade(delta: number) {
    local.simGrade = Math.max(-20, Math.min(20, +(local.simGrade + delta).toFixed(1)));
    save();
    sendSimulation();
  }

  function handleKeydown(e: KeyboardEvent) {
    if ((e.target as HTMLElement)?.tagName === 'INPUT') return;

    const big = e.shiftKey;
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      if (local.mode === 'erg') adjustErg(big ? 25 : 5);
      else if (local.mode === 'resistance') adjustResistance(big ? 10 : 1);
      else adjustGrade(big ? 1 : 0.1);
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      if (local.mode === 'erg') adjustErg(big ? -25 : -5);
      else if (local.mode === 'resistance') adjustResistance(big ? -10 : -1);
      else adjustGrade(big ? -1 : -0.1);
    }
  }

  onMount(() => {
    window.addEventListener('keydown', handleKeydown);
    return () => window.removeEventListener('keydown', handleKeydown);
  });
</script>

<div class="trainer-strip">
  <div class="strip-row">
    <div class="mode-tabs">
      <button class="tab" class:active={local.mode === 'erg'} onclick={() => setMode('erg')}>ERG</button>
      <button class="tab" class:active={local.mode === 'resistance'} onclick={() => setMode('resistance')}>RES</button>
      <button class="tab" class:active={local.mode === 'simulation'} onclick={() => setMode('simulation')}>SIM</button>
    </div>

    <div class="strip-divider"></div>

    {#if local.mode === 'erg'}
      <div class="strip-target">
        <span class="target-value">{local.ergTarget}<span class="target-unit">W</span></span>
        {#if $currentPower != null}
          <span class="actual-value">{$currentPower}W</span>
        {/if}
      </div>
      <div class="strip-adjust">
        <button class="adj-btn" onclick={() => adjustErg(-25)}>-25</button>
        <button class="adj-btn" onclick={() => adjustErg(-5)}>-5</button>
        <button class="adj-btn" onclick={() => adjustErg(5)}>+5</button>
        <button class="adj-btn" onclick={() => adjustErg(25)}>+25</button>
      </div>
    {:else if local.mode === 'resistance'}
      <div class="strip-target">
        <span class="target-value">{local.resistanceLevel}<span class="target-unit">%</span></span>
      </div>
      <div class="strip-adjust">
        <button class="adj-btn" onclick={() => adjustResistance(-10)}>-10</button>
        <button class="adj-btn" onclick={() => adjustResistance(-1)}>-1</button>
        <button class="adj-btn" onclick={() => adjustResistance(1)}>+1</button>
        <button class="adj-btn" onclick={() => adjustResistance(10)}>+10</button>
      </div>
    {:else}
      <div class="strip-target">
        <span class="target-value">{local.simGrade.toFixed(1)}<span class="target-unit">%</span></span>
      </div>
      <div class="strip-adjust">
        <button class="adj-btn" onclick={() => adjustGrade(-1)}>-1</button>
        <button class="adj-btn" onclick={() => adjustGrade(-0.1)}>-.1</button>
        <button class="adj-btn" onclick={() => adjustGrade(0.1)}>+.1</button>
        <button class="adj-btn" onclick={() => adjustGrade(1)}>+1</button>
      </div>
    {/if}

  </div>
</div>

<style>
  .trainer-strip {
    display: contents;
  }

  .strip-row {
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

  .strip-divider {
    width: 1px;
    height: 24px;
    background: var(--border-default);
    flex-shrink: 0;
  }

  .strip-target {
    display: flex;
    align-items: baseline;
    gap: var(--space-sm);
    flex-shrink: 0;
  }

  .target-value {
    font-family: var(--font-data);
    font-size: var(--text-xl);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    color: var(--text-primary);
    line-height: 1;
  }

  .target-unit {
    font-size: 0.6em;
    font-weight: 500;
    color: var(--text-muted);
    margin-left: 1px;
  }

  .actual-value {
    font-family: var(--font-data);
    font-size: var(--text-sm);
    color: var(--info);
    font-weight: 500;
  }

  .strip-adjust {
    display: flex;
    gap: 4px;
  }

  .adj-btn {
    padding: var(--space-xs) var(--space-sm);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-sm);
    background: var(--bg-body);
    color: var(--text-secondary);
    font-family: var(--font-data);
    font-size: var(--text-xs);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
    transition: all var(--transition-fast);
    min-width: 36px;
    text-align: center;
  }

  .adj-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
    background: var(--accent-soft);
  }

  .adj-btn:active {
    transform: scale(0.95);
  }

</style>
