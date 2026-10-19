<script lang="ts">
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import type { SessionSummary, SessionAnalysis, SessionConfig, ZoneRideConfig, PowerCurvePoint } from '$lib/tauri';
  import { api, extractError } from '$lib/tauri';
  import SessionTimeseries from '$lib/components/SessionTimeseries.svelte';
  import PowerCurve from '$lib/components/PowerCurve.svelte';
  import PowerHistogram from '$lib/components/PowerHistogram.svelte';
  import HrHistogram from '$lib/components/HrHistogram.svelte';
  import ZoneDistribution from '$lib/components/ZoneDistribution.svelte';
  import ZoneRideAnalysis from '$lib/components/ZoneRideAnalysis.svelte';
  import ActivityModal from '$lib/components/ActivityModal.svelte';
  import MetricCard from '$lib/components/MetricCard.svelte';
  import { formatDuration, formatDateLong, formatTime } from '$lib/utils/format';
  import { TYPE_LABELS, displayTitle, rpeColor } from '$lib/utils/session';
  import { formatSpeed } from '$lib/stores/units';
  import { SERIES } from '$lib/charts/theme';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import StateView from '$lib/ui/StateView.svelte';
  import Notice from '$lib/ui/Notice.svelte';

  const SMOOTHING = [
    { value: 1, label: 'Raw' },
    { value: 3, label: '3s' },
    { value: 10, label: '10s' },
    { value: 30, label: '30s' },
  ] as const;
  const PERIODS = [
    { value: 'all', label: 'All' },
    { value: '90d', label: '90d' },
    { value: '30d', label: '30d' },
  ] as const;
  const POWER_BUCKETS = [
    { value: 10, label: '10W' },
    { value: 20, label: '20W' },
    { value: 50, label: '50W' },
  ] as const;
  const HR_BUCKETS = [
    { value: 3, label: '3 bpm' },
    { value: 5, label: '5 bpm' },
    { value: 10, label: '10 bpm' },
  ] as const;

  let session = $state<SessionSummary | null>(null);
  let analysis = $state<SessionAnalysis | null>(null);
  let config = $state<SessionConfig | null>(null);
  let loading = $state(true);
  let analysisLoading = $state(true);
  /** Loading the ride itself failed (page-level error state). */
  let loadError = $state('');
  /** Loading the analysis (charts) failed; the summary still shows. */
  let analysisError = $state('');
  /** An action (export, save, delete) failed: dismissible notice. */
  let error = $state('');
  /** Bumped by Retry to re-run the loads. */
  let reloadToken = $state(0);
  let notFound = $derived(/not found/i.test(loadError));
  let editSession = $state<SessionSummary | null>(null);
  let exportingFit = $state(false);
  let smoothing = $state(10);
  let zoneConfig = $state<ZoneRideConfig | null>(null);
  let histBucket = $state(20);
  let hrHistBucket = $state(5);
  let overlayPeriod = $state<string>('all');
  let bestCurve = $state<PowerCurvePoint[]>([]);

  let units = $derived(config?.units ?? 'metric');
  // Zones in effect when the session was recorded (falls back to current zones in the backend)
  let hrZones = $derived(analysis?.zones.hr_zones ?? config?.hr_zones ?? null);
  let powerZonePcts = $derived(analysis?.zones.power_zones ?? config?.power_zones ?? null);

  $effect(() => {
    const sessionId = $page.params.id;
    void reloadToken;
    if (!sessionId) return;

    let cancelled = false;
    loading = true;
    analysisLoading = true;
    loadError = '';
    analysisError = '';

    // Load session summary (fast) and config
    Promise.all([api.getSession(sessionId), api.getUserConfig()])
      .then(([sess, cfg]) => {
        if (cancelled) return;
        session = sess;
        config = cfg;
        loading = false;
      })
      .catch((e) => {
        if (cancelled) return;
        loadError = extractError(e);
        loading = false;
      });

    // Load analysis (slower, file I/O + computation)
    api.getSessionAnalysis(sessionId)
      .then((a) => { if (!cancelled) analysis = a; })
      .catch((e) => { if (!cancelled) analysisError = extractError(e); })
      .finally(() => { if (!cancelled) analysisLoading = false; });

    // Load zone ride config (if any)
    api.getZoneRideConfig(sessionId)
      .then((json) => {
        if (cancelled || !json) return;
        try { zoneConfig = JSON.parse(json) as ZoneRideConfig; } catch { /* ignore malformed */ }
      })
      .catch(() => { /* optional data, ignore errors */ });

    // Backfill power curves (idempotent, skips already-done)
    api.backfillPowerCurves().catch(() => { /* best-effort */ });

    return () => { cancelled = true; };
  });

  $effect(() => {
    const period = overlayPeriod;
    api.getBestPowerCurve(period)
      .then((curve) => { bestCurve = curve; })
      .catch(() => { bestCurve = []; });
  });

  function retry() {
    reloadToken++;
  }

  async function exportFit() {
    if (!session) return;
    error = '';
    exportingFit = true;
    try {
      await api.exportSessionFit(session.id);
    } catch (e) {
      error = extractError(e);
    } finally {
      exportingFit = false;
    }
  }

  async function handleSave(title: string | null, activityType: string | null, rpe: number | null, notes: string | null) {
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

  let hasPowerHist = $derived(analysis != null && analysis.timeseries.some((p) => p.power != null));
  let hasHrHist = $derived(analysis != null && analysis.timeseries.some((p) => p.heart_rate != null));
</script>

<div class="page-wide detail" data-page="history-detail">
  <a href="/history" class="btn btn-ghost back">
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <polyline points="15 18 9 12 15 6"/>
    </svg>
    History
  </a>

  {#if loading}
    <StateView kind="loading" title="Loading ride…" />
  {:else if loadError && notFound}
    <StateView
      kind="empty"
      title="Ride not found"
      message="It may have been deleted."
      action={{ label: 'Back to History', onclick: () => goto('/history') }}
    />
  {:else if loadError}
    <StateView
      kind="error"
      title="Couldn't load this ride"
      message="The ride is still saved; it just couldn't be read right now."
      details={loadError}
      action={{ label: 'Retry', onclick: retry }}
    />
  {:else if session}
    <header class="detail-header">
      <div class="title-block">
        <div class="title-row">
          <h1 class="page-title">{displayTitle(session)}</h1>
          {#if session.activity_type}
            <span class="type-badge">{TYPE_LABELS[session.activity_type] ?? session.activity_type}</span>
          {/if}
          {#if session.rpe != null}
            <span class="rpe-badge data" style="color: {rpeColor(session.rpe)}">RPE {session.rpe}</span>
          {/if}
        </div>
        <div class="subtitle">{formatDateLong(session.start_time)} · {formatTime(session.start_time)}</div>
        {#if session.notes}
          <p class="notes">{session.notes}</p>
        {/if}
      </div>
      <div class="actions">
        <button type="button" class="btn btn-secondary" onclick={() => (editSession = session)}>
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/>
            <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/>
          </svg>
          Edit
        </button>
        <button type="button" class="btn btn-secondary" disabled={exportingFit} onclick={exportFit}>
          {exportingFit ? 'Exporting…' : 'Export FIT'}
        </button>
      </div>
    </header>

    {#if error}
      <div class="notice-slot">
        <Notice tone="error" ondismiss={() => (error = '')}>{error}</Notice>
      </div>
    {/if}

    <div class="groups">
      <section class="group" style="--panel: var(--panel-time)" aria-label="Time">
        <h2 class="group-label">Time</h2>
        <div class="group-cards">
          <MetricCard label="Duration" value={formatDuration(session.duration_secs)} size="sm" />
          <MetricCard label="Work" value={session.work_kj != null ? Math.round(session.work_kj) : null} unit="kJ" size="sm" />
        </div>
      </section>

      <section class="group" style="--panel: var(--panel-effort)" aria-label="Effort">
        <h2 class="group-label">Effort</h2>
        <div class="group-cards">
          <MetricCard label="NP" value={session.normalized_power} unit="W" size="sm" />
          <MetricCard label="TSS" value={session.tss != null ? Math.round(session.tss) : null} size="sm" />
          <MetricCard label="IF" value={session.intensity_factor != null ? session.intensity_factor.toFixed(2) : null} size="sm" />
          <MetricCard label="VI" value={session.variability_index != null ? session.variability_index.toFixed(2) : null} size="sm" />
        </div>
      </section>

      <section class="group" style="--panel: var(--panel-power)" aria-label="Power">
        <h2 class="group-label">Power</h2>
        <div class="group-cards">
          <MetricCard label="Avg Power" value={session.avg_power} unit="W" size="sm" />
          <MetricCard label="Max Power" value={session.max_power} unit="W" size="sm" />
          <MetricCard label="FTP" value={session.ftp} unit="W" size="sm" />
          {#if analysis?.pwc}
            <MetricCard label="PWC150" value={analysis.pwc.pwc150} unit="W" size="sm" />
            <MetricCard label="PWC170" value={analysis.pwc.pwc170} unit="W" size="sm" />
          {/if}
        </div>
      </section>

      {#if session.avg_hr != null || session.max_hr != null}
        <section class="group" style="--panel: var(--panel-hr)" aria-label="Heart rate">
          <h2 class="group-label">Heart rate</h2>
          <div class="group-cards">
            <MetricCard label="Avg HR" value={session.avg_hr} unit="bpm" size="sm" />
            <MetricCard label="Max HR" value={session.max_hr} unit="bpm" size="sm" />
          </div>
        </section>
      {/if}

      {#if session.avg_cadence != null || session.avg_speed != null || session.distance_km != null}
        <section class="group" style="--panel: var(--panel-move)" aria-label="Movement">
          <h2 class="group-label">Movement</h2>
          <div class="group-cards">
            {#if session.avg_cadence != null}
              <MetricCard label="Avg Cadence" value={Math.round(session.avg_cadence)} unit="rpm" size="sm" />
            {/if}
            {#if session.avg_speed != null}
              <MetricCard
                label="Avg Speed"
                value={formatSpeed(session.avg_speed, units as 'metric' | 'imperial')}
                unit={units === 'imperial' ? 'mph' : 'km/h'}
                size="sm"
              />
            {/if}
            {#if session.distance_km != null}
              <MetricCard
                label="Distance"
                value={units === 'imperial' ? (session.distance_km * 0.621371).toFixed(1) : session.distance_km.toFixed(1)}
                unit={units === 'imperial' ? 'mi' : 'km'}
                size="sm"
              />
            {/if}
          </div>
        </section>
      {/if}
    </div>

    {#if analysisError}
      <div class="panel">
        <StateView
          kind="error"
          title="Couldn't load the charts for this ride"
          message="The summary above is complete; the detailed recording couldn't be read."
          details={analysisError}
          action={{ label: 'Retry', onclick: retry }}
        />
      </div>
    {:else}
      <section class="chart-section">
        <div class="section-header">
          <h2 class="section-title">Time series</h2>
          <SegmentedControl label="Smoothing" options={SMOOTHING} bind:value={smoothing} />
        </div>
        <div class="chart-box timeseries-box">
          {#if analysisLoading}
            <div class="chart-placeholder loading">Loading chart…</div>
          {:else if analysis && analysis.timeseries.length > 0}
            <SessionTimeseries timeseries={analysis.timeseries} {smoothing} {units} />
          {:else}
            <div class="chart-placeholder">No recorded data for this ride.</div>
          {/if}
        </div>
      </section>

      {#if zoneConfig && analysis && analysis.timeseries.length > 0}
        <ZoneRideAnalysis
          config={zoneConfig}
          timeseries={analysis.timeseries.map((pt) => ({
            elapsed_secs: pt.elapsed_secs,
            power: pt.power ?? null,
            heart_rate: pt.heart_rate ?? null,
          }))}
        />
      {/if}

      <div class="two-col">
        <section class="chart-section">
          <div class="section-header">
            <h2 class="section-title">Power curve</h2>
            <SegmentedControl label="Compare with best from" options={PERIODS} bind:value={overlayPeriod} />
          </div>
          <div class="chart-box panel-box">
            {#if analysisLoading}
              <div class="chart-placeholder loading">Loading chart…</div>
            {:else if analysis && analysis.power_curve.length > 0}
              <PowerCurve
                powerCurve={analysis.power_curve}
                ftp={session.ftp}
                overlays={bestCurve.length > 0
                  ? [{ label: `Best (${overlayPeriod === 'all' ? 'All time' : overlayPeriod})`, data: bestCurve, color: SERIES.best }]
                  : []}
              />
            {:else}
              <div class="chart-placeholder">No power data for this ride.</div>
            {/if}
          </div>
        </section>

        <section class="chart-section">
          <div class="section-header">
            <h2 class="section-title">Zone distribution</h2>
          </div>
          {#if analysisLoading}
            <div class="chart-box panel-box"><div class="chart-placeholder loading">Loading zones…</div></div>
          {:else if analysis}
            <ZoneDistribution
              powerZones={analysis.power_zone_distribution}
              hrZones={analysis.hr_zone_distribution}
              ftp={session.ftp}
              {powerZonePcts}
              hrZoneBounds={hrZones}
            />
          {/if}
        </section>
      </div>

      {#if hasPowerHist || hasHrHist}
        <div class="two-col">
          {#if hasPowerHist}
            <section class="chart-section">
              <div class="section-header">
                <h2 class="section-title">Power distribution</h2>
                <SegmentedControl label="Bucket size" options={POWER_BUCKETS} bind:value={histBucket} />
              </div>
              <div class="chart-box panel-box">
                <PowerHistogram timeseries={analysis!.timeseries} bucketWidth={histBucket} ftp={session.ftp} />
              </div>
            </section>
          {/if}

          {#if hasHrHist}
            <section class="chart-section">
              <div class="section-header">
                <h2 class="section-title">Heart rate distribution</h2>
                <SegmentedControl label="Bucket size" options={HR_BUCKETS} bind:value={hrHistBucket} />
              </div>
              <div class="chart-box panel-box">
                <HrHistogram timeseries={analysis!.timeseries} bucketWidth={hrHistBucket} {hrZones} />
              </div>
            </section>
          {/if}
        </div>
      {/if}
    {/if}
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
  .detail {
    padding-bottom: var(--space-3xl);
  }

  .back {
    margin: 0 0 var(--space-md) calc(-1 * var(--space-md));
  }

  .detail-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: var(--space-md) var(--space-xl);
    margin-bottom: var(--space-lg);
  }

  .title-block {
    min-width: 0;
    flex: 1 1 420px;
  }

  .title-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-sm) var(--space-md);
    margin-bottom: 4px;
  }

  .type-badge {
    padding: 2px var(--space-sm);
    border-radius: var(--radius-full);
    background: var(--accent-soft);
    color: var(--text-primary);
    font-size: var(--text-xs);
    font-weight: 600;
    white-space: nowrap;
  }

  .rpe-badge {
    font-size: var(--text-sm);
    font-weight: 700;
  }

  .subtitle {
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  .notes {
    max-width: 80ch;
    margin: var(--space-sm) 0 0;
    color: var(--text-secondary);
    font-size: var(--text-sm);
    font-style: italic;
  }

  .actions {
    display: flex;
    gap: var(--space-sm);
  }

  .notice-slot {
    margin-bottom: var(--space-lg);
  }

  /* --- Summary groups --- */
  /* Groups size to their cards and wrap; cards never exceed 360px */
  .groups {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-md);
    margin-bottom: var(--space-2xl);
  }

  .group {
    flex: 1 1 auto;
    min-width: min(100%, 260px);
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-md) var(--space-md);
    border-radius: var(--radius-lg);
    background: var(--panel, transparent);
  }

  .group-label {
    margin: 0;
    padding-left: 2px;
    color: var(--text-secondary);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .group-cards {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-sm);
  }

  .group-cards > :global(*) {
    flex: 1 1 120px;
    max-width: 360px;
  }

  /* MetricCard's small size draws its unit at 0.5em (11px); keep units at the 12px floor */
  .group-cards :global(.unit) {
    font-size: max(0.5em, var(--text-xs));
  }

  /* --- Charts --- */
  .chart-section {
    min-width: 0;
    margin-bottom: var(--space-2xl);
  }

  .section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: var(--space-sm) var(--space-md);
    margin-bottom: var(--space-md);
  }

  .section-header .section-title {
    margin: 0;
  }

  .chart-box {
    position: relative;
  }

  .timeseries-box {
    height: clamp(320px, 45vh, 560px);
  }

  .panel-box {
    height: clamp(300px, 38vh, 460px);
  }

  .panel {
    margin-bottom: var(--space-2xl);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-lg);
    background: var(--bg-surface);
  }

  .chart-placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-lg);
    background: var(--bg-surface);
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  .chart-placeholder.loading {
    animation: pulse 1.5s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 0.6; }
    50% { opacity: 1; }
  }

  .two-col {
    display: grid;
    grid-template-columns: 1fr;
    gap: 0 var(--space-xl);
  }

  @media (min-width: 1200px) {
    .two-col {
      grid-template-columns: 1fr 1fr;
    }

    .two-col > :only-child {
      grid-column: 1 / -1;
    }
  }
</style>
