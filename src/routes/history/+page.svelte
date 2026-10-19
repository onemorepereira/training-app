<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import StateView from '$lib/ui/StateView.svelte';
  import Notice from '$lib/ui/Notice.svelte';
  import type { SessionSummary } from '$lib/tauri';
  import { api, extractError } from '$lib/tauri';
  import { formatDuration, formatDateLong as formatDate, formatDateShort, formatTime } from '$lib/utils/format';
  import { TYPE_LABELS, displayTitle, rpeColor } from '$lib/utils/session';

  let sessions = $state<SessionSummary[]>([]);
  let loading = $state(true);
  /** Failure to load the list (page-level error state). */
  let loadError = $state('');
  /** Failure of an action on the list (export), shown as a dismissible notice. */
  let error = $state('');
  let exportingId = $state<string | null>(null);
  let viewMode = $state<'cards' | 'table'>(
    (typeof localStorage !== 'undefined' && localStorage.getItem('historyView') as 'cards' | 'table') || 'cards'
  );
  let sortColumn = $state<string>('start_time');
  let sortAsc = $state(false);

  async function load() {
    loading = true;
    loadError = '';
    try {
      sessions = await api.listSessions();
    } catch (e) {
      loadError = extractError(e);
    } finally {
      loading = false;
    }
  }

  onMount(load);

  const VIEWS = [
    { value: 'cards', label: 'Cards' },
    { value: 'table', label: 'Table' },
  ] as const;

  /** Columns; `minor` ones hide on narrow windows so the table never scrolls sideways. */
  const COLUMNS = [
    { key: 'start_time', label: 'Date', minor: false, num: false },
    { key: 'title', label: 'Title', minor: false, num: false },
    { key: 'activity_type', label: 'Type', minor: true, num: false },
    { key: 'duration_secs', label: 'Duration', minor: false, num: true },
    { key: 'avg_power', label: 'Avg W', minor: true, num: true },
    { key: 'normalized_power', label: 'NP', minor: false, num: true },
    { key: 'tss', label: 'TSS', minor: false, num: true },
    { key: 'intensity_factor', label: 'IF', minor: true, num: true },
    { key: 'avg_hr', label: 'Avg HR', minor: true, num: true },
    { key: 'rpe', label: 'RPE', minor: false, num: true },
  ];

  function setViewMode(mode: 'cards' | 'table') {
    viewMode = mode;
    try {
      localStorage.setItem('historyView', mode);
    } catch {
      // Storage unavailable: the choice just isn't remembered.
    }
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
    error = '';
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

<div class="page-wide" data-page="history">
  <div class="page-header">
    <div class="heading">
      <h1 class="page-title">History</h1>
      {#if !loading && !loadError && sessions.length > 0}
        <span class="count">{sessions.length} ride{sessions.length === 1 ? '' : 's'}</span>
      {/if}
    </div>
    {#if !loading && !loadError && sessions.length > 0}
      <SegmentedControl label="Layout" options={VIEWS} value={viewMode} onchange={setViewMode} />
    {/if}
  </div>

  {#if error}
    <div class="notice-slot">
      <Notice tone="error" title="Couldn't export the ride" ondismiss={() => (error = '')}>{error}</Notice>
    </div>
  {/if}

  {#if loading}
    <StateView kind="loading" title="Loading your rides…" />
  {:else if loadError}
    <StateView
      kind="error"
      title="Couldn't load your rides"
      message="Your rides are safe; the list just couldn't be read right now."
      details={loadError}
      action={{ label: 'Retry', onclick: load }}
    />
  {:else if sessions.length === 0}
    <StateView
      kind="empty"
      title="No rides yet"
      message="Start a ride from the Ride screen and it will show up here."
      action={{ label: 'Go to Ride', onclick: () => goto('/') }}
    />
  {:else if viewMode === 'cards'}
    <div class="cards">
      {#each sortedSessions as session (session.id)}
        <article class="card">
          <div class="card-head">
            <h2 class="card-title"><a href="/history/{session.id}" class="card-link">{displayTitle(session)}</a></h2>
            <div class="badges">
              {#if session.activity_type}
                <span class="type-badge">{TYPE_LABELS[session.activity_type] ?? session.activity_type}</span>
              {/if}
              {#if session.rpe != null}
                <span class="rpe-badge data" style="color: {rpeColor(session.rpe)}">RPE {session.rpe}</span>
              {/if}
            </div>
            <div class="card-date">{formatDate(session.start_time)} · {formatTime(session.start_time)}</div>
          </div>

          <dl class="card-stats">
            <div class="stat">
              <dt>Duration</dt>
              <dd class="data">{formatDuration(session.duration_secs)}</dd>
            </div>
            <div class="stat">
              <dt>Avg power</dt>
              <dd class="data">{#if session.avg_power != null}{session.avg_power}<span class="stat-unit">W</span>{:else}–{/if}</dd>
            </div>
            <div class="stat">
              <dt>NP · TSS</dt>
              <dd class="data">{session.normalized_power ?? '–'}{#if session.normalized_power != null}<span class="stat-unit">W</span>{/if} · {session.tss != null ? Math.round(session.tss) : '–'}</dd>
            </div>
            <div class="stat">
              <dt>Avg HR</dt>
              <dd class="data">{#if session.avg_hr != null}{session.avg_hr}<span class="stat-unit">bpm</span>{:else}–{/if}</dd>
            </div>
          </dl>

          {#if session.notes}
            <p class="card-notes">{session.notes}</p>
          {/if}

          <div class="card-actions">
            <button
              type="button"
              class="btn btn-secondary export"
              disabled={exportingId === session.id}
              onclick={(e) => exportFit(e, session.id)}
            >
              {exportingId === session.id ? 'Exporting…' : 'Export FIT'}
            </button>
          </div>
        </article>
      {/each}
    </div>
  {:else}
    <div class="table-wrap">
      <table class="sessions-table">
        <thead>
          <tr>
            {#each COLUMNS as col (col.key)}
              <th
                class:minor={col.minor}
                class:num={col.num}
                aria-sort={sortColumn === col.key ? (sortAsc ? 'ascending' : 'descending') : 'none'}
              >
                <button type="button" class="sort-btn" onclick={() => toggleSort(col.key)}>
                  {col.label}
                  <svg class="sort-icon" class:active={sortColumn === col.key} class:asc={sortColumn === col.key && sortAsc} viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg>
                </button>
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each sortedSessions as session (session.id)}
            <tr class="table-row" onclick={() => goto('/history/' + session.id)}>
              <td class="col-date">{formatDateShort(session.start_time)}</td>
              <td class="col-title"><a href="/history/{session.id}" onclick={(e) => e.stopPropagation()}>{displayTitle(session)}</a></td>
              <td class="minor">
                {#if session.activity_type}
                  <span class="type-badge">{TYPE_LABELS[session.activity_type] ?? session.activity_type}</span>
                {:else}
                  <span class="none">–</span>
                {/if}
              </td>
              <td class="num data">{formatDuration(session.duration_secs)}</td>
              <td class="num data minor">{session.avg_power ?? '–'}</td>
              <td class="num data">{session.normalized_power ?? '–'}</td>
              <td class="num data">{session.tss != null ? Math.round(session.tss) : '–'}</td>
              <td class="num data minor">{session.intensity_factor != null ? session.intensity_factor.toFixed(2) : '–'}</td>
              <td class="num data minor">{session.avg_hr ?? '–'}</td>
              <td class="num data">
                {#if session.rpe != null}
                  <span style="color: {rpeColor(session.rpe)}; font-weight: 700">{session.rpe}</span>
                {:else}
                  <span class="none">–</span>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>

<style>
  .heading {
    display: flex;
    align-items: baseline;
    gap: var(--space-md);
  }

  .count {
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  .notice-slot {
    margin-bottom: var(--space-lg);
  }

  /* --- Cards --- */
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
    gap: var(--space-md);
  }

  .card {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    padding: var(--space-lg);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-lg);
    background: var(--bg-surface);
    transition: background var(--transition-fast), border-color var(--transition-fast);
  }

  .card:hover {
    border-color: var(--border-default);
    background: var(--bg-elevated);
  }

  .card:focus-within {
    border-color: var(--accent);
  }

  .card-head {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .card-title {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 700;
    line-height: 1.3;
  }

  /* Stretched link: the whole card opens the ride */
  .card-link {
    display: inline-flex;
    align-items: center;
    min-height: var(--control-h-sm);
    color: var(--text-primary);
    text-decoration: none;
  }

  .card-link::after {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: inherit;
  }

  .card-link:focus-visible {
    outline: none;
  }

  .card-link:focus-visible::after {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .badges {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-sm);
  }

  .badges:empty {
    display: none;
  }

  .card-date {
    color: var(--text-secondary);
    font-size: var(--text-sm);
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
    font-size: var(--text-xs);
    font-weight: 700;
  }

  .card-stats {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-md) var(--space-lg);
    margin: 0;
  }

  .stat dt {
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .stat dd {
    margin: 2px 0 0;
    font-size: 1.125rem;
    font-weight: 700;
    color: var(--text-primary);
    white-space: nowrap;
  }

  .stat-unit {
    margin-left: 3px;
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--text-muted);
  }

  .card-notes {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--text-sm);
    font-style: italic;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .card-actions {
    margin-top: auto;
    padding-top: var(--space-md);
    border-top: 1px solid var(--border-subtle);
  }

  /* Sits above the stretched link so it gets its own clicks */
  .export {
    position: relative;
    z-index: 1;
  }

  /* --- Table --- */
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
    padding: 0 var(--space-xs);
    border-bottom: 1px solid var(--border-default);
    background: var(--bg-elevated);
    text-align: left;
    white-space: nowrap;
  }

  .sessions-table th.num {
    text-align: right;
  }

  .sort-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-height: var(--control-h-sm);
    padding: 0 var(--space-sm);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-secondary);
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    cursor: pointer;
  }

  .sort-btn:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .sort-icon {
    opacity: 0;
    color: var(--accent);
    transition: transform var(--transition-fast);
  }

  .sort-btn:hover .sort-icon {
    opacity: 0.5;
  }

  .sort-icon.active {
    opacity: 1;
  }

  .sort-icon.asc {
    transform: rotate(180deg);
  }

  .sessions-table td {
    padding: var(--space-xs) var(--space-md);
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

  .col-date {
    color: var(--text-primary);
    font-weight: 600;
  }

  .col-title {
    min-width: 24ch;
    max-width: 40ch;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .col-title a {
    display: inline-flex;
    align-items: center;
    min-height: var(--control-h-sm);
    color: var(--text-primary);
    font-weight: 600;
    text-decoration: none;
  }

  .col-title a:hover {
    text-decoration: underline;
  }

  .num {
    text-align: right;
  }

  .none {
    color: var(--text-muted);
  }

  /* Narrow windows: drop the lower-priority columns instead of scrolling sideways */
  @media (max-width: 1279px) {
    .minor {
      display: none;
    }
  }
</style>
