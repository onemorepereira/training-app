<script lang="ts">
  import { formatDuration } from '$lib/utils/format';
  import type { ZoneBucket } from '$lib/tauri';

  interface Props {
    powerZones: ZoneBucket[];
    hrZones: ZoneBucket[];
  }

  let { powerZones, hrZones }: Props = $props();

  const POWER_COLORS = ['#70708a', '#4a90d9', '#4caf50', '#ffc107', '#ff9800', '#f44336', '#b71c1c'];
  const HR_COLORS = ['#70708a', '#4a90d9', '#4caf50', '#ffc107', '#f44336'];
</script>

<div class="zone-dist">
  <section>
    <h3>Power Zones</h3>
    <div class="bars">
      {#each powerZones as z, i}
        {@const pct = z.percentage}
        {#if pct > 0}
          <div class="bar-row">
            <span class="zone-label">Z{z.zone}</span>
            <div class="bar-track">
              <div
                class="bar-fill"
                style:width="{Math.max(pct, 1)}%"
                style:background={POWER_COLORS[z.zone - 1] ?? POWER_COLORS[POWER_COLORS.length - 1]}
              ></div>
            </div>
            <span class="bar-value">{formatDuration(Math.round(z.duration_secs))}</span>
            <span class="bar-pct">{pct.toFixed(1)}%</span>
          </div>
        {/if}
      {/each}
    </div>
  </section>

  <section>
    <h3>HR Zones</h3>
    <div class="bars">
      {#each hrZones as z, i}
        {@const pct = z.percentage}
        {#if pct > 0}
          <div class="bar-row">
            <span class="zone-label">Z{z.zone}</span>
            <div class="bar-track">
              <div
                class="bar-fill"
                style:width="{Math.max(pct, 1)}%"
                style:background={HR_COLORS[z.zone - 1] ?? HR_COLORS[HR_COLORS.length - 1]}
              ></div>
            </div>
            <span class="bar-value">{formatDuration(Math.round(z.duration_secs))}</span>
            <span class="bar-pct">{pct.toFixed(1)}%</span>
          </div>
        {/if}
      {/each}
    </div>
  </section>
</div>

<style>
  .zone-dist {
    display: flex;
    flex-direction: column;
    gap: var(--space-lg);
  }

  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }

  h3 {
    margin: 0;
    font-size: 0.85rem;
    font-weight: 500;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .bars {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .bar-row {
    display: grid;
    grid-template-columns: 2rem 1fr 3.5rem 3.5rem;
    align-items: center;
    gap: var(--space-xs);
  }

  .zone-label {
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--text-secondary);
    font-family: 'IBM Plex Mono', monospace;
  }

  .bar-track {
    height: 20px;
    background: rgba(255, 255, 255, 0.04);
    border-radius: 4px;
    overflow: hidden;
  }

  .bar-fill {
    height: 100%;
    border-radius: 4px;
    transition: width 0.3s ease;
  }

  .bar-value {
    font-size: 0.75rem;
    color: var(--text-secondary);
    font-family: 'IBM Plex Mono', monospace;
    text-align: right;
  }

  .bar-pct {
    font-size: 0.75rem;
    color: var(--text-primary);
    font-family: 'IBM Plex Mono', monospace;
    text-align: right;
    font-weight: 500;
  }
</style>
