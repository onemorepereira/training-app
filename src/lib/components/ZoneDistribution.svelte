<script lang="ts">
  import { formatDuration } from '$lib/utils/format';
  import type { ZoneBucket } from '$lib/tauri';
  import { POWER_ZONE_COLORS, HR_ZONE_COLORS, POWER_ZONE_NAMES, HR_ZONE_NAMES } from '$lib/charts/theme';

  interface Props {
    powerZones: ZoneBucket[];
    hrZones: ZoneBucket[];
    ftp?: number | null;
    powerZonePcts?: [number, number, number, number, number, number] | null;
    hrZoneBounds?: [number, number, number, number, number] | null;
  }

  let { powerZones, hrZones, ftp = null, powerZonePcts = null, hrZoneBounds = null }: Props = $props();

  /** Every zone, in order; zones the ride never reached show 0:00 so the list never has gaps. */
  function allZones(buckets: ZoneBucket[], count: number): ZoneBucket[] {
    const byZone = new Map(buckets.map((b) => [b.zone, b]));
    return Array.from({ length: count }, (_, i) => byZone.get(i + 1) ?? ({ zone: i + 1, duration_secs: 0, percentage: 0 } as ZoneBucket));
  }

  let powerRows = $derived(allZones(powerZones, 7));
  let hrRows = $derived(allZones(hrZones, 5));

  function powerZoneRange(zone: number): string {
    if (ftp == null || ftp <= 0 || !powerZonePcts) return '';
    // powerZonePcts are 6 boundaries: [55, 75, 90, 105, 120, 150] (% of FTP)
    const boundaries = [0, ...powerZonePcts.map(p => Math.round(ftp! * p / 100))];
    if (zone === 7) return `> ${boundaries[6]} W`;
    return `${boundaries[zone - 1]}–${boundaries[zone]} W`;
  }

  function hrZoneRange(zone: number): string {
    if (!hrZoneBounds) return '';
    // hrZoneBounds are 5 boundaries: [z1_top, z2_top, z3_top, z4_top, z5_top]
    const bounds = [0, ...hrZoneBounds];
    if (zone === 5) return `> ${bounds[4]} bpm`;
    return `${bounds[zone - 1]}–${bounds[zone]} bpm`;
  }
</script>

{#snippet rows(list: ZoneBucket[], colors: readonly string[], names: readonly string[], range: (z: number) => string, title: string)}
  <section class="zone-group" aria-label={title}>
    <h3 class="zone-group-title">{title}</h3>
    <ul class="bars">
      {#each list as z (z.zone)}
        {@const pct = z.percentage}
        <li class="bar-row" class:empty={pct <= 0}>
          <span class="zone-name">
            <span class="zone-num data">Z{z.zone}</span>
            <span class="zone-text">{names[z.zone - 1] ?? ''}</span>
            {#if range(z.zone)}<span class="zone-range data">{range(z.zone)}</span>{/if}
          </span>
          <span class="bar-track" aria-hidden="true">
            {#if pct > 0}
              <span class="bar-fill" style:width="{Math.max(pct, 1)}%" style:background={colors[z.zone - 1] ?? colors[colors.length - 1]}></span>
            {/if}
          </span>
          <span class="bar-value data">{formatDuration(Math.round(z.duration_secs))}</span>
          <span class="bar-pct data">{pct.toFixed(1)}%</span>
        </li>
      {/each}
    </ul>
  </section>
{/snippet}

<div class="zone-dist">
  {@render rows(powerRows, POWER_ZONE_COLORS, POWER_ZONE_NAMES, powerZoneRange, 'Power zones')}
  {@render rows(hrRows, HR_ZONE_COLORS, HR_ZONE_NAMES, hrZoneRange, 'Heart rate zones')}
</div>

<style>
  .zone-dist {
    display: flex;
    flex-direction: column;
    gap: var(--space-lg);
    padding: var(--space-lg);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-lg);
    background: var(--bg-surface);
  }

  .zone-group {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }

  .zone-group-title {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .bars {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .bar-row {
    display: grid;
    grid-template-columns: minmax(150px, 1.1fr) 2fr 4.5rem 3.75rem;
    align-items: center;
    gap: var(--space-sm);
    min-height: 28px;
    font-size: var(--text-sm);
  }

  .zone-name {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .zone-num {
    font-weight: 700;
    color: var(--text-primary);
  }

  .zone-text {
    color: var(--text-secondary);
  }

  .zone-range {
    color: var(--text-muted);
    font-size: var(--text-xs);
  }

  .bar-track {
    display: block;
    height: 18px;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.05);
    overflow: hidden;
  }

  .bar-fill {
    display: block;
    height: 100%;
    border-radius: 4px;
    transition: width 0.3s ease;
  }

  .bar-value,
  .bar-pct {
    text-align: right;
    white-space: nowrap;
  }

  .bar-value {
    color: var(--text-secondary);
  }

  .bar-pct {
    color: var(--text-primary);
    font-weight: 600;
  }

  .bar-row.empty .bar-value,
  .bar-row.empty .bar-pct {
    color: var(--text-muted);
    font-weight: 400;
  }

  /* Narrow panels: range drops under the name */
  @media (max-width: 900px) {
    .zone-range {
      display: none;
    }
  }
</style>
