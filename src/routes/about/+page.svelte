<script lang="ts">
  import { onMount } from 'svelte';
  import { getVersion, getTauriVersion } from '@tauri-apps/api/app';
  import { openUrl } from '@tauri-apps/plugin-opener';

  let appVersion = $state('...');
  let tauriVersion = $state('...');

  onMount(async () => {
    [appVersion, tauriVersion] = await Promise.all([getVersion(), getTauriVersion()]);
  });
</script>

<div class="page">
  <h1>About</h1>

  <section class="section">
    <h2 class="section-title">App</h2>
    <div class="field-group">
      <div class="field">
        <span class="field-label">Version</span>
        <span class="field-value">{appVersion}</span>
      </div>
      <div class="field">
        <span class="field-label">Tauri</span>
        <span class="field-value">{tauriVersion}</span>
      </div>
      <div class="field">
        <span class="field-label">License</span>
        <span class="field-value">GPL-3.0</span>
      </div>
    </div>
  </section>

  <section class="section">
    <h2 class="section-title">Author</h2>
    <div class="field-group">
      <div class="field">
        <span class="field-label">GitHub</span>
        <button class="link" onclick={() => openUrl('https://github.com/onemorepereira')}>onemorepereira</button>
      </div>
    </div>
  </section>

  <section class="section">
    <h2 class="section-title">Links</h2>
    <div class="field-group">
      <div class="field">
        <span class="field-label">Source Code</span>
        <button class="link" onclick={() => openUrl('https://github.com/onemorepereira/training-app')}>GitHub</button>
      </div>
      <div class="field">
        <span class="field-label">Report a Bug</span>
        <button class="link" onclick={() => openUrl('https://github.com/onemorepereira/training-app/issues')}>Issues</button>
      </div>
    </div>
  </section>
</div>

<style>
  .page {
    max-width: 960px;
  }

  h1 {
    margin: 0 0 var(--space-xl);
    font-size: var(--text-2xl);
    font-weight: 800;
  }

  .section {
    margin-bottom: var(--space-3xl);
  }

  .section-title {
    font-size: var(--text-sm);
    font-weight: 700;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin: 0 0 var(--space-md);
  }

  .field-group {
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    max-width: 480px;
  }

  .field {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-md);
    background: var(--bg-surface);
    border-radius: var(--radius-md);
    border: 1px solid var(--border-subtle);
  }

  .field-label {
    color: var(--text-secondary);
    font-size: var(--text-base);
    font-weight: 500;
  }

  .field-value {
    font-family: var(--font-data);
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--text-primary);
  }

  .link {
    background: none;
    border: none;
    padding: 0;
    font-family: inherit;
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--accent);
    cursor: pointer;
    transition: opacity var(--transition-fast);
  }

  .link:hover {
    opacity: 0.8;
  }
</style>
