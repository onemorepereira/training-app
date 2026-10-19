<script lang="ts">
  import type { HTMLButtonAttributes } from 'svelte/elements';

  /** On/off switch with a visible label; the whole row is the target. */
  let {
    checked = $bindable(false),
    label,
    size = 'md',
    disabled = false,
    onchange,
    ...rest
  }: {
    checked?: boolean;
    label: string;
    size?: 'md' | 'lg';
    disabled?: boolean;
    onchange?: (checked: boolean) => void;
  } & Omit<HTMLButtonAttributes, 'onchange'> = $props();

  function toggle() {
    checked = !checked;
    onchange?.(checked);
  }
</script>

<button
  type="button"
  class="switch"
  class:lg={size === 'lg'}
  role="switch"
  aria-checked={checked}
  {disabled}
  onclick={toggle}
  {...rest}
>
  <span class="track" aria-hidden="true"><span class="thumb"></span></span>
  <span class="switch-label">{label}</span>
</button>

<style>
  .switch {
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    min-height: var(--control-h);
    padding: 0 var(--space-md) 0 var(--space-sm);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
    color: var(--text-secondary);
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
  }

  .switch.lg {
    min-height: var(--control-h-lg);
    font-size: var(--text-base);
  }

  .switch:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .switch:disabled {
    cursor: not-allowed;
    color: var(--text-faint);
  }

  .track {
    position: relative;
    width: 34px;
    height: 20px;
    border-radius: var(--radius-full);
    background: var(--bg-hover);
    border: 1px solid var(--border-strong);
    transition: background var(--transition-fast);
  }

  .thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--text-secondary);
    transition: transform var(--transition-fast), background var(--transition-fast);
  }

  .switch[aria-checked='true'] {
    color: var(--text-primary);
  }

  .switch[aria-checked='true'] .track {
    background: var(--success-strong);
    border-color: var(--success-strong);
  }

  .switch[aria-checked='true'] .thumb {
    transform: translateX(14px);
    background: #fff;
  }
</style>
