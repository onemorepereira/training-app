<script lang="ts">
  import type { Snippet } from 'svelte';

  /**
   * Whole-region state: loading, empty (with a next step) or error (plain title,
   * backend message under Details, and Retry). Error and empty never show together.
   */
  let {
    kind,
    title,
    message,
    details,
    action,
    icon,
  }: {
    kind: 'loading' | 'empty' | 'error';
    title: string;
    message?: string;
    details?: string;
    action?: { label: string; onclick: () => unknown };
    icon?: Snippet;
  } = $props();

  let busy = $state(false);

  async function run() {
    if (!action || busy) return;
    busy = true;
    try {
      await action.onclick();
    } finally {
      busy = false;
    }
  }
</script>

<div class="state state-{kind}" role={kind === 'error' ? 'alert' : 'status'} aria-busy={kind === 'loading'}>
  <div class="state-icon" aria-hidden="true">
    {#if kind === 'loading'}
      <span class="spinner"></span>
    {:else if icon}
      {@render icon()}
    {:else if kind === 'error'}
      <svg viewBox="0 0 24 24" width="32" height="32" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10" /><path d="M12 8v5M12 16h.01" /></svg>
    {:else}
      <svg viewBox="0 0 24 24" width="32" height="32" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 7h18M3 12h18M3 17h10" /></svg>
    {/if}
  </div>
  <h2 class="state-title">{title}</h2>
  {#if message}<p class="state-message">{message}</p>{/if}
  {#if details}
    <details class="state-details">
      <summary>Details</summary>
      <code>{details}</code>
    </details>
  {/if}
  {#if action}
    <button type="button" class="btn {kind === 'error' ? 'btn-primary' : 'btn-secondary'}" disabled={busy} onclick={run}>
      {busy ? 'Working…' : action.label}
    </button>
  {/if}
</div>

<style>
  .state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-3xl) var(--space-xl);
    text-align: center;
    animation: fade-in 200ms ease;
  }

  .state-icon {
    color: var(--text-muted);
    margin-bottom: var(--space-xs);
  }

  .state-error .state-icon {
    color: var(--danger-text);
  }

  .state-title {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
  }

  .state-message {
    margin: 0;
    max-width: 48ch;
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  .state-details {
    max-width: min(640px, 100%);
    color: var(--text-muted);
    font-size: var(--text-xs);
  }

  .state-details summary {
    display: inline-flex;
    align-items: center;
    min-height: var(--control-h-sm);
    padding: 0 var(--space-sm);
    cursor: pointer;
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  .state-details code {
    display: block;
    margin-top: var(--space-xs);
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    font-family: var(--font-data);
    white-space: pre-wrap;
    word-break: break-word;
    text-align: left;
  }

  .state .btn {
    margin-top: var(--space-sm);
  }

  .spinner {
    display: block;
    width: 28px;
    height: 28px;
    border: 3px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
</style>
