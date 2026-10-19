<script lang="ts">
  import type { Snippet } from 'svelte';
  import IconButton from './IconButton.svelte';

  /**
   * Modal dialog on native <dialog>: Esc closes it (the browser's cancel event),
   * a labelled Close button is always shown, focus returns to the opener, and
   * the backdrop doesn't close it unless asked (a stray click mustn't lose input).
   */
  let {
    open = $bindable(false),
    title,
    description,
    onclose,
    closeOnBackdrop = false,
    width = 'md',
    children,
    footer,
  }: {
    open?: boolean;
    title: string;
    description?: string;
    onclose?: () => void;
    closeOnBackdrop?: boolean;
    width?: 'sm' | 'md' | 'lg';
    children: Snippet;
    footer?: Snippet;
  } = $props();

  let dialog = $state<HTMLDialogElement | null>(null);
  let opener: HTMLElement | null = null;
  const titleId = `dlg-${Math.random().toString(36).slice(2, 9)}`;

  $effect(() => {
    if (!dialog) return;
    if (open && !dialog.open) {
      opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      dialog.showModal();
      // Focus the dialog itself (not its first button), so nothing looks pre-selected.
      dialog.focus();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  });

  // Closing while mounted, or unmounting while open, both release the modal.
  $effect(() => {
    const d = dialog;
    return () => {
      if (d?.open) d.close();
    };
  });

  function handleClose() {
    open = false;
    onclose?.();
    opener?.focus?.();
    opener = null;
  }

  function onclick(e: MouseEvent) {
    if (closeOnBackdrop && e.target === dialog) dialog?.close();
  }
</script>

<dialog bind:this={dialog} class="dialog dialog-{width}" tabindex="-1" aria-labelledby={titleId} onclose={handleClose} {onclick}>
  <header class="dialog-header">
    <div class="dialog-heading">
      <h2 id={titleId} class="dialog-title">{title}</h2>
      {#if description}<p class="dialog-description">{description}</p>{/if}
    </div>
    <IconButton label="Close" onclick={() => dialog?.close()}>
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6 6 18M6 6l12 12" /></svg>
    </IconButton>
  </header>
  <div class="dialog-body">
    {@render children()}
  </div>
  {#if footer}
    <footer class="dialog-footer">
      {@render footer()}
    </footer>
  {/if}
</dialog>

<style>
  .dialog {
    padding: 0;
    border: 1px solid var(--border-default);
    border-radius: var(--radius-lg);
    background: var(--bg-surface);
    color: var(--text-primary);
    box-shadow: var(--shadow-lg);
    width: min(calc(100vw - 32px), var(--dlg-w));
    max-height: min(calc(100vh - 32px), 760px);
  }

  .dialog:focus {
    outline: none;
  }

  .dialog[open] {
    display: flex;
    flex-direction: column;
    animation: slide-up 160ms ease;
  }

  .dialog-sm { --dlg-w: 420px; }
  .dialog-md { --dlg-w: 560px; }
  .dialog-lg { --dlg-w: 760px; }

  .dialog::backdrop {
    background: rgba(0, 0, 0, 0.6);
    backdrop-filter: blur(4px);
  }

  .dialog-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-lg);
    padding: var(--space-lg) var(--space-lg) var(--space-md) var(--space-xl);
    border-bottom: 1px solid var(--border-subtle);
  }

  .dialog-title {
    margin: 6px 0 0;
    font-size: var(--text-xl);
    font-weight: 700;
  }

  .dialog-description {
    margin: var(--space-xs) 0 0;
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  .dialog-body {
    padding: var(--space-lg) var(--space-xl);
    overflow-y: auto;
    min-height: 0;
  }

  .dialog-footer {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-sm);
    padding: var(--space-md) var(--space-xl) var(--space-lg);
    border-top: 1px solid var(--border-subtle);
  }
</style>
