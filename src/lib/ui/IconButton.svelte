<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';

  /** An icon-only button that always has a name: aria-label plus a tooltip on hover and keyboard focus. */
  let {
    label,
    size = 'md',
    variant = 'ghost',
    tooltipSide = 'bottom',
    children,
    ...rest
  }: {
    label: string;
    size?: 'md' | 'lg';
    variant?: 'ghost' | 'secondary';
    tooltipSide?: 'top' | 'right' | 'bottom' | 'left';
    children: Snippet;
  } & Omit<HTMLButtonAttributes, 'children'> = $props();
</script>

<span class="icon-button-wrap">
  <button
    type="button"
    class="btn btn-icon btn-{variant}"
    class:btn-lg={size === 'lg'}
    aria-label={label}
    {...rest}
  >
    {@render children()}
  </button>
  <span class="tip tip-{tooltipSide}" role="tooltip" aria-hidden="true">{label}</span>
</span>

<style>
  .icon-button-wrap {
    position: relative;
    display: inline-flex;
  }

  .tip {
    position: absolute;
    z-index: 200;
    padding: 4px 8px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border-default);
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-size: var(--text-xs);
    font-weight: 600;
    white-space: nowrap;
    pointer-events: none;
    opacity: 0;
    transition: opacity var(--transition-fast);
    box-shadow: var(--shadow-md);
  }

  .tip-bottom { top: calc(100% + 6px); left: 50%; transform: translateX(-50%); }
  .tip-top { bottom: calc(100% + 6px); left: 50%; transform: translateX(-50%); }
  .tip-right { left: calc(100% + 8px); top: 50%; transform: translateY(-50%); }
  .tip-left { right: calc(100% + 8px); top: 50%; transform: translateY(-50%); }

  .icon-button-wrap:hover .tip,
  button:focus-visible + .tip {
    opacity: 1;
    transition-delay: 300ms;
  }

  button:focus-visible + .tip {
    transition-delay: 0ms;
  }
</style>
