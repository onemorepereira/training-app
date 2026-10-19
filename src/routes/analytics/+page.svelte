<script lang="ts">
  import { onMount } from 'svelte';
  import type { SessionSummary } from '$lib/tauri';
  import { api, extractError } from '$lib/tauri';
  import { computePmc, computeWeeklyTrends, extractFtpProgression, computeRampRate, currentWeekTss } from '$lib/utils/analytics';
  import MetricCard from '$lib/components/MetricCard.svelte';
  import StateView from '$lib/ui/StateView.svelte';
  import { SERIES } from '$lib/charts/theme';
  import PmcChart from '$lib/components/PmcChart.svelte';
  import TrendChart from '$lib/components/TrendChart.svelte';

  let sessions = $state<SessionSummary[]>([]);
  let loading = $state(true);
  let error = $state('');

  async function load() {
    loading = true;
    error = '';
    try {
      sessions = await api.listSessions();
    } catch (e) {
      error = extractError(e);
    } finally {
      loading = false;
    }
  }

  onMount(load);

  let pmcData = $derived(computePmc(sessions));
  let weeklyData = $derived(computeWeeklyTrends(sessions));
  let ftpPoints = $derived(extractFtpProgression(sessions));

  let currentCtl = $derived(pmcData.length > 0 ? Math.round(pmcData[pmcData.length - 1].ctl) : null);
  let currentAtl = $derived(pmcData.length > 0 ? Math.round(pmcData[pmcData.length - 1].atl) : null);
  let currentTsb = $derived(pmcData.length > 0 ? Math.round(pmcData[pmcData.length - 1].tsb) : null);
  let currentFtp = $derived.by(() => {
    const withFtp = sessions.filter((s) => s.ftp != null);
    if (withFtp.length === 0) return null;
    const latest = withFtp.sort((a, b) => b.start_time.localeCompare(a.start_time))[0];
    return latest.ftp;
  });
  let rampRate = $derived(computeRampRate(pmcData));
  let rampAccent = $derived.by(() => {
    if (!rampRate) return '';
    switch (rampRate.classification) {
      case 'moderate': return 'var(--success)';
      case 'aggressive': return 'var(--warning)';
      case 'excessive': return 'var(--danger-text)';
      default: return '';
    }
  });

  let thisWeekTss = $derived(currentWeekTss(weeklyData));
</script>

<div class="page-wide" data-page="analytics">
  <header class="page-header">
    <h1 class="page-title">Analytics</h1>
  </header>

  {#if loading}
    <StateView kind="loading" title="Loading your training data…" />
  {:else if error}
    <StateView
      kind="error"
      title="Couldn't load your training data"
      message="Your rides are safe. Try again; if it keeps failing, restart the app."
      details={error}
      action={{ label: 'Retry', onclick: load }}
    />
  {:else if sessions.length === 0}
    <StateView kind="empty" title="Nothing to analyse yet" message="Analytics appear after your first ride. Start one from the Ride screen." />
  {:else}
    <div class="summary">
      <div class="tile"><MetricCard label="Fitness" value={currentCtl} unit="CTL" size="sm" accent={SERIES.ctl} /></div>
      <div class="tile"><MetricCard label="Fatigue" value={currentAtl} unit="ATL" size="sm" accent={SERIES.atl} /></div>
      <div class="tile"><MetricCard label="Form" value={currentTsb} unit="TSB" size="sm" accent={SERIES.speed} /></div>
      <div class="tile"><MetricCard label="FTP" value={currentFtp} unit="W" size="sm" /></div>
      <div class="tile">
        <MetricCard label="Week TSS" value={thisWeekTss} size="sm" />
        {#if !thisWeekTss}<span class="tile-note">No rides yet this week</span>{/if}
      </div>
      <div class="tile">
        <MetricCard
          label="Ramp"
          value={rampRate ? Math.round(rampRate.current * 10) / 10 : null}
          unit="CTL/wk"
          size="sm"
          accent={rampAccent}
        />
        {#if rampRate}<span class="tile-note ramp-note">{rampRate.classification}</span>{/if}
      </div>
    </div>

    <section class="chart-section">
      <h2 class="section-title">Performance management</h2>
      <PmcChart {pmcData} />
    </section>

    <div class="chart-grid">
      <section class="chart-section">
        <h2 class="section-title">Weekly volume</h2>
        <TrendChart
          labels={weeklyData.map((w) => w.weekStart)}
          barData={weeklyData.map((w) => Math.round(w.totalTss))}
          barLabel="TSS"
          barColor={SERIES.ctl}
          lineData={weeklyData.map((w) => w.avgPower)}
          lineLabel="Avg Power"
          lineColor={SERIES.power}
          lineUnit="W"
        />
      </section>

      <section class="chart-section">
        <h2 class="section-title">Heart rate trends</h2>
        <TrendChart
          labels={weeklyData.map((w) => w.weekStart)}
          barData={weeklyData.map((w) => w.sessionCount)}
          barLabel="Sessions"
          barColor={SERIES.cadence}
          lineData={weeklyData.map((w) => w.avgHr)}
          lineLabel="Avg HR"
          lineColor={SERIES.hr}
          lineUnit="bpm"
        />
      </section>
    </div>

    {#if ftpPoints.length > 1}
      <section class="chart-section">
        <h2 class="section-title">FTP progression</h2>
        <TrendChart
          labels={ftpPoints.map((p) => p.date)}
          barData={ftpPoints.map((p) => p.ftp)}
          barLabel="FTP"
          barColor={SERIES.speed}
          barUnit="W"
        />
      </section>
    {/if}
  {/if}
</div>

<style>
  .summary {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-md);
    margin-bottom: var(--space-xl);
  }

  .tile {
    flex: 1 1 180px;
    max-width: 360px;
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }

  /* MetricCard's small unit is 0.5em of 22px (11px); keep it at the 12px floor */
  .tile :global(.unit) {
    font-size: max(12px, 0.5em);
  }

  .tile-note {
    font-size: var(--text-xs);
    color: var(--text-muted);
    text-align: center;
  }

  .ramp-note {
    text-transform: capitalize;
  }

  .chart-section {
    margin-bottom: var(--space-xl);
  }

  .chart-grid {
    display: grid;
    grid-template-columns: 1fr;
    gap: 0 var(--space-xl);
  }

  @media (min-width: 1280px) {
    .chart-grid {
      grid-template-columns: 1fr 1fr;
    }
  }
</style>
