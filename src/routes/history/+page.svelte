<script lang="ts">
  import { onMount } from 'svelte';
  import type { SessionSummary } from '$lib/tauri';
  import { api, extractError } from '$lib/tauri';
  import ActivityModal from '$lib/components/ActivityModal.svelte';

  let sessions = $state<SessionSummary[]>([]);
  let loading = $state(true);
  let error = $state('');
  let exportingId = $state<string | null>(null);
  let editSession = $state<SessionSummary | null>(null);
  let viewMode = $state<'cards' | 'table'>(
    (typeof localStorage !== 'undefined' && localStorage.getItem('historyView') as 'cards' | 'table') || 'cards'
  );
  let sortColumn = $state<string>('start_time');
  let sortAsc = $state(false);

  onMount(async () => {
    try {
      sessions = await api.listSessions();
    } catch (e) {
      error = extractError(e);
    } finally {
      loading = false;
    }
  });

  async function refreshSessions() {
    try {
      sessions = await api.listSessions();
    } catch (e) {
      error = extractError(e);
    }
  }

  function setViewMode(mode: 'cards' | 'table') {
    viewMode = mode;
    localStorage.setItem('historyView', mode);
  }

  function autoTitle(startTime: string): string {
    const hour = new Date(startTime).getHours();
    if (hour >= 5 && hour < 12) return 'Morning Ride';
    if (hour >= 12 && hour < 17) return 'Afternoon Ride';
    if (hour >= 17 && hour < 21) return 'Evening Ride';
    return 'Night Ride';
  }

  function displayTitle(session: SessionSummary): string {
    return session.title ?? autoTitle(session.start_time);
  }

  const TYPE_LABELS: Record<string, string> = {
    endurance: 'Endurance', intervals: 'Intervals', threshold: 'Threshold',
    sweet_spot: 'Sweet Spot', vo2max: 'VO2max', sprint: 'Sprint',
    tempo: 'Tempo', recovery: 'Recovery', race: 'Race', test: 'Test',
    warmup: 'Warmup', group_ride: 'Group Ride', free_ride: 'Free Ride', other: 'Other',
  };

  function formatDate(iso: string): string {
    const d = new Date(iso);
    return d.toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric', year: 'numeric' });
  }

  function formatDateShort(iso: string): string {
    const d = new Date(iso);
    return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  }

  function formatTime(iso: string): string {
    const d = new Date(iso);
    return d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
  }

  function formatDuration(secs: number): string {
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = secs % 60;
    if (h > 0) return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
    return `${m}:${String(s).padStart(2, '0')}`;
  }

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

  function toggleSort(column: string) {
    if (sortColumn === column) {
      sortAsc = !sortAsc;
    } else {
      sortColumn = column;
      sortAsc = false;
    }
  }

  function getSortValue(session: SessionSummary, column: string): number | string {
    switch (column) {
      case 'start_time': return session.start_time;
      case 'title': return displayTitle(session).toLowerCase();
      case 'activity_type': return session.activity_type ?? '';
      case 'duration_secs': return session.duration_secs;
      case 'avg_power': return session.avg_power ?? -1;
      case 'normalized_power': return session.normalized_power ?? -1;
      case 'tss': return session.tss ?? -1;
      case 'intensity_factor': return session.intensity_factor ?? -1;
      case 'avg_hr': return session.avg_hr ?? -1;
      case 'rpe': return session.rpe ?? -1;
      default: return 0;
    }
  }

  let sortedSessions = $derived(
    [...sessions].sort((a, b) => {
      const aVal = getSortValue(a, sortColumn);
      const bVal = getSortValue(b, sortColumn);
      const cmp = aVal < bVal ? -1 : aVal > bVal ? 1 : 0;
      return sortAsc ? cmp : -cmp;
    })
  );

  async function exportFit(e: Event, sessionId: string) {
    e.stopPropagation();
    exportingId = sessionId;
    try {
      await api.exportSessionFit(sessionId);
    } catch (err) {
      error = extractError(err);
    } finally {
      exportingId = null;
    }
  }
</script>

<div class="page">
  <div class="page-header">
    <h1>History</h1>
    {#if sessions.length > 0}
      <div class="view-toggle">
        <button
          class="toggle-btn"
          class:active={viewMode === 'cards'}
          onclick={() => setViewMode('cards')}
          aria-label="Card view"
        >
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="3" width="7" height="7"/><rect x="14" y="3" width="7" height="7"/><rect x="3" y="14" width="7" height="7"/><rect x="14" y="14" width="7" height="7"/>
          </svg>
        </button>
        <button
          class="toggle-btn"
          class:active={viewMode === 'table'}
          onclick={() => setViewMode('table')}
          aria-label="Table view"
        >
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><line x1="3" y1="6" x2="3.01" y2="6"/><line x1="3" y1="12" x2="3.01" y2="12"/><line x1="3" y1="18" x2="3.01" y2="18"/>
          </svg>
        </button>
      </div>
    {/if}
  </div>

  {#if error}
    <div class="error">{error}</div>
  {/if}

  {#if loading}
    <p class="empty">Loading sessions...</p>
  {:else if sessions.length === 0}
    <p class="empty">No sessions yet. Complete a ride to see it here.</p>
  {:else if viewMode === 'cards'}
    <div class="sessions">
      {#each sortedSessions as session}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="card" onclick={() => editSession = session} onkeydown={(e) => e.key === 'Enter' && (editSession = session)} tabindex="0" role="button">
          <div class="card-header">
            <div class="card-title-row">
              <span class="card-title">{displayTitle(session)}</span>
              {#if session.activity_type}
                <span class="type-badge">{TYPE_LABELS[session.activity_type] ?? session.activity_type}</span>
              {/if}
              {#if session.rpe != null}
                <span class="rpe-badge" style="color: {rpeColor(session.rpe)}">RPE {session.rpe}</span>
              {/if}
            </div>
            <div class="card-subtitle">
              <span class="date">{formatDate(session.start_time)}</span>
              <span class="time">{formatTime(session.start_time)}</span>
            </div>
          </div>
          <div class="card-stats">
            <div class="stat">
              <span class="stat-value">{formatDuration(session.duration_secs)}</span>
              <span class="stat-label">Duration</span>
            </div>
            {#if session.avg_power != null}
              <div class="stat">
                <span class="stat-value">{session.avg_power}<span class="stat-unit">W</span></span>
                <span class="stat-label">Avg Power</span>
              </div>
            {/if}
            {#if session.normalized_power != null}
              <div class="stat">
                <span class="stat-value">{session.normalized_power}<span class="stat-unit">W</span></span>
                <span class="stat-label">NP</span>
              </div>
            {/if}
            {#if session.tss != null}
              <div class="stat">
                <span class="stat-value">{Math.round(session.tss)}</span>
                <span class="stat-label">TSS</span>
              </div>
            {/if}
            {#if session.avg_hr != null}
              <div class="stat">
                <span class="stat-value">{session.avg_hr}<span class="stat-unit">bpm</span></span>
                <span class="stat-label">Avg HR</span>
              </div>
            {/if}
          </div>
          {#if session.notes}
            <div class="card-notes">{session.notes.length > 60 ? session.notes.slice(0, 60) + '...' : session.notes}</div>
          {/if}
          <div class="card-actions">
            <button
              class="export-btn"
              disabled={exportingId === session.id}
              onclick={(e) => exportFit(e, session.id)}
            >
              {exportingId === session.id ? 'Exporting...' : 'Export FIT'}
            </button>
          </div>
        </div>
      {/each}
    </div>
  {:else}
    <div class="table-wrap">
      <table class="sessions-table">
        <thead>
          <tr>
            {#each [
              { key: 'start_time', label: 'Date' },
              { key: 'title', label: 'Title' },
              { key: 'activity_type', label: 'Type' },
              { key: 'duration_secs', label: 'Duration' },
              { key: 'avg_power', label: 'Avg W' },
              { key: 'normalized_power', label: 'NP' },
              { key: 'tss', label: 'TSS' },
              { key: 'intensity_factor', label: 'IF' },
              { key: 'avg_hr', label: 'Avg HR' },
              { key: 'rpe', label: 'RPE' },
            ] as col}
              <th onclick={() => toggleSort(col.key)} class="sortable">
                {col.label}
                {#if sortColumn === col.key}
                  <span class="sort-arrow">{sortAsc ? '\u25B2' : '\u25BC'}</span>
                {/if}
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each sortedSessions as session}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <tr class="table-row" onclick={() => editSession = session} onkeydown={(e) => e.key === 'Enter' && (editSession = session)} tabindex="0">
              <td class="col-date">{formatDateShort(session.start_time)}</td>
              <td class="col-title">{displayTitle(session)}</td>
              <td>
                {#if session.activity_type}
                  <span class="type-badge small">{TYPE_LABELS[session.activity_type] ?? session.activity_type}</span>
                {:else}
                  <span class="text-muted">-</span>
                {/if}
              </td>
              <td class="col-num">{formatDuration(session.duration_secs)}</td>
              <td class="col-num">{session.avg_power ?? '-'}</td>
              <td class="col-num">{session.normalized_power ?? '-'}</td>
              <td class="col-num">{session.tss != null ? Math.round(session.tss) : '-'}</td>
              <td class="col-num">{session.intensity_factor != null ? session.intensity_factor.toFixed(2) : '-'}</td>
              <td class="col-num">{session.avg_hr ?? '-'}</td>
              <td class="col-num">
                {#if session.rpe != null}
                  <span style="color: {rpeColor(session.rpe)}; font-weight: 700">{session.rpe}</span>
                {:else}
                  -
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>

{#if editSession}
  <ActivityModal
    session={editSession}
    mode="edit"
    onSave={async (title, activityType, rpe, notes) => {
      try {
        await api.updateSessionMetadata(editSession!.id, title, activityType, rpe, notes);
        await refreshSessions();
      } catch (e) {
        error = extractError(e);
      }
      editSession = null;
    }}
    onDelete={async () => {
      try {
        await api.deleteSession(editSession!.id);
        await refreshSessions();
      } catch (e) {
        error = extractError(e);
      }
      editSession = null;
    }}
    onClose={() => { editSession = null; }}
  />
{/if}

<style>
  .page {
    max-width: 960px;
  }

  .page-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-xl);
  }

  h1 {
    margin: 0;
    font-size: var(--text-2xl);
    font-weight: 800;
  }

  .view-toggle {
    display: flex;
    gap: 2px;
    background: var(--bg-body);
    border-radius: var(--radius-md);
    padding: 3px;
  }

  .toggle-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 28px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .toggle-btn:hover { color: var(--text-secondary); }
  .toggle-btn.active {
    background: var(--bg-elevated);
    color: var(--accent);
    box-shadow: var(--shadow-sm);
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

  .empty {
    color: var(--text-muted);
    font-size: var(--text-base);
    padding: var(--space-3xl) 0;
    text-align: center;
  }

  /* --- Card View --- */
  .sessions {
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
  }

  .card {
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-lg);
    padding: var(--space-lg);
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .card:hover {
    border-color: var(--border-default);
    background: var(--bg-elevated);
  }

  .card:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .card-header {
    margin-bottom: var(--space-md);
  }

  .card-title-row {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    flex-wrap: wrap;
    margin-bottom: 4px;
  }

  .card-title {
    font-size: var(--text-base);
    font-weight: 700;
    color: var(--text-primary);
  }

  .card-subtitle {
    display: flex;
    align-items: baseline;
    gap: var(--space-md);
  }

  .date {
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }

  .time {
    font-size: var(--text-xs);
    color: var(--text-muted);
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

  .type-badge.small {
    font-size: 0.625rem;
    padding: 1px 6px;
  }

  .rpe-badge {
    font-size: var(--text-xs);
    font-weight: 700;
    font-family: var(--font-data);
  }

  .card-stats {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-lg);
  }

  .stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .stat-value {
    font-family: var(--font-data);
    font-size: var(--text-lg);
    font-weight: 700;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .stat-unit {
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--text-muted);
    margin-left: 2px;
  }

  .stat-label {
    font-size: var(--text-xs);
    color: var(--text-faint);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    font-weight: 600;
  }

  .card-notes {
    margin-top: var(--space-sm);
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-style: italic;
  }

  .card-actions {
    margin-top: var(--space-md);
    padding-top: var(--space-md);
    border-top: 1px solid var(--border-subtle);
  }

  .export-btn {
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

  .export-btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
    background: var(--accent-soft);
  }

  .export-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  /* --- Table View --- */
  .table-wrap {
    overflow-x: auto;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-lg);
  }

  .sessions-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-sm);
  }

  .sessions-table th {
    padding: var(--space-sm) var(--space-md);
    text-align: left;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    border-bottom: 1px solid var(--border-default);
    background: var(--bg-elevated);
    white-space: nowrap;
    user-select: none;
  }

  .sessions-table th.sortable {
    cursor: pointer;
  }

  .sessions-table th.sortable:hover {
    color: var(--text-primary);
  }

  .sort-arrow {
    font-size: 0.6em;
    margin-left: 2px;
    color: var(--accent);
  }

  .sessions-table td {
    padding: var(--space-sm) var(--space-md);
    border-bottom: 1px solid var(--border-subtle);
    color: var(--text-secondary);
    white-space: nowrap;
  }

  .table-row {
    cursor: pointer;
    transition: background var(--transition-fast);
  }

  .table-row:hover {
    background: var(--bg-hover);
  }

  .table-row:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .col-date {
    color: var(--text-primary);
    font-weight: 600;
  }

  .col-title {
    color: var(--text-primary);
    font-weight: 600;
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .col-num {
    font-family: var(--font-data);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  .text-muted {
    color: var(--text-faint);
  }
</style>
