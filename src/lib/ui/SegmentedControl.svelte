<script lang="ts" generics="T extends string | number">
  import { nextIndex } from './segmented';

  /** A single-choice group (radiogroup) with one tab stop; arrows, Home and End move the choice. */
  let {
    options,
    value = $bindable(),
    label,
    size = 'md',
    onchange,
    ...rest
  }: {
    options: readonly { value: T; label: string; title?: string }[];
    value: T;
    label: string;
    size?: 'md' | 'lg';
    onchange?: (value: T) => void;
    [key: `data-${string}`]: unknown;
  } = $props();

  let buttons: HTMLButtonElement[] = $state([]);
  let selected = $derived(options.findIndex((o) => o.value === value));

  function choose(i: number) {
    const v = options[i].value;
    if (v !== value) {
      value = v;
      onchange?.(v);
    }
  }

  function onkeydown(e: KeyboardEvent) {
    const i = nextIndex(e.key, selected, options.length);
    if (i === null) return;
    e.preventDefault();
    choose(i);
    buttons[i]?.focus();
  }
</script>

<div class="segmented" class:lg={size === 'lg'} role="radiogroup" aria-label={label} tabindex="-1" {onkeydown} {...rest}>
  {#each options as option, i (option.value)}
    <button
      bind:this={buttons[i]}
      type="button"
      role="radio"
      aria-checked={i === selected}
      tabindex={i === selected || (selected < 0 && i === 0) ? 0 : -1}
      title={option.title}
      onclick={() => choose(i)}
    >
      {option.label}
    </button>
  {/each}
</div>

<style>
  .segmented {
    display: inline-flex;
    width: fit-content;
    max-width: 100%;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--border-default);
    border-radius: var(--radius-md);
    background: var(--bg-input);
  }

  .segmented:focus {
    outline: none;
  }

  button {
    min-height: var(--control-h-sm);
    min-width: 44px;
    padding: 0 var(--space-md);
    border: none;
    border-radius: calc(var(--radius-md) - 2px);
    background: transparent;
    color: var(--text-secondary);
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    transition: background var(--transition-fast), color var(--transition-fast);
  }

  .segmented.lg button {
    min-height: var(--control-h-lg);
    min-width: 56px;
    padding: 0 var(--space-lg);
    font-size: var(--text-base);
  }

  button:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  button[aria-checked='true'] {
    background: var(--accent-soft);
    color: var(--text-primary);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
</style>
