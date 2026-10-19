<script lang="ts">
  import { untrack } from 'svelte';
  import type { SessionSummary } from '$lib/tauri';
  import { formatDuration, formatDate, autoTitle } from '$lib/utils/format';
  import Dialog from '$lib/ui/Dialog.svelte';

  const ACTIVITY_TYPES = [
    { value: 'endurance', label: 'Endurance' },
    { value: 'intervals', label: 'Intervals' },
    { value: 'threshold', label: 'Threshold' },
    { value: 'sweet_spot', label: 'Sweet Spot' },
    { value: 'vo2max', label: 'VO2max' },
    { value: 'sprint', label: 'Sprint' },
    { value: 'tempo', label: 'Tempo' },
    { value: 'recovery', label: 'Recovery' },
    { value: 'race', label: 'Race' },
    { value: 'test', label: 'Test' },
    { value: 'warmup', label: 'Warmup' },
    { value: 'group_ride', label: 'Group Ride' },
    { value: 'free_ride', label: 'Free Ride' },
    { value: 'other', label: 'Other' },
  ] as const;

  let {
    session,
    onSave,
    onClose,
    onDelete,
    mode = 'post-ride',
  }: {
    session: SessionSummary;
    onSave: (title: string | null, activityType: string | null, rpe: number | null, notes: string | null) => void;
    onClose: () => void;
    onDelete?: () => void;
    mode?: 'post-ride' | 'edit';
  } = $props();

  let open = $state(true);

  // Modal is freshly mounted each time — snapshot initial values (one-time read)
  const init = untrack(() => ({
    title: session.title?.trim() ? session.title : autoTitle(session.start_time),
    type: session.activity_type ?? null,
    rpe: session.rpe ?? null,
    notes: session.notes ?? '',
  }));

  let title = $state(init.title);
  let selectedType = $state<string | null>(init.type);
  let selectedRpe = $state<number | null>(init.rpe);
  let notes = $state(init.notes);
  let confirmingDelete = $state(false);

  function handleSave() {
    // Full-replace contract: every field is sent; null clears it.
    onSave(title.trim() || null, selectedType, selectedRpe, notes.trim() || null);
  }

  function handleDelete() {
    if (!confirmingDelete) {
      confirmingDelete = true;
      return;
    }
    onDelete?.();
  }

  function rpeColor(value: number): string {
    // green (1) -> yellow (5) -> red (10)
    if (value <= 5) {
      const ratio = (value - 1) / 4;
      const r = Math.round(76 + ratio * (255 - 76));
      const g = Math.round(175 + ratio * (183 - 175));
      const b = Math.round(80 + ratio * (77 - 80));
      return `rgb(${r}, ${g}, ${b})`;
    }
    const ratio = (value - 5) / 5;
    const r = Math.round(255 - ratio * (255 - 244));
    const g = Math.round(183 - ratio * (183 - 67));
    const b = Math.round(77 - ratio * (77 - 54));
    return `rgb(${r}, ${g}, ${b})`;
  }
</script>

<Dialog
  bind:open
  title={mode === 'post-ride' ? 'Ride saved' : 'Edit ride'}
  description={mode === 'post-ride' ? 'Your ride is saved. Add details now, or later from History.' : undefined}
  onclose={onClose}
  width="md"
>
  <div class="summary">
    <span>{formatDate(session.start_time)}</span>
    <span class="data">{formatDuration(session.duration_secs)}</span>
    {#if session.normalized_power != null}<span><span class="data">{session.normalized_power} W</span> NP</span>{/if}
    {#if session.tss != null}<span><span class="data">{Math.round(session.tss)}</span> TSS</span>{/if}
  </div>

  <div class="form">
    <label class="field">
      <span class="field-label">Title</span>
      <input type="text" bind:value={title} placeholder="Name this ride" />
    </label>

    <div class="field" role="group" aria-labelledby="am-type">
      <span class="field-label" id="am-type">Type</span>
      <div class="chips">
        {#each ACTIVITY_TYPES as type_ (type_.value)}
          <button
            type="button"
            class="btn btn-secondary chip"
            aria-pressed={selectedType === type_.value}
            onclick={() => (selectedType = selectedType === type_.value ? null : type_.value)}
          >{type_.label}</button>
        {/each}
      </div>
    </div>

    <div class="field" role="group" aria-labelledby="am-rpe">
      <span class="field-label" id="am-rpe">Effort (RPE)</span>
      <div class="rpe">
        {#each Array.from({ length: 10 }, (_, i) => i + 1) as value (value)}
          <button
            type="button"
            class="btn btn-secondary rpe-btn data"
            aria-pressed={selectedRpe === value}
            aria-label="RPE {value}"
            style="--rpe-color: {rpeColor(value)}"
            onclick={() => (selectedRpe = selectedRpe === value ? null : value)}
          >{value}</button>
        {/each}
      </div>
      <div class="rpe-anchors" aria-hidden="true"><span>Easy</span><span>Max</span></div>
    </div>

    <label class="field">
      <span class="field-label">Notes</span>
      <textarea bind:value={notes} placeholder="How did it feel?" rows="3"></textarea>
    </label>
  </div>

  {#snippet footer()}
    {#if mode === 'edit' && onDelete}
      <button type="button" class="btn btn-ghost btn-danger-text delete" onclick={handleDelete}>
        {confirmingDelete ? 'Click again to delete' : 'Delete ride'}
      </button>
    {/if}
    <button type="button" class="btn btn-secondary" onclick={() => (open = false)}>
      {mode === 'post-ride' ? 'Done' : 'Cancel'}
    </button>
    <button type="button" class="btn btn-primary" onclick={handleSave}>
      {mode === 'post-ride' ? 'Save details' : 'Save'}
    </button>
  {/snippet}
</Dialog>

<style>
  .summary {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs) var(--space-lg);
    margin-bottom: var(--space-lg);
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-lg);
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
  }

  .chip {
    font-weight: 500;
  }

  .rpe {
    display: grid;
    grid-template-columns: repeat(10, minmax(36px, 1fr));
    gap: var(--space-xs);
  }

  .rpe-btn {
    padding: 0;
  }

  .rpe-btn[aria-pressed='true'] {
    background: var(--rpe-color);
    border-color: var(--rpe-color);
    color: #0f0f1a;
  }

  .rpe-anchors {
    display: flex;
    justify-content: space-between;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .delete {
    margin-right: auto;
  }
</style>
