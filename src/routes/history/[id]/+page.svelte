<script lang="ts">
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import type { SessionSummary, SessionAnalysis } from '$lib/tauri';
  import { api, extractError } from '$lib/tauri';
  import SessionTimeseries from '$lib/components/SessionTimeseries.svelte';
  import PowerCurve from '$lib/components/PowerCurve.svelte';
  import ZoneDistribution from '$lib/components/ZoneDistribution.svelte';
  import ActivityModal from '$lib/components/ActivityModal.svelte';
  import MetricCard from '$lib/components/MetricCard.svelte';
  import { formatDuration, formatDateLong, formatTime, autoTitle } from '$lib/utils/format';
  import { unitSystem, formatSpeed, speedUnit, kmhToMph } from '$lib/stores/units';

  let session = $state<SessionSummary | null>(null);
  let analysis = $state<SessionAnalysis | null>(null);
  let units = $state<string>('metric');
  let loading = $state(true);
  let analysisLoading = $state(true);
  let error = $state('');
  let editSession = $state<SessionSummary | null>(null);
  let exportingFit = $state(false);
  let smoothing = $state(1);

  const TYPE_LABELS: Record<string, string> = {
    endurance: 'Endurance', intervals: 'Intervals', threshold: 'Threshold',
    sweet_spot: 'Sweet Spot', vo2max: 'VO2max', sprint: 'Sprint',
    tempo: 'Tempo', recovery: 'Recovery', race: 'Race', test: 'Test',
    warmup: 'Warmup', group_ride: 'Group Ride', free_ride: 'Free Ride', other: 'Other',
  };

  function rpeColor(value: number): string {
    if (value <= 5) {
      const ratio = (value - 1) / 4;
      const r = Math.round(76 + ratio * (255 - 76));
      const g = Math.round(175 + ratio * (255 - 175));
      const b = Math.round(80 + ratio * (77 - 80));
      return `rgb(${r}, ${g}, ${b})`;
    }
    const ratio = (value - 5) / 5;
    const r = Math.round(255 - ratio * (255 - 244));
    const g = Math.round(255 - ratio * (255 - 67));
    const b = Math.round(77 - ratio * (77 - 54));
    return `rgb(${r}, ${g}, ${b})`;
  }

  function displayTitle(s: SessionSummary): string {
    return s.title ?? autoTitle(s.start_time);
  }

  $effect(() => {
    const sessionId = $page.params.id;
    if (!sessionId) return;

    loading = true;
    analysisLoading = true;

    // Load session summary (fast) and config for units
    Promise.all([api.getSession(sessionId), api.getUserConfig()])
      .then(([sess, cfg]) => {
        session = sess;
        units = cfg.units;
        loading = false;
      })
      .catch((e) => {
        error = extractError(e);
        loading = false;
      });

    // Load analysis (slower, file I/O + computation)
    api.getSessionAnalysis(sessionId)
      .then((a) => { analysis = a; })
      .catch((e) => { error = extractError(e); })
      .finally(() => { analysisLoading = false; });
  });

  async function exportFit() {
    if (!session) return;
    exportingFit = true;
    try {
      await api.exportSessionFit(session.id);
    } catch (e) {
      error = extractError(e);
    } finally {
      exportingFit = false;
    }
  }

  async function handleSave(title: string, activityType: string | null, rpe: number | null, notes: string | null) {
    if (!session) return;
    try {
      await api.updateSessionMetadata(session.id, title, activityType, rpe, notes);
      session = await api.getSession(session.id);
    } catch (e) {
      error = extractError(e);
    }
    editSession = null;
  }

  async function handleDelete() {
    if (!session) return;
    try {
      await api.deleteSession(session.id);
      goto('/history');
    } catch (e) {
      error = extractError(e);
    }
    editSession = null;
  }
</script>

<div class="page">
  <!-- Back nav -->
  <a href="/history" class="back-link">
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <polyline points="15 18 9 12 15 6"/>
    </svg>
    History
  </a>

  {#if error}
    <div class="error">{error}</div>
  {/if}

  {#if loading}
    <p class="loading-text">Loading session...</p>
  {:else if session}
    <!-- Summary header -->
    <div class="summary-header">
      <div class="title-row">
        <h1>{displayTitle(session)}</h1>
        {#if session.activity_type}
          <span class="type-badge">{TYPE_LABELS[session.activity_type] ?? session.activity_type}</span>
        {/if}
        {#if session.rpe != null}
          <span class="rpe-badge" style="color: {rpeColor(session.rpe)}">RPE {session.rpe}</span>
        {/if}
      </div>
      <div class="subtitle">
        <span>{formatDateLong(session.start_time)}</span>
        <span class="sep"></span>
        <span class="time">{formatTime(session.start_time)}</span>
      </div>
      {#if session.notes}
        <p class="notes">{session.notes}</p>
      {/if}
    </div>

    <!-- Metrics grid -->
    <div class="metrics-grid">
      <MetricCard label="Duration" value={formatDuration(session.duration_secs)} size="sm" />
      <MetricCard label="NP" value={session.normalized_power} unit="W" size="sm" />
      <MetricCard label="TSS" value={session.tss != null ? Math.round(session.tss) : null} size="sm" />
      <MetricCard label="IF" value={session.intensity_factor != null ? session.intensity_factor.toFixed(2) : null} size="sm" />
      <MetricCard label="Avg Power" value={session.avg_power} unit="W" size="sm" />
      <MetricCard label="Max Power" value={session.max_power} unit="W" size="sm" />
      <MetricCard label="Avg HR" value={session.avg_hr} unit="bpm" size="sm" />
      <MetricCard label="Max HR" value={session.max_hr} unit="bpm" size="sm" />
      <MetricCard label="Avg Cadence" value={session.avg_cadence != null ? Math.round(session.avg_cadence) : null} unit="rpm" size="sm" />
      <MetricCard
        label="Avg Speed"
        value={session.avg_speed != null ? formatSpeed(session.avg_speed, units as 'metric' | 'imperial') : null}
        unit={units === 'imperial' ? 'mph' : 'km/h'}
        size="sm"
      />
      <MetricCard label="FTP" value={session.ftp} unit="W" size="sm" />
    </div>

    <!-- Actions -->
    <div class="actions">
      <button class="btn-secondary" onclick={() => editSession = session}>
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/>
          <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/>
        </svg>
        Edit
      </button>
      <button class="btn-secondary" disabled={exportingFit} onclick={exportFit}>
        {exportingFit ? 'Exporting...' : 'Export FIT'}
      </button>
    </div>

    <!-- Time-series chart -->
    <section class="chart-section">
      <div class="section-header">
        <h2>Time Series</h2>
        <div class="smoothing-toggle">
          {#each [{ v: 1, l: 'Raw' }, { v: 3, l: '3s' }, { v: 10, l: '10s' }, { v: 30, l: '30s' }] as opt}
            <button
              class="smooth-btn"
              class:active={smoothing === opt.v}
              onclick={() => smoothing = opt.v}
            >
              {opt.l}
            </button>
          {/each}
        </div>
      </div>
      <div class="timeseries-wrap">
        {#if analysisLoading}
          <div class="chart-skeleton">Loading chart...</div>
        {:else if analysis && analysis.timeseries.length > 0}
          <SessionTimeseries timeseries={analysis.timeseries} {smoothing} {units} />
        {:else}
          <div class="chart-empty">No time-series data</div>
        {/if}
      </div>
    </section>

    <!-- Power Curve + Zone Distribution (two-column) -->
    <div class="two-col">
      <section class="chart-section">
        <h2>Power Curve</h2>
        <div class="panel-wrap">
          {#if analysisLoading}
            <div class="chart-skeleton">Loading chart...</div>
          {:else if analysis && analysis.power_curve.length > 0}
            <PowerCurve powerCurve={analysis.power_curve} ftp={session.ftp} />
          {:else}
            <div class="chart-empty">No power data</div>
          {/if}
        </div>
      </section>

      <section class="zone-section">
        <h2>Zone Distribution</h2>
        {#if analysisLoading}
          <div class="chart-skeleton">Loading zones...</div>
        {:else if analysis}
          <ZoneDistribution
            powerZones={analysis.power_zone_distribution}
            hrZones={analysis.hr_zone_distribution}
          />
        {/if}
      </section>
    </div>
  {/if}
</div>

{#if editSession}
  <ActivityModal
    session={editSession}
    mode="edit"
    onSave={handleSave}
    onDelete={handleDelete}
    onClose={() => { editSession = null; }}
  />
{/if}

<style>
  .page {
    max-width: 100%;
    padding-bottom: var(--space-3xl);
  }

  .back-link {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    color: var(--text-muted);
    text-decoration: none;
    font-size: var(--text-sm);
    font-weight: 600;
    margin-bottom: var(--space-lg);
    transition: color var(--transition-fast);
  }

  .back-link:hover {
    color: var(--accent);
  }

  .error {
    margin-bottom: var(--space-lg);
    padding: var(--space-md);
    background: rgba(244, 67, 54, 0.08);
    border: 1px solid rgba(244, 67, 54, 0.3);
    border-radius: var(--radius-md);
    color: var(--danger);
    font-size: var(--text-base);
  }

  .loading-text {
    color: var(--text-muted);
    font-size: var(--text-base);
    padding: var(--space-3xl) 0;
    text-align: center;
  }

  /* --- Summary Header --- */
  .summary-header {
    margin-bottom: var(--space-lg);
  }

  .title-row {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    flex-wrap: wrap;
    margin-bottom: 4px;
  }

  h1 {
    margin: 0;
    font-size: var(--text-2xl);
    font-weight: 800;
  }

  .type-badge {
    padding: 2px var(--space-sm);
    border-radius: var(--radius-full);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: var(--text-xs);
    font-weight: 600;
    white-space: nowrap;
  }

  .rpe-badge {
    font-size: var(--text-xs);
    font-weight: 700;
    font-family: var(--font-data);
  }

  .subtitle {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }

  .sep {
    width: 3px;
    height: 3px;
    border-radius: 50%;
    background: var(--text-faint);
  }

  .time {
    color: var(--text-muted);
    font-size: var(--text-xs);
  }

  .notes {
    margin: var(--space-sm) 0 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-style: italic;
  }

  /* --- Metrics Grid --- */
  .metrics-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
    gap: var(--space-sm);
    margin-bottom: var(--space-lg);
  }

  /* --- Actions --- */
  .actions {
    display: flex;
    gap: var(--space-sm);
    margin-bottom: var(--space-xl);
  }

  .btn-secondary {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    padding: var(--space-xs) var(--space-md);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
    color: var(--text-secondary);
    font-size: var(--text-xs);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .btn-secondary:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
    background: var(--accent-soft);
  }

  .btn-secondary:disabled {
    opacity: 0.5;
    cursor: default;
  }

  /* --- Chart Sections --- */
  .chart-section {
    margin-bottom: var(--space-xl);
  }

  .chart-section h2, .zone-section h2 {
    margin: 0 0 var(--space-md);
    font-size: var(--text-base);
    font-weight: 700;
    color: var(--text-primary);
  }

  .section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-md);
  }

  .section-header h2 {
    margin: 0;
  }

  .smoothing-toggle {
    display: flex;
    gap: 2px;
    background: var(--bg-body);
    border-radius: var(--radius-md);
    padding: 3px;
  }

  .smooth-btn {
    padding: 4px 10px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-xs);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .smooth-btn:hover {
    color: var(--text-secondary);
  }

  .smooth-btn.active {
    background: var(--bg-elevated);
    color: var(--accent);
    box-shadow: var(--shadow-sm);
  }

  .timeseries-wrap {
    position: relative;
    height: 400px;
  }

  .panel-wrap {
    position: relative;
    height: 350px;
  }

  .chart-skeleton, .chart-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-lg);
    color: var(--text-muted);
    font-size: var(--text-sm);
  }

  .chart-skeleton {
    animation: pulse 1.5s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 0.6; }
    50% { opacity: 1; }
  }

  /* --- Two Column Layout --- */
  .two-col {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-xl);
  }

  .zone-section {
    margin-bottom: var(--space-xl);
  }

  @media (max-width: 720px) {
    .two-col {
      grid-template-columns: 1fr;
    }
  }
</style>
