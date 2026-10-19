<script lang="ts">
  import { onMount } from 'svelte';
  import { getVersion, getTauriVersion } from '@tauri-apps/api/app';
  import { openUrl } from '@tauri-apps/plugin-opener';

  let appVersion = $state('…');
  let tauriVersion = $state('…');

  onMount(async () => {
    [appVersion, tauriVersion] = await Promise.all([
      getVersion().catch(() => 'Unknown'),
      getTauriVersion().catch(() => 'Unknown'),
    ]);
  });

  type LinkRow = { label: string; text: string; url: string };

  const groups: { title: string; rows: LinkRow[] }[] = [
    {
      title: 'Project',
      rows: [
        { label: 'Source code', text: 'GitHub', url: 'https://github.com/onemorepereira/training-app' },
        { label: 'Report a bug', text: 'Issues', url: 'https://github.com/onemorepereira/training-app/issues' },
        { label: 'Author', text: 'onemorepereira', url: 'https://github.com/onemorepereira' },
      ],
    },
    {
      title: 'Frameworks and libraries',
      rows: [
        { label: 'App framework', text: 'Tauri', url: 'https://tauri.app' },
        { label: 'UI framework', text: 'Svelte', url: 'https://svelte.dev' },
        { label: 'Charts', text: 'Apache ECharts', url: 'https://echarts.apache.org' },
        { label: 'Bluetooth', text: 'btleplug', url: 'https://github.com/deviceplug/btleplug' },
        { label: 'Database', text: 'SQLx', url: 'https://github.com/launchbadge/sqlx' },
        { label: 'Async runtime', text: 'Tokio', url: 'https://tokio.rs' },
      ],
    },
    {
      title: 'Fonts',
      rows: [{ label: 'Numbers', text: 'IBM Plex Mono', url: 'https://github.com/IBM/plex' }],
    },
    {
      title: 'Protocols and standards',
      rows: [
        { label: 'ANT+', text: 'Garmin ANT+', url: 'https://www.thisisant.com' },
        { label: 'BLE FTMS', text: 'Bluetooth FTMS', url: 'https://www.bluetooth.com/specifications/specs/fitness-machine-service-1-0' },
        { label: 'Activity files', text: 'Garmin FIT SDK', url: 'https://developer.garmin.com/fit/overview' },
      ],
    },
  ];

  function open(e: MouseEvent, url: string) {
    e.preventDefault();
    openUrl(url).catch(() => {});
  }
</script>

<div class="page" data-page="about">
  <header class="page-header">
    <h1 class="page-title">About</h1>
  </header>

  <div class="columns">
    <section class="card" aria-labelledby="app-h">
      <h2 class="section-title" id="app-h">Training App</h2>
      <dl class="facts">
        <div><dt>Version</dt><dd class="data">{appVersion}</dd></div>
        <div><dt>Tauri</dt><dd class="data">{tauriVersion}</dd></div>
        <div><dt>License</dt><dd>GPL-3.0</dd></div>
      </dl>
      <p class="note">The interface uses your system's font. Numbers use IBM Plex Mono, which ships with the app, so nothing is downloaded at startup.</p>
    </section>

    <div class="link-groups">
      {#each groups as g (g.title)}
        <section class="card" aria-label={g.title}>
          <h2 class="section-title">{g.title}</h2>
          <ul class="links">
            {#each g.rows as r (r.url)}
              <li>
                <a class="link-row" href={r.url} onclick={(e) => open(e, r.url)}>
                  <span class="link-label">{r.label}</span>
                  <span class="link-text">{r.text}</span>
                  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 3h6v6M10 14 21 3M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/></svg>
                  <span class="sr-only">(opens in your browser)</span>
                </a>
              </li>
            {/each}
          </ul>
        </section>
      {/each}
    </div>
  </div>
</div>

<style>
  .columns {
    display: grid;
    grid-template-columns: 1fr;
    gap: var(--space-xl);
    align-items: start;
  }

  @media (min-width: 1280px) {
    .columns {
      grid-template-columns: minmax(300px, 2fr) 3fr;
    }
  }

  .card {
    padding: var(--space-xl);
    border-radius: var(--radius-lg);
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
  }

  .link-groups {
    display: flex;
    flex-direction: column;
    gap: var(--space-lg);
  }

  .facts {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }

  .facts div {
    display: flex;
    justify-content: space-between;
    gap: var(--space-lg);
  }

  .facts dt {
    color: var(--text-secondary);
  }

  .facts dd {
    margin: 0;
    color: var(--text-primary);
  }

  .note {
    margin: var(--space-lg) 0 0;
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }

  .links {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .link-row {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    min-height: 40px;
    padding: 0 var(--space-sm);
    margin: 0 calc(-1 * var(--space-sm));
    border-radius: var(--radius-md);
    color: var(--text-primary);
    text-decoration: none;
  }

  .link-row:hover {
    background: var(--bg-hover);
  }

  .link-label {
    flex: 1;
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  .link-text {
    color: var(--accent);
    font-weight: 600;
  }

  .link-row svg {
    color: var(--text-muted);
  }
</style>
