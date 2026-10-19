<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { trainerState, trainerError, type TrainerMode, type TrainerState } from '$lib/stores/trainer';
  import { currentPower } from '$lib/stores/sensor';
  import { api, extractError } from '$lib/tauri';
  import { createLatestWinsSender } from '$lib/utils/latestWins';
  import { zoneActive } from '$lib/stores/zoneRide';
  import { get } from 'svelte/store';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';

  const MODES = [
    { value: 'erg', label: 'ERG', title: 'Hold a target power' },
    { value: 'resistance', label: 'RES', title: 'Fixed resistance' },
    { value: 'simulation', label: 'SIM', title: 'Simulate a grade' },
  ] as const satisfies readonly { value: TrainerMode; label: string; title: string }[];

  const STEPS: Record<TrainerMode, { label: string; delta: number; aria: string }[]> = {
    erg: [
      { label: '−25', delta: -25, aria: 'Decrease target by 25 watts' },
      { label: '−5', delta: -5, aria: 'Decrease target by 5 watts' },
      { label: '+5', delta: 5, aria: 'Increase target by 5 watts' },
      { label: '+25', delta: 25, aria: 'Increase target by 25 watts' },
    ],
    resistance: [
      { label: '−10', delta: -10, aria: 'Decrease resistance by 10 percent' },
      { label: '−1', delta: -1, aria: 'Decrease resistance by 1 percent' },
      { label: '+1', delta: 1, aria: 'Increase resistance by 1 percent' },
      { label: '+10', delta: 10, aria: 'Increase resistance by 10 percent' },
    ],
    simulation: [
      { label: '−1', delta: -1, aria: 'Decrease grade by 1 percent' },
      { label: '−.1', delta: -0.1, aria: 'Decrease grade by 0.1 percent' },
      { label: '+.1', delta: 0.1, aria: 'Increase grade by 0.1 percent' },
      { label: '+1', delta: 1, aria: 'Increase grade by 1 percent' },
    ],
  };

  function adjust(delta: number) {
    if (local.mode === 'erg') adjustErg(delta);
    else if (local.mode === 'resistance') adjustResistance(delta);
    else adjustGrade(delta);
  }

  let local: TrainerState = $state({
    mode: 'erg',
    ergTarget: 150,
    resistanceLevel: 50,
    simGrade: 0,
    simCrr: 0.004,
    simCw: 0.51,
  });
  let errorTimer: ReturnType<typeof setTimeout> | null = null;

  const unsubscribe = trainerState.subscribe((v) => { local = { ...v }; });
  onDestroy(() => {
    unsubscribe();
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

  type TrainerCommand =
    | { kind: 'erg'; watts: number }
    | { kind: 'resistance'; level: number }
    | { kind: 'sim'; grade: number; crr: number; cw: number };

  // Rapid changes while a command is in flight collapse to the latest one,
  // which is sent as soon as the in-flight command settles.
  const sender = createLatestWinsSender<TrainerCommand>(
    (cmd) => {
      trainerError.set('');
      if (cmd.kind === 'erg') return api.setTrainerPower(cmd.watts);
      if (cmd.kind === 'resistance') return api.setTrainerResistance(cmd.level);
      return api.setTrainerSimulation(cmd.grade, cmd.crr, cmd.cw);
    },
    (e) => setError(extractError(e)),
  );

  function sendErg() {
    sender.submit({ kind: 'erg', watts: local.ergTarget });
  }

  function sendResistance() {
    sender.submit({ kind: 'resistance', level: local.resistanceLevel });
  }

  function sendSimulation() {
    sender.submit({ kind: 'sim', grade: local.simGrade, crr: local.simCrr, cw: local.simCw });
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
    // The zone controller owns the trainer during a zone ride.
    if (get(zoneActive)) return;

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
  <fieldset class="strip-fieldset" disabled={$zoneActive}>
    <legend class="sr-only">Trainer control</legend>
    <SegmentedControl
      size="lg"
      label="Trainer mode"
      options={MODES}
      value={local.mode}
      onchange={(m) => setMode(m)}
      data-ride-control
    />

    <div class="strip-target" aria-live="polite">
      {#if local.mode === 'erg'}
        <span class="target-value data">{local.ergTarget}<span class="target-unit">W</span></span>
        <span class="target-caption">target{#if $currentPower != null}{' · now '}<span class="data">{$currentPower} W</span>{/if}</span>
      {:else if local.mode === 'resistance'}
        <span class="target-value data">{local.resistanceLevel}<span class="target-unit">%</span></span>
        <span class="target-caption">resistance</span>
      {:else}
        <span class="target-value data">{local.simGrade.toFixed(1)}<span class="target-unit">%</span></span>
        <span class="target-caption">grade</span>
      {/if}
    </div>

    <div class="strip-adjust" role="group" aria-label="Adjust {local.mode === 'erg' ? 'target power' : local.mode === 'resistance' ? 'resistance' : 'grade'}">
      {#each STEPS[local.mode] as step (step.label)}
        <button type="button" class="btn btn-lg btn-secondary adj-btn data" data-ride-control onclick={() => adjust(step.delta)} aria-label={step.aria}>{step.label}</button>
      {/each}
    </div>
  </fieldset>
  {#if $zoneActive}
    <p class="zone-lock-hint">Zone ride active. Stop it to adjust the trainer manually.</p>
  {/if}
</div>

<style>
  .trainer-strip {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    min-width: 0;
  }

  .strip-fieldset {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-md);
    border: 0;
    margin: 0;
    padding: 0;
    min-width: 0;
  }

  .strip-fieldset:disabled {
    opacity: 0.5;
  }

  .strip-target {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    min-width: 92px;
    line-height: 1.1;
  }

  .target-value {
    font-size: 1.75rem;
    font-weight: 700;
    color: var(--text-primary);
  }

  .target-unit {
    margin-left: 2px;
    font-size: 0.5em;
    font-weight: 500;
    color: var(--text-muted);
  }

  .target-caption {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .strip-adjust {
    display: flex;
    gap: var(--space-xs);
  }

  .adj-btn {
    min-width: 56px;
    padding: 0 var(--space-sm);
  }

  .zone-lock-hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
</style>
