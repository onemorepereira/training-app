<script lang="ts">
  import type { Snippet } from 'svelte';
  import IconButton from './IconButton.svelte';

  /** Inline message with an icon, optional actions and an optional labelled Dismiss button. */
  let {
    tone = 'info',
    title,
    children,
    actions,
    ondismiss,
  }: {
    tone?: 'info' | 'success' | 'warning' | 'error';
    title?: string;
    children?: Snippet;
    actions?: Snippet;
    ondismiss?: () => void;
  } = $props();
</script>

<div class="notice notice-{tone}" role={tone === 'error' ? 'alert' : 'status'} data-notice>
  <span class="notice-icon" aria-hidden="true">
    {#if tone === 'success'}
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10" /><path d="m8 12 3 3 5-6" /></svg>
    {:else if tone === 'info'}
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="10" /><path d="M12 16v-5M12 8h.01" /></svg>
    {:else}
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z" /><path d="M12 9v4M12 17h.01" /></svg>
    {/if}
  </span>
  <div class="notice-content">
    {#if title}<div class="notice-title">{title}</div>{/if}
    {#if children}<div class="notice-text">{@render children()}</div>{/if}
  </div>
  {#if actions}<div class="notice-actions">{@render actions()}</div>{/if}
  {#if ondismiss}
    <IconButton label="Dismiss" tooltipSide="left" onclick={ondismiss}>
      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"><path d="M18 6 6 18M6 6l12 12" /></svg>
    </IconButton>
  {/if}
</div>

<style>
  .notice {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: var(--space-sm) var(--space-sm) var(--space-sm) var(--space-md);
    border: 1px solid var(--tone-border);
    border-radius: var(--radius-md);
    background: var(--tone-bg);
    color: var(--text-primary);
    font-size: var(--text-sm);
    animation: slide-up 200ms ease;
  }

  .notice-info { --tone: var(--info); --tone-bg: rgba(100, 181, 246, 0.08); --tone-border: rgba(100, 181, 246, 0.35); }
  .notice-success { --tone: var(--success); --tone-bg: rgba(76, 175, 80, 0.08); --tone-border: rgba(76, 175, 80, 0.35); }
  .notice-warning { --tone: var(--warning); --tone-bg: rgba(255, 183, 77, 0.08); --tone-border: rgba(255, 183, 77, 0.35); }
  .notice-error { --tone: var(--danger-text); --tone-bg: rgba(244, 67, 54, 0.08); --tone-border: rgba(244, 67, 54, 0.4); }

  .notice-icon {
    display: flex;
    flex-shrink: 0;
    color: var(--tone);
  }

  .notice-content {
    flex: 1;
    min-width: 0;
    padding: 6px 0;
  }

  .notice-title {
    font-weight: 650;
  }

  .notice-text {
    color: var(--text-secondary);
  }

  .notice-title + .notice-text {
    margin-top: 2px;
  }

  .notice-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-sm);
    flex-shrink: 0;
  }
</style>
