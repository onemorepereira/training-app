<script lang="ts">
  let { label, value, unit = '', size = 'md', accent = '' }: {
    label: string;
    value: string | number | null;
    unit?: string;
    size?: 'sm' | 'md' | 'lg';
    accent?: string;
  } = $props();
</script>

<div class="metric-card {size}">
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

  .metric-card:hover {
    border-color: var(--border-default);
    background: var(--bg-elevated);
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
