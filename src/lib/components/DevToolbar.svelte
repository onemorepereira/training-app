<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type SimProfile } from '$lib/tauri';

  let status = $state<'Stopped' | 'Running'>('Stopped');
  let profile = $state<SimProfile>('SteadyState');
  let hidden = $state(false);

  const profiles: SimProfile[] = ['SteadyState', 'Intervals', 'Ramp', 'Stochastic'];

  async function refresh() {
    try {
      const res = await api.simStatus();
      status = res.status;
      profile = res.profile;
    } catch {
      hidden = true;
    }
  }

  async function toggle() {
    if (status === 'Running') {
      await api.simStop();
    } else {
      await api.simStart(profile);
    }
    await refresh();
  }

  onMount(() => {
    refresh();
  });
</script>

{#if !hidden}
  <div class="dev-toolbar">
    <span class="dev-label">SIM</span>
    <select bind:value={profile} disabled={status === 'Running'}>
      {#each profiles as p}
        <option value={p}>{p}</option>
      {/each}
    </select>
    <button onclick={toggle} class:running={status === 'Running'}>
      {status === 'Running' ? 'Stop' : 'Start'}
    </button>
  </div>
{/if}

<style>
  .dev-toolbar {
    position: fixed;
    top: 6px;
    right: 6px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px;
    background: var(--bg-elevated);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-sm);
    z-index: 9999;
    font-size: 11px;
    opacity: 0.55;
    transition: opacity 150ms;
  }

  .dev-toolbar:hover {
    opacity: 1;
  }

  .dev-label {
    color: var(--accent);
    font-weight: 700;
    font-size: 10px;
    letter-spacing: 0.05em;
  }

  select {
    background: var(--bg-surface);
    color: var(--text-primary);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-sm);
    padding: 1px 4px;
    font-size: inherit;
  }

  select:disabled {
    opacity: 0.5;
  }

  button {
    background: var(--accent-soft);
    color: var(--accent);
    border: none;
    border-radius: var(--radius-sm);
    padding: 1px 8px;
    font-size: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  button.running {
    background: rgba(255, 77, 109, 0.3);
  }

  button:hover {
    background: var(--accent);
    color: var(--bg-surface);
  }
</style>
