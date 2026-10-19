<script lang="ts">
  import PowerGauge from '$lib/components/PowerGauge.svelte';
  import MetricCard from '$lib/components/MetricCard.svelte';
  import MetricsChart from '$lib/components/MetricsChart.svelte';
  import RetroRide from '$lib/components/RetroRide.svelte';
  import TrainerControl from '$lib/components/TrainerControl.svelte';
  import ZoneRideBuilder from '$lib/components/ZoneRideBuilder.svelte';
  import ZoneRideStatus from '$lib/components/ZoneRideStatus.svelte';
  import ConnectionHealth from '$lib/components/ConnectionHealth.svelte';
  import RideStage from '$lib/components/RideStage.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import IconButton from '$lib/ui/IconButton.svelte';
  import Switch from '$lib/ui/Switch.svelte';
  import Notice from '$lib/ui/Notice.svelte';
  import { reconnectingDevices } from '$lib/stores/devices';
  import { currentPower, currentHR, currentCadence, currentSpeed, liveMetrics } from '$lib/stores/sensor';
  import { sessionActive, sessionPaused, dashboardView } from '$lib/stores/session';
  import { endSession, startSession, postRideSession, lifecycleError } from '$lib/stores/sessionLifecycle';
  import { autoSessionEnabled, autoSessionCountdown } from '$lib/stores/autoSession';
  import { trainerConnected } from '$lib/stores/devices';
  import { unitSystem, formatSpeed, speedUnit } from '$lib/stores/units';
  import { trainerError } from '$lib/stores/trainer';
  import { zoneActive, zoneStatus, zoneRideResult, startZoneRide, stopZoneRide, pauseZoneRide, resumeZoneRide, describeZoneStop, type ZoneRideResult } from '$lib/stores/zoneRide';
  import { api, extractError, type SessionConfig, type ZoneTarget } from '$lib/tauri';
  import ActivityModal from '$lib/components/ActivityModal.svelte';
  import { formatDuration } from '$lib/utils/format';
  import { toggleFullscreen, fullscreen, fullscreenShortcut } from '$lib/stores/fullscreen';

  let error = $state('');
  /** Zone-ride notice the user dismissed (the result itself is kept until saved). */
  let dismissedZoneResult = $state.raw<ZoneRideResult | null>(null);
  let zoneNotice = $derived($zoneRideResult && $zoneRideResult !== dismissedZoneResult ? $zoneRideResult : null);
  let showZoneBuilder = $state(false);
  let userConfig = $state<SessionConfig | null>(null);
  let configError = $state('');
  let pageWidth = $state(0);

  const VIEWS = [
    { value: 'gauges', label: 'Gauges' },
    { value: 'graphs', label: 'Graphs' },
    { value: 'retro', label: 'Retro' },
  ] as const;

  // One notice at a time, most important first; lower ones show once it clears.
  let notice = $derived.by(() => {
    if ($lifecycleError) return 'lifecycle';
    if (error) return 'page';
    if ($trainerError) return 'trainer';
    if (Object.keys($reconnectingDevices).length > 0) return 'health';
    if (configError) return 'config';
    if (zoneNotice) return 'zone';
    return null;
  });

  let zoneBand = $derived.by(() => {
    const s = $zoneStatus;
    if (!s?.active || s.lower_bound == null || s.upper_bound == null) return null;
    return { lower: s.lower_bound, upper: s.upper_bound };
  });

  // Load user config for zone bounds and the gauge's FTP
  async function loadConfig() {
    try {
      userConfig = await api.getUserConfig();
      configError = '';
    } catch (e) {
      configError = extractError(e);
    }
  }
  $effect(() => {
    loadConfig();
  });

  async function toggleSession() {
    error = '';
    if ($sessionActive) {
      showZoneBuilder = false;
      await endSession(); // surfaces its own errors via lifecycleError
      return;
    }
    lifecycleError.set('');
    try {
      await startSession();
    } catch (e) {
      lifecycleError.set(extractError(e));
    }
  }

  async function togglePause() {
    error = '';
    try {
      if ($sessionPaused) {
        await api.resumeSession();
        if ($zoneActive) await resumeZoneRide();
        sessionPaused.set(false);
      } else {
        await api.pauseSession();
        if ($zoneActive) await pauseZoneRide();
        sessionPaused.set(true);
      }
    } catch (e) {
      error = extractError(e);
    }
  }

  let zoneStarting = $state(false);
  let zoneStartError = $state('');

  async function handleStartZoneRide(target: ZoneTarget) {
    if (zoneStarting) return;
    zoneStarting = true;
    zoneStartError = '';
    try {
      await startZoneRide(target);
      showZoneBuilder = false;
    } catch (e) {
      // Shown inside the builder, which stays open, rather than behind it.
      zoneStartError = extractError(e);
    } finally {
      zoneStarting = false;
    }
  }

  async function handleStopZoneRide() {
    error = '';
    try {
      await stopZoneRide();
    } catch (e) {
      error = extractError(e);
    }
  }
</script>

<div class="ride" data-page="ride" bind:clientWidth={pageWidth}>
  <div class="ride-header">
    <SegmentedControl size="lg" label="Dashboard view" options={VIEWS} bind:value={$dashboardView} />
    {#if $autoSessionCountdown != null && !$sessionActive}
      <span class="countdown" role="status">Starting in {$autoSessionCountdown}…</span>
    {/if}
    <IconButton
      label={$fullscreen ? `Exit fullscreen (Esc)` : `Enter fullscreen (${fullscreenShortcut()})`}
      size="lg"
      variant="secondary"
      tooltipSide="left"
      onclick={toggleFullscreen}
    >
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <polyline points="15 3 21 3 21 9"/>
        <polyline points="9 21 3 21 3 15"/>
        <line x1="21" y1="3" x2="14" y2="10"/>
        <line x1="3" y1="21" x2="10" y2="14"/>
      </svg>
    </IconButton>
  </div>

  <RideStage>
    {#snippet children(layout)}
      {#if $dashboardView === 'gauges'}
        <div class="gauges gauges-{layout.mode}" class:no-strip={!layout.showSecondary}>
          <div class="gauge-cell">
            <PowerGauge power={$currentPower} ftp={userConfig?.ftp} />
          </div>
          <div class="primary-grid">
            <MetricCard label="Heart Rate" kind="primary" size="stage" value={$currentHR} unit="bpm" accent="var(--danger-text)" />
            <MetricCard label="Cadence" kind="primary" size="stage" value={$currentCadence != null ? Math.round($currentCadence) : null} unit="rpm" />
            <MetricCard label="Speed" kind="primary" size="stage" value={$currentSpeed != null ? formatSpeed($currentSpeed, $unitSystem) : null} unit={$speedUnit} />
            <MetricCard label="Time" kind="primary" size="stage" value={$liveMetrics ? formatDuration($liveMetrics.elapsed_secs) : null} />
          </div>
          {#if layout.showSecondary}
            <div class="secondary-strip">
              <MetricCard label="3s Power" kind="secondary" size="stage-sm" value={$liveMetrics?.avg_power_3s != null ? Math.round($liveMetrics.avg_power_3s) : null} unit="W" />
              <MetricCard label="NP" kind="secondary" size="stage-sm" value={$liveMetrics?.normalized_power != null ? Math.round($liveMetrics.normalized_power) : null} unit="W" />
              <MetricCard label="TSS" kind="secondary" size="stage-sm" value={$liveMetrics?.tss != null ? $liveMetrics.tss.toFixed(1) : null} />
              <MetricCard label="IF" kind="secondary" size="stage-sm" value={$liveMetrics?.intensity_factor != null ? $liveMetrics.intensity_factor.toFixed(2) : null} />
            </div>
          {/if}
        </div>
      {:else if $dashboardView === 'graphs'}
        <div class="graphs">
          <MetricsChart {zoneBand} />
        </div>
      {:else if $dashboardView === 'retro'}
        <div class="retro">
          <RetroRide />
        </div>
      {/if}
    {/snippet}
  </RideStage>

  <section class="controls" class:merged={pageWidth >= 1100} aria-label="Ride controls">
    {#if notice === 'lifecycle'}
      <Notice tone="error" ondismiss={() => lifecycleError.set('')}>{$lifecycleError}</Notice>
    {:else if notice === 'page'}
      <Notice tone="error" ondismiss={() => (error = '')}>{error}</Notice>
    {:else if notice === 'trainer'}
      <Notice tone="error" title="Trainer command failed" ondismiss={() => trainerError.set('')}>{$trainerError}</Notice>
    {:else if notice === 'health'}
      <ConnectionHealth />
    {:else if notice === 'config'}
      <Notice tone="warning" title="Couldn't load your settings" ondismiss={() => (configError = '')}>
        Zones and the power dial use defaults (FTP 200 W).
        {#snippet actions()}
          <button type="button" class="btn btn-secondary" onclick={loadConfig}>Retry</button>
        {/snippet}
      </Notice>
    {:else if notice === 'zone' && zoneNotice}
      <Notice tone={zoneNotice.error != null ? 'error' : 'success'} title={describeZoneStop(zoneNotice)} ondismiss={() => { dismissedZoneResult = zoneNotice; }}>
        {formatDuration(zoneNotice.config.time_in_zone_secs)} in zone.{#if zoneNotice.error} {zoneNotice.error}{/if}
      </Notice>
    {/if}

    <div class="control-rows">
      <div class="row row-session">
        <button
          type="button"
          class="btn btn-lg session-btn {$sessionActive ? 'btn-danger' : 'btn-success'}"
          data-ride-control
          onclick={toggleSession}
        >
          {#if $sessionActive}
            <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><rect x="5" y="5" width="14" height="14" rx="2" fill="currentColor"/></svg>
            Stop
          {:else}
            <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path d="M7 4v16l13-8z" fill="currentColor"/></svg>
            Start
          {/if}
        </button>
        {#if $sessionActive}
          <button type="button" class="btn btn-lg btn-secondary" data-ride-control onclick={togglePause}>
            {#if $sessionPaused}
              <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path d="M7 4v16l13-8z" fill="currentColor"/></svg>
              Resume
            {:else}
              <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><rect x="6" y="5" width="4" height="14" rx="1" fill="currentColor"/><rect x="14" y="5" width="4" height="14" rx="1" fill="currentColor"/></svg>
              Pause
            {/if}
          </button>
        {/if}
        <Switch size="lg" label="Auto start/stop" bind:checked={$autoSessionEnabled} data-ride-control title="Start when you begin pedalling and stop after you stop" />
      </div>

      {#if $trainerConnected}
        <div class="row row-trainer">
          {#if $zoneActive}
            <ZoneRideStatus onStop={handleStopZoneRide} />
          {:else}
            <TrainerControl />
            <button
              type="button"
              class="btn btn-lg btn-secondary"
              data-ride-control
              disabled={!userConfig}
              title={userConfig ? undefined : 'Zone rides need your settings to load'}
              onclick={() => { showZoneBuilder = true; }}
            >
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M22 12h-4l-3 9L9 3l-3 9H2"/></svg>
              Zone ride
            </button>
          {/if}
        </div>
      {/if}
    </div>
  </section>
</div>

{#if userConfig}
  <ZoneRideBuilder
    bind:open={showZoneBuilder}
    config={userConfig}
    trainerConnected={$trainerConnected}
    starting={zoneStarting}
    startError={zoneStartError}
    onStart={handleStartZoneRide}
    onClose={() => { showZoneBuilder = false; zoneStartError = ''; }}
  />
{/if}

{#if $postRideSession}
  <ActivityModal
    session={$postRideSession}
    mode="post-ride"
    onSave={async (title, activityType, rpe, notes) => {
      const id = $postRideSession?.id;
      if (!id) return;
      try {
        await api.updateSessionMetadata(id, title, activityType, rpe, notes);
      } catch (e) {
        error = extractError(e);
      }
      postRideSession.set(null);
    }}
    onClose={() => { postRideSession.set(null); }}
  />
{/if}

<style>
  .ride {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
  }

  .ride-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-md);
    min-height: var(--control-h-lg);
  }

  .countdown {
    padding: 6px 14px;
    border-radius: var(--radius-full);
    background: rgba(76, 175, 80, 0.12);
    border: 1px solid rgba(76, 175, 80, 0.4);
    color: var(--text-primary);
    font-weight: 600;
    font-size: var(--text-sm);
  }

  /* ── Gauges view ── */
  .gauges {
    height: 100%;
    display: grid;
    gap: 8px 12px;
  }

  .gauges-wide {
    grid-template-columns: 1fr 1.5fr;
    grid-template-rows: 1fr var(--strip-px);
    grid-template-areas: 'gauge primary' 'strip strip';
  }

  .gauges-stacked {
    grid-template-columns: 1fr;
    grid-template-rows: 0.45fr 0.55fr var(--strip-px);
    grid-template-areas: 'gauge' 'primary' 'strip';
  }

  .gauges-wide.no-strip {
    grid-template-rows: 1fr;
    grid-template-areas: 'gauge primary';
  }

  .gauges-stacked.no-strip {
    grid-template-rows: 0.45fr 0.55fr;
    grid-template-areas: 'gauge' 'primary';
  }

  .gauge-cell {
    grid-area: gauge;
    min-height: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-lg);
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
  }

  .primary-grid {
    grid-area: primary;
    min-height: 0;
    display: grid;
    grid-template-columns: 1fr 1fr;
    grid-template-rows: 1fr 1fr;
    gap: 8px;
  }

  .secondary-strip {
    grid-area: strip;
    min-height: 0;
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 8px;
  }

  /* ── Graphs / Retro ── */
  .graphs {
    position: relative;
    height: 100%;
  }

  .retro {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  /* ── Control bar: never scrolls away ── */
  .controls {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-md);
    border-radius: var(--radius-lg);
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
  }

  .control-rows {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }

  .controls.merged .control-rows {
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-xl);
  }

  .row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-sm) var(--space-md);
  }

  .row-trainer {
    flex: 1;
    min-width: 0;
  }

  .controls.merged .row-trainer {
    justify-content: flex-end;
  }

  .row-trainer :global(.zone-status) {
    flex: 1;
  }

  .session-btn {
    min-width: 140px;
  }
</style>
