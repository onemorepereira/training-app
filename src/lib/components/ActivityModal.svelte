<script lang="ts">
  import { untrack } from 'svelte';
  import type { SessionSummary } from '$lib/tauri';
  import { formatDuration, formatDate, autoTitle } from '$lib/utils/format';

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
    onSave: (title: string, activityType: string | null, rpe: number | null, notes: string | null) => void;
    onClose: () => void;
    onDelete?: () => void;
    mode?: 'post-ride' | 'edit';
  } = $props();

  let dialogEl = $state<HTMLDialogElement | null>(null);

  $effect(() => {
    if (dialogEl && !dialogEl.open) {
      dialogEl.showModal();
    }
  });

  function handleClose() {
    dialogEl?.close();
    onClose();
  }

  // Modal is freshly mounted each time — snapshot initial values (one-time read)
  const init = untrack(() => ({
    title: session.title ?? autoTitle(session.start_time),
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
    onSave(title, selectedType, selectedRpe, notes || null);
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

<dialog bind:this={dialogEl} class="modal-dialog" onclose={handleClose}>
  <div class="modal">
    <div class="modal-header">
      <h2>{mode === 'post-ride' ? 'Save Activity' : 'Edit Activity'}</h2>
      <button class="close-btn" onclick={handleClose} aria-label="Close">
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
        </svg>
      </button>
    </div>

    <div class="session-summary">
      <span class="summary-item">{formatDate(session.start_time)}</span>
      <span class="summary-sep"></span>
      <span class="summary-item">{formatDuration(session.duration_secs)}</span>
      {#if session.normalized_power != null}
        <span class="summary-sep"></span>
        <span class="summary-item">{session.normalized_power}W NP</span>
      {/if}
      {#if session.tss != null}
        <span class="summary-sep"></span>
        <span class="summary-item">{Math.round(session.tss)} TSS</span>
      {/if}
    </div>

    <div class="form-group">
      <label class="form-label" for="activity-title">Title</label>
      <input
        id="activity-title"
        class="form-input"
        type="text"
        bind:value={title}
        placeholder="Activity title..."
      />
    </div>

    <div class="form-group" role="group" aria-label="Activity Type">
      <span class="form-label">Activity Type</span>
      <div class="type-grid">
        {#each ACTIVITY_TYPES as type_}
          <button
            class="type-chip"
            class:selected={selectedType === type_.value}
            onclick={() => selectedType = selectedType === type_.value ? null : type_.value}
          >
            {type_.label}
          </button>
        {/each}
      </div>
    </div>

    <div class="form-group" role="group" aria-label="RPE (1-10)">
      <span class="form-label">RPE (1-10)</span>
      <div class="rpe-row">
        {#each Array.from({length: 10}, (_, i) => i + 1) as value}
          <button
            class="rpe-btn"
            class:selected={selectedRpe === value}
            style="--rpe-color: {rpeColor(value)}"
            onclick={() => selectedRpe = selectedRpe === value ? null : value}
          >
            {value}
          </button>
        {/each}
      </div>
    </div>

    <div class="form-group">
      <label class="form-label" for="activity-notes">Notes</label>
      <textarea
        id="activity-notes"
        class="form-textarea"
        bind:value={notes}
        placeholder="How did it feel?"
        rows="3"
      ></textarea>
    </div>

    <div class="modal-actions">
      {#if mode === 'edit' && onDelete}
        <button
          class="btn-delete"
          onclick={handleDelete}
        >
          {confirmingDelete ? 'Confirm Delete' : 'Delete'}
        </button>
      {/if}
      <div class="actions-right">
        <button class="btn-secondary" onclick={handleClose}>
          {mode === 'post-ride' ? 'Skip' : 'Cancel'}
        </button>
        <button class="btn-primary" onclick={handleSave}>
          Save
        </button>
      </div>
    </div>
  </div>
</dialog>

<style>
  .modal-dialog {
    border: none;
    background: transparent;
    padding: 0;
    max-width: none;
    max-height: none;
    overflow: visible;
  }

  .modal-dialog::backdrop {
    background: rgba(0, 0, 0, 0.6);
    backdrop-filter: blur(4px);
    animation: fade-in 150ms ease;
  }

  @keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .modal {
    background: var(--bg-surface);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-xl);
    padding: var(--space-xl);
    width: 480px;
    max-width: 90vw;
    max-height: 90vh;
    overflow-y: auto;
    box-shadow: var(--shadow-lg);
    animation: slide-up 200ms ease;
  }

  @keyframes slide-up {
    from { opacity: 0; transform: translateY(12px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .modal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-lg);
  }

  .modal-header h2 {
    font-size: var(--text-lg);
    font-weight: 700;
    color: var(--text-primary);
    margin: 0;
  }

  .close-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: none;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .close-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .session-summary {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-sm);
    padding: var(--space-md);
    background: var(--bg-elevated);
    border-radius: var(--radius-md);
    margin-bottom: var(--space-xl);
    font-size: var(--text-sm);
    font-family: var(--font-data);
    color: var(--text-secondary);
  }

  .summary-sep {
    width: 3px;
    height: 3px;
    border-radius: 50%;
    background: var(--text-faint);
    flex-shrink: 0;
  }

  .form-group {
    margin-bottom: var(--space-lg);
  }

  .form-label {
    display: block;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin-bottom: var(--space-sm);
  }

  .form-input, .form-textarea {
    width: 100%;
    padding: var(--space-sm) var(--space-md);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-md);
    background: var(--bg-input);
    color: var(--text-primary);
    font-size: var(--text-base);
    font-family: var(--font-sans);
    transition: border-color var(--transition-fast);
    box-sizing: border-box;
  }

  .form-input:focus, .form-textarea:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }

  .form-textarea {
    resize: vertical;
    min-height: 60px;
  }

  .type-grid {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
  }

  .type-chip {
    padding: var(--space-xs) var(--space-md);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-full);
    background: transparent;
    color: var(--text-secondary);
    font-size: var(--text-xs);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--transition-fast);
    white-space: nowrap;
  }

  .type-chip:hover {
    border-color: var(--accent);
    color: var(--accent);
    background: var(--accent-soft);
  }

  .type-chip.selected {
    border-color: var(--accent);
    background: var(--accent);
    color: white;
  }

  .rpe-row {
    display: flex;
    gap: 4px;
  }

  .rpe-btn {
    flex: 1;
    aspect-ratio: 1;
    max-width: 40px;
    border: 1px solid var(--border-default);
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-secondary);
    font-size: var(--text-sm);
    font-weight: 700;
    font-family: var(--font-data);
    cursor: pointer;
    transition: all var(--transition-fast);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .rpe-btn:hover {
    border-color: var(--rpe-color);
    color: var(--rpe-color);
    background: color-mix(in srgb, var(--rpe-color) 12%, transparent);
  }

  .rpe-btn.selected {
    border-color: var(--rpe-color);
    background: var(--rpe-color);
    color: white;
    box-shadow: 0 0 8px color-mix(in srgb, var(--rpe-color) 40%, transparent);
  }

  .modal-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-md);
    padding-top: var(--space-lg);
    border-top: 1px solid var(--border-subtle);
    margin-top: var(--space-md);
  }

  .actions-right {
    display: flex;
    gap: var(--space-sm);
    margin-left: auto;
  }

  .btn-primary {
    padding: var(--space-sm) var(--space-xl);
    border: none;
    border-radius: var(--radius-md);
    background: var(--accent);
    color: white;
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .btn-primary:hover {
    box-shadow: 0 0 12px var(--accent-glow);
    filter: brightness(1.1);
  }

  .btn-secondary {
    padding: var(--space-sm) var(--space-xl);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-secondary);
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .btn-secondary:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .btn-delete {
    padding: var(--space-sm) var(--space-md);
    border: 1px solid rgba(244, 67, 54, 0.3);
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--danger);
    font-size: var(--text-xs);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .btn-delete:hover {
    background: rgba(244, 67, 54, 0.1);
    border-color: var(--danger);
  }
</style>
