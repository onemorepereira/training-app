<script lang="ts">
  import PowerGauge from '$lib/components/PowerGauge.svelte';
  import MetricCard from '$lib/components/MetricCard.svelte';
  import MetricsChart from '$lib/components/MetricsChart.svelte';
  import TrainerControl from '$lib/components/TrainerControl.svelte';
  import ConnectionHealth from '$lib/components/ConnectionHealth.svelte';
  import { currentPower, currentHR, currentCadence, currentSpeed, liveMetrics } from '$lib/stores/sensor';
  import { sessionActive, sessionPaused, sessionId, dashboardView } from '$lib/stores/session';
  import { autoSessionEnabled, autoSessionCountdown } from '$lib/stores/autoSession';
  import { trainerConnected } from '$lib/stores/devices';
  import { unitSystem, formatSpeed, speedUnit } from '$lib/stores/units';
  import { trainerError } from '$lib/stores/trainer';
  import { api, extractError, type SessionSummary } from '$lib/tauri';
  import ActivityModal from '$lib/components/ActivityModal.svelte';
  import { formatDuration } from '$lib/utils/format';

  let error = $state('');
  let postRideSession = $state<SessionSummary | null>(null);

  async function toggleSession() {
    error = '';
    try {
      if ($sessionActive) {
        const result = await api.stopSession();
        sessionActive.set(false);
        sessionId.set(null);
        sessionPaused.set(false);
        if (result) {
          postRideSession = result;
        }
      } else {
        const id = await api.startSession();
        sessionId.set(id);
        sessionActive.set(true);
        sessionPaused.set(false);
      }
    } catch (e) {
      error = extractError(e);
    }
  }

  async function togglePause() {
    error = '';
    try {
      if ($sessionPaused) {
        await api.resumeSession();
        sessionPaused.set(false);
      } else {
        await api.pauseSession();
        sessionPaused.set(true);
      }
    } catch (e) {
      error = extractError(e);
    }
  }

  function toggleFullscreen() {
    window.dispatchEvent(new CustomEvent('toggle-fullscreen'));
  }
</script>

<div class="dashboard">
  {#if error}
    <div class="error">{error}</div>
  {/if}

  <div class="dash-header">
    <div class="view-toggle">
      <button class="toggle-tab" class:active={$dashboardView === 'gauges'} onclick={() => $dashboardView = 'gauges'}>
        Gauges
      </button>
      <button class="toggle-tab" class:active={$dashboardView === 'graphs'} onclick={() => $dashboardView = 'graphs'}>
        Graphs
      </button>
    </div>
    <button class="fullscreen-btn" onclick={toggleFullscreen} aria-label="Toggle fullscreen (Esc)">
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <polyline points="15 3 21 3 21 9"/>
        <polyline points="9 21 3 21 3 15"/>
        <line x1="21" y1="3" x2="14" y2="10"/>
        <line x1="3" y1="21" x2="10" y2="14"/>
      </svg>
    </button>
  </div>

  <div class="dash-main">
    {#if $dashboardView === 'gauges'}
      <div class="gauge-section">
        <PowerGauge power={$currentPower} />
      </div>

      <div class="metrics-grid">
        <MetricCard label="Heart Rate" value={$currentHR} unit="bpm" accent="var(--danger)" size="lg" />
        <MetricCard label="Cadence" value={$currentCadence != null ? Math.round($currentCadence) : null} unit="rpm" size="lg" />
        <MetricCard label="Speed" value={$currentSpeed != null ? formatSpeed($currentSpeed, $unitSystem) : null} unit={$speedUnit} size="lg" />
        <MetricCard
          label="Time"
          value={$liveMetrics ? formatDuration($liveMetrics.elapsed_secs) : '--'}
          size="lg"
        />
      </div>
    {:else}
      <div class="chart-section">
        <MetricsChart />
      </div>
    {/if}
  </div>

  <div class="dash-secondary">
    <MetricCard
      label="3s Power"
      value={$liveMetrics?.avg_power_3s != null ? Math.round($liveMetrics.avg_power_3s) : null}
      unit="W"
      size="sm"
    />
    <MetricCard
      label="NP"
      value={$liveMetrics?.normalized_power != null ? Math.round($liveMetrics.normalized_power) : null}
      unit="W"
      size="sm"
    />
    <MetricCard
      label="TSS"
      value={$liveMetrics?.tss != null ? $liveMetrics.tss.toFixed(1) : null}
      size="sm"
    />
    <MetricCard
      label="IF"
      value={$liveMetrics?.intensity_factor != null ? $liveMetrics.intensity_factor.toFixed(2) : null}
      size="sm"
    />
  </div>

  <ConnectionHealth />

  <div class="dash-controls">
    <div class="controls-bar">
      <div class="session-btn-wrap">
        {#if $autoSessionCountdown != null && !$sessionActive}
          <span class="countdown-badge">Starting in {$autoSessionCountdown}...</span>
        {/if}
        <button
          class="btn-session"
          class:stop={$sessionActive}
          onclick={toggleSession}
        >
          {#if $sessionActive}
            <span class="btn-icon">&#x25A0;</span> Stop
          {:else}
            <span class="btn-icon">&#x25B6;</span> Start
          {/if}
        </button>
      </div>
      {#if $sessionActive}
        <button class="btn-pause" onclick={togglePause}>
          {#if $sessionPaused}
            <span class="btn-icon">&#x25B6;</span> Resume
          {:else}
            <span class="btn-icon">&#x23F8;</span> Pause
          {/if}
        </button>
      {/if}
      <label class="auto-toggle">
        <input type="checkbox" bind:checked={$autoSessionEnabled} />
        <span class="auto-toggle-label">Auto</span>
      </label>
      {#if $trainerConnected}
        <div class="controls-divider"></div>
        <TrainerControl />
      {/if}
    </div>
  </div>

  {#if $trainerError}
    <div class="trainer-error">
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="10"/>
        <line x1="12" y1="8" x2="12" y2="12"/>
        <line x1="12" y1="16" x2="12.01" y2="16"/>
      </svg>
      <span>{$trainerError}</span>
    </div>
  {/if}
</div>

{#if postRideSession}
  <ActivityModal
    session={postRideSession}
    mode="post-ride"
    onSave={async (title, activityType, rpe, notes) => {
      const id = postRideSession?.id;
      if (!id) return;
      try {
        await api.updateSessionMetadata(id, title, activityType, rpe, notes);
      } catch (e) {
        error = extractError(e);
      }
      postRideSession = null;
    }}
    onClose={() => { postRideSession = null; }}
  />
{/if}

<style>
  .dashboard {
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
  }

  .dash-header {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-md);
  }

  .view-toggle {
    display: flex;
    gap: 2px;
    background: var(--bg-body);
    border-radius: var(--radius-md);
    padding: 3px;
    width: fit-content;
  }

  .toggle-tab {
    flex: 1;
    padding: var(--space-sm) var(--space-md);
    border: none;
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
    border-radius: 6px;
    transition: all var(--transition-fast);
  }

  .toggle-tab:hover {
    color: var(--text-secondary);
  }

  .toggle-tab.active {
    background: var(--bg-elevated);
    color: var(--accent);
    box-shadow: var(--shadow-sm);
  }

  .fullscreen-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    transition: all var(--transition-fast);
    position: relative;
  }

  .fullscreen-btn:hover {
    color: var(--text-primary);
    border-color: var(--border-default);
    background: var(--bg-hover);
  }

  .gauge-section {
    height: clamp(240px, 40vh, 500px);
    position: relative;
  }

  .chart-section {
    height: clamp(240px, 40vh, 500px);
    position: relative;
  }

  .metrics-grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: var(--space-md);
  }

  .dash-secondary {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: var(--space-sm);
    opacity: 0.85;
  }

  @media (min-width: 640px) {
    .metrics-grid {
      grid-template-columns: repeat(4, 1fr);
    }
    .dash-secondary {
      grid-template-columns: repeat(4, 1fr);
    }
  }

  .dash-controls {
    border-top: 1px solid var(--border-subtle);
    padding: var(--space-md) 0 0;
  }

  .controls-bar {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-md);
    flex-wrap: wrap;
    position: relative;
  }

  .controls-divider {
    width: 1px;
    height: 28px;
    background: var(--border-default);
    flex-shrink: 0;
  }

  .error {
    padding: var(--space-md);
    background: rgba(244, 67, 54, 0.08);
    border: 1px solid rgba(244, 67, 54, 0.3);
    border-radius: var(--radius-md);
    color: var(--danger);
    font-size: var(--text-base);
    animation: slide-up 200ms ease;
    flex-shrink: 0;
  }

  .btn-session {
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-md) var(--space-2xl);
    border: none;
    border-radius: var(--radius-lg);
    font-size: var(--text-lg);
    font-weight: 600;
    cursor: pointer;
    background: var(--success);
    color: white;
    transition: all var(--transition-fast);
    box-shadow: 0 2px 8px rgba(76, 175, 80, 0.3);
  }

  .btn-session:hover {
    transform: translateY(-1px);
    box-shadow: 0 4px 12px rgba(76, 175, 80, 0.4);
  }

  .btn-session:active {
    transform: translateY(0);
  }

  .btn-session.stop {
    background: var(--danger);
    box-shadow: 0 2px 8px rgba(244, 67, 54, 0.3);
  }

  .btn-session.stop:hover {
    box-shadow: 0 4px 12px rgba(244, 67, 54, 0.4);
  }

  .btn-pause {
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-md) var(--space-xl);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-lg);
    font-size: var(--text-lg);
    font-weight: 600;
    cursor: pointer;
    background: var(--bg-elevated);
    color: var(--text-primary);
    transition: all var(--transition-fast);
  }

  .btn-pause:hover {
    border-color: var(--text-muted);
    background: var(--bg-hover);
  }

  .btn-icon {
    font-size: 0.8em;
  }

  .session-btn-wrap {
    position: relative;
    display: inline-flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-xs);
  }

  .countdown-badge {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--accent);
    animation: pulse 1s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.5; }
  }

  .auto-toggle {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    cursor: pointer;
    user-select: none;
    padding: var(--space-sm) var(--space-md);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
    transition: all var(--transition-fast);
  }

  .auto-toggle:hover {
    border-color: var(--border-strong);
  }

  .auto-toggle input[type="checkbox"] {
    accent-color: var(--accent);
    width: 16px;
    height: 16px;
    cursor: pointer;
  }

  .auto-toggle-label {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-secondary);
  }

  .trainer-error {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-md);
    background: rgba(244, 67, 54, 0.08);
    border: 1px solid rgba(244, 67, 54, 0.2);
    border-radius: var(--radius-md);
    color: var(--danger);
    font-size: var(--text-sm);
    animation: slide-up 150ms ease;
  }

  .trainer-error svg {
    flex-shrink: 0;
    opacity: 0.7;
  }

  .trainer-error span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
