<script lang="ts">
  import { formatDuration } from '$lib/utils/format';
  import type { ZoneBucket } from '$lib/tauri';

  interface Props {
    powerZones: ZoneBucket[];
    hrZones: ZoneBucket[];
    ftp?: number | null;
    powerZonePcts?: [number, number, number, number, number, number] | null;
    hrZoneBounds?: [number, number, number, number, number] | null;
  }

  let { powerZones, hrZones, ftp = null, powerZonePcts = null, hrZoneBounds = null }: Props = $props();

  const POWER_COLORS = ['#70708a', '#4a90d9', '#4caf50', '#ffc107', '#ff9800', '#f44336', '#b71c1c'];
  const HR_COLORS = ['#70708a', '#4a90d9', '#4caf50', '#ffc107', '#f44336'];

  const POWER_ZONE_NAMES = ['Active Recovery', 'Endurance', 'Tempo', 'Threshold', 'VO2max', 'Anaerobic', 'Neuromuscular'];
  const HR_ZONE_NAMES = ['Recovery', 'Endurance', 'Tempo', 'Threshold', 'VO2max'];

  function powerZoneRange(zone: number): string {
    if (ftp == null || ftp <= 0 || !powerZonePcts) return '';
    // powerZonePcts are 6 boundaries: [55, 75, 90, 105, 120, 150] (% of FTP)
    const boundaries = [0, ...powerZonePcts.map(p => Math.round(ftp! * p / 100))];
    if (zone === 7) return `>${boundaries[6]}W`;
    return `${boundaries[zone - 1]}-${boundaries[zone]}W`;
  }

  function hrZoneRange(zone: number): string {
    if (!hrZoneBounds) return '';
    // hrZoneBounds are 5 boundaries: [z1_top, z2_top, z3_top, z4_top, z5_top]
    const bounds = [0, ...hrZoneBounds];
    if (zone === 5) return `>${bounds[4]} bpm`;
    return `${bounds[zone - 1]}-${bounds[zone]} bpm`;
  }
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
            <div class="zone-tooltip">
              <strong>Z{z.zone} — {POWER_ZONE_NAMES[z.zone - 1] ?? ''}</strong>
              {#if ftp && powerZonePcts}
                <span class="tooltip-range">{powerZoneRange(z.zone)}</span>
              {/if}
              <span class="tooltip-detail">{pct.toFixed(1)}% &middot; {formatDuration(Math.round(z.duration_secs))}</span>
            </div>
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
            <div class="zone-tooltip">
              <strong>Z{z.zone} — {HR_ZONE_NAMES[z.zone - 1] ?? ''}</strong>
              {#if hrZoneBounds}
                <span class="tooltip-range">{hrZoneRange(z.zone)}</span>
              {/if}
              <span class="tooltip-detail">{pct.toFixed(1)}% &middot; {formatDuration(Math.round(z.duration_secs))}</span>
            </div>
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
    position: relative;
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

  .zone-tooltip {
    display: none;
    position: absolute;
    bottom: calc(100% + 6px);
    left: 2rem;
    background: #1c1c30;
    color: #f0f0f5;
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: var(--radius-md);
    padding: var(--space-sm) var(--space-md);
    font-size: var(--text-xs);
    line-height: 1.5;
    z-index: 100;
    pointer-events: none;
    white-space: nowrap;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
  }

  .zone-tooltip strong {
    display: block;
    margin-bottom: 2px;
  }

  .tooltip-range, .tooltip-detail {
    display: block;
    color: #a0a0b8;
    font-family: 'IBM Plex Mono', monospace;
  }

  .zone-tooltip::after {
    content: '';
    position: absolute;
    top: 100%;
    left: 24px;
    border: 5px solid transparent;
    border-top-color: #1c1c30;
  }

  .bar-row:hover .zone-tooltip {
    display: block;
  }
</style>
