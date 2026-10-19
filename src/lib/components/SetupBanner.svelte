<script lang="ts" module>
  /** Dismissed for this app session only (in memory); it comes back after a restart if setup is still incomplete. */
  let dismissedThisSession = $state(false);
</script>

<script lang="ts">
  import type { PrereqStatus } from '$lib/tauri';
  import { api, extractError } from '$lib/tauri';
  import { onMount } from 'svelte';
  import Notice from '$lib/ui/Notice.svelte';

  let status = $state<PrereqStatus | null>(null);
  let fixing = $state(false);
  let fixError = $state('');

  onMount(async () => {
    try {
      status = await api.checkPrerequisites();
    } catch {
      // Non-Linux or check failed — hide banner
      status = null;
    }
  });

  async function fixAll() {
    fixing = true;
    fixError = '';
    try {
      const result = await api.fixPrerequisites();
      status = result.status;
      if (!result.success) {
        fixError = result.message;
      }
    } catch (e) {
      fixError = extractError(e);
    } finally {
      fixing = false;
    }
  }

  let checks = $derived(
    status
      ? [
          { ok: status.udev_rules, label: 'ANT+ USB stick permissions (udev rules)' },
          { ok: status.bluez_installed, label: 'Bluetooth stack (BlueZ) installed' },
          { ok: status.bluetooth_service, label: 'Bluetooth service running' },
        ]
      : []
  );
</script>

{#if status && !status.all_met && !dismissedThisSession}
  <div class="setup">
    <Notice tone="warning" title="Finish Bluetooth/ANT+ setup" ondismiss={() => (dismissedThisSession = true)}>
      <ul class="checks">
        {#each checks as c (c.label)}
          <li class:ok={c.ok}>
            {#if c.ok}
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 6 9 17l-5-5"/></svg>
            {:else}
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" aria-hidden="true"><path d="M18 6 6 18M6 6l12 12"/></svg>
            {/if}
            <span class="sr-only">{c.ok ? 'Done:' : 'Missing:'}</span>{c.label}
          </li>
        {/each}
      </ul>
      {#if status.pkexec_available}
        <p class="explain">Fix setup will ask for your administrator password once.</p>
      {:else}
        <p class="explain">Your system can't ask for an administrator password here (pkexec isn't available). Run these in a terminal:</p>
        <code>sudo cp &lt;rules-file&gt; /etc/udev/rules.d/99-ant-usb.rules</code>
        <code>sudo systemctl enable --now bluetooth</code>
      {/if}
      {#if fixError}
        <p class="fix-error" role="alert">Setup didn't finish: {fixError}</p>
      {/if}
      {#snippet actions()}
        {#if status?.pkexec_available}
          <button type="button" class="btn btn-primary" onclick={fixAll} disabled={fixing}>
            {fixing ? 'Fixing…' : 'Fix setup'}
          </button>
        {/if}
      {/snippet}
    </Notice>
  </div>
{/if}

<style>
  .setup {
    margin-bottom: var(--space-lg);
  }

  .checks {
    list-style: none;
    margin: var(--space-xs) 0 var(--space-sm);
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: var(--text-sm);
    color: var(--text-primary);
  }

  .checks li {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }

  .checks svg {
    color: var(--danger-text);
    flex-shrink: 0;
  }

  .checks li.ok svg {
    color: var(--success);
  }

  .explain {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }

  code {
    display: block;
    margin-top: 4px;
    padding: 4px 8px;
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    font-family: var(--font-data);
    font-size: var(--text-xs);
    color: var(--text-primary);
    overflow-wrap: anywhere;
  }

  .fix-error {
    margin: var(--space-xs) 0 0;
    font-size: var(--text-sm);
    color: var(--danger-text);
  }
</style>
