<script lang="ts">
  import { onMount } from 'svelte';
  import type { SessionSummary } from '$lib/tauri';
  import { api, extractError } from '$lib/tauri';
  import { computePmc, computeWeeklyTrends, extractFtpProgression, computeRampRate } from '$lib/utils/analytics';
  import MetricCard from '$lib/components/MetricCard.svelte';
  import PmcChart from '$lib/components/PmcChart.svelte';
  import TrendChart from '$lib/components/TrendChart.svelte';

  let sessions = $state<SessionSummary[]>([]);
  let loading = $state(true);
  let error = $state('');

  onMount(async () => {
    try {
      sessions = await api.listSessions();
    } catch (e) {
      error = extractError(e);
    } finally {
      loading = false;
    }
  });

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
      case 'moderate': return '#4caf50';
      case 'aggressive': return '#ff9800';
      case 'excessive': return '#f44336';
      default: return '#70708a';
    }
  });

  let thisWeekTss = $derived.by(() => {
    if (weeklyData.length === 0) return null;
    const lastBucket = weeklyData[weeklyData.length - 1];
    // Check if the last bucket is this week
    const now = new Date();
    const day = now.getUTCDay();
    const offset = day === 0 ? 6 : day - 1;
    const monday = new Date(now);
    monday.setUTCDate(monday.getUTCDate() - offset);
    monday.setUTCHours(0, 0, 0, 0);
    const thisMonday = monday.toISOString().slice(0, 10);
    return lastBucket.weekStart === thisMonday ? Math.round(lastBucket.totalTss) : 0;
  });
</script>

<div class="page">
  <div class="page-header">
    <h1>Analytics</h1>
  </div>

  {#if error}
    <div class="error-banner">{error}</div>
  {/if}

  {#if loading}
    <div class="empty-state">
      <div class="empty-spinner"></div>
      <p class="empty-text">Loading analytics...</p>
    </div>
  {:else if sessions.length === 0}
    <div class="empty-state">
      <div class="empty-icon">
        <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/>
        </svg>
      </div>
      <p class="empty-text">No sessions yet</p>
      <p class="empty-hint">Complete some rides to see your training trends</p>
    </div>
  {:else}
    <div class="summary-cards">
      <MetricCard label="Fitness" value={currentCtl} unit="CTL" size="sm" accent="#4a90d9" />
      <MetricCard label="Fatigue" value={currentAtl} unit="ATL" size="sm" accent="#ff4d6d" />
      <MetricCard label="Form" value={currentTsb} unit="TSB" size="sm" accent="#4caf50" />
      <MetricCard label="FTP" value={currentFtp} unit="W" size="sm" />
      <MetricCard label="Week TSS" value={thisWeekTss} size="sm" />
      <MetricCard
        label="Ramp"
        value={rampRate ? Math.round(rampRate.current * 10) / 10 : null}
        unit="CTL/wk"
        size="sm"
        accent={rampAccent}
      />
    </div>

    <section class="chart-section">
      <h2>Performance Management</h2>
      <PmcChart {pmcData} />
    </section>

    <div class="chart-grid">
      <section class="chart-section">
        <h2>Weekly Volume</h2>
        <TrendChart
          labels={weeklyData.map((w) => w.weekStart)}
          barData={weeklyData.map((w) => Math.round(w.totalTss))}
          barLabel="TSS"
          barColor="#4a90d9"
          lineData={weeklyData.map((w) => w.avgPower)}
          lineLabel="Avg Power"
          lineColor="#ff4d6d"
          lineUnit="W"
        />
      </section>

      <section class="chart-section">
        <h2>HR Trends</h2>
        <TrendChart
          labels={weeklyData.map((w) => w.weekStart)}
          barData={weeklyData.map((w) => w.sessionCount)}
          barLabel="Sessions"
          barColor="#64b5f6"
          lineData={weeklyData.map((w) => w.avgHr)}
          lineLabel="Avg HR"
          lineColor="#f44336"
          lineUnit="bpm"
        />
      </section>
    </div>

    {#if ftpPoints.length > 1}
      <section class="chart-section">
        <h2>FTP Progression</h2>
        <TrendChart
          labels={ftpPoints.map((p) => p.date)}
          barData={ftpPoints.map((p) => p.ftp)}
          barLabel="FTP"
          barColor="#4caf50"
          barUnit="W"
        />
      </section>
    {/if}
  {/if}
</div>

<style>
  .page {
    max-width: 1100px;
  }

  .page-header {
    margin-bottom: var(--space-xl);
  }

  h1 {
    margin: 0;
    font-size: var(--text-2xl);
    font-weight: 800;
  }

  h2 {
    margin: 0 0 var(--space-md) 0;
    font-size: var(--text-lg);
    font-weight: 700;
    color: var(--text-primary);
  }

  .empty-state {
    text-align: center;
    padding: var(--space-3xl) var(--space-lg);
    color: var(--text-muted);
  }

  .empty-icon {
    margin-bottom: var(--space-md);
    opacity: 0.4;
  }

  .empty-text {
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--text-secondary);
    margin: 0 0 var(--space-xs);
  }

  .empty-hint {
    font-size: var(--text-sm);
    color: var(--text-muted);
    margin: 0;
  }

  .empty-spinner {
    display: inline-block;
    width: 24px;
    height: 24px;
    border: 2.5px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    margin-bottom: var(--space-md);
  }

  .summary-cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: var(--space-md);
    margin-bottom: var(--space-xl);
  }

  .chart-section {
    margin-bottom: var(--space-xl);
  }

  .chart-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-xl);
  }

  .chart-grid .chart-section {
    margin-bottom: 0;
  }
</style>
