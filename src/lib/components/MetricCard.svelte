<script lang="ts">
  import { metricTooltips } from '$lib/tooltips';

  let { label, value, unit = '', size = 'md', accent = '', tooltip = '' }: {
    label: string;
    value: string | number | null;
    unit?: string;
    size?: 'sm' | 'md' | 'lg';
    accent?: string;
    tooltip?: string;
  } = $props();

  let resolvedTooltip = $derived(tooltip || metricTooltips[label] || '');
</script>

<div class="metric-card {size}" class:has-tooltip={!!resolvedTooltip}>
  <div class="label">{label}</div>
  <div
    class="value"
    style={accent ? `color: ${accent}; text-shadow: 0 0 24px ${accent}` : ''}
  >
    {value ?? '--'}
    {#if value !== null && unit}
      <span class="unit">{unit}</span>
    {/if}
  </div>
  {#if resolvedTooltip}
    <div class="tooltip-popup">{resolvedTooltip}</div>
  {/if}
</div>

<style>
  .metric-card {
    background: var(--bg-surface);
    border-radius: var(--radius-lg);
    padding: var(--space-md) var(--space-lg);
    text-align: center;
    border: 1px solid var(--border-subtle);
    transition: all var(--transition-fast);
  }

  .has-tooltip {
    position: relative;
    cursor: help;
  }

  .metric-card:hover {
    border-color: var(--border-default);
    background: var(--bg-elevated);
  }

  .tooltip-popup {
    display: none;
    position: absolute;
    bottom: calc(100% + 8px);
    left: 50%;
    transform: translateX(-50%);
    background: #1c1c30;
    color: #f0f0f5;
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: var(--radius-md);
    padding: var(--space-sm) var(--space-md);
    font-size: var(--text-xs);
    font-family: var(--font-body);
    font-weight: 400;
    line-height: 1.4;
    max-width: 220px;
    width: max-content;
    text-align: left;
    z-index: 100;
    pointer-events: none;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
  }

  .tooltip-popup::after {
    content: '';
    position: absolute;
    top: 100%;
    left: 50%;
    transform: translateX(-50%);
    border: 5px solid transparent;
    border-top-color: #1c1c30;
  }

  .has-tooltip:hover .tooltip-popup {
    display: block;
  }

  .label {
    font-size: var(--text-xs);
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    font-weight: 500;
    margin-bottom: var(--space-xs);
  }

  .value {
    font-family: var(--font-data);
    font-variant-numeric: tabular-nums;
    font-weight: 700;
    color: var(--text-primary);
    line-height: 1.2;
  }

  .sm .value { font-size: 1.375rem; }
  .md .value { font-size: 2.75rem; }
  .lg .value { font-size: 3.5rem; }

  .unit {
    font-size: 0.5em;
    font-weight: 500;
    color: var(--text-muted);
    margin-left: 1px;
  }
</style>
