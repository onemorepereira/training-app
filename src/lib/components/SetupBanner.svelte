<script lang="ts">
  import type { PrereqStatus } from '$lib/tauri';
  import { api, extractError } from '$lib/tauri';
  import { onMount } from 'svelte';

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
</script>

{#if status && !status.all_met}
  <div class="setup-banner">
    <div class="banner-header">
      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"/>
        <line x1="12" y1="9" x2="12" y2="13"/>
        <line x1="12" y1="17" x2="12.01" y2="17"/>
      </svg>
      <span>System setup required</span>
    </div>

    <div class="checks">
      <div class="check-item" class:pass={status.udev_rules}>
        <span class="check-icon">{status.udev_rules ? '\u2713' : '\u2717'}</span>
        <span>ANT+ USB rules</span>
      </div>
      <div class="check-item" class:pass={status.bluez_installed}>
        <span class="check-icon">{status.bluez_installed ? '\u2713' : '\u2717'}</span>
        <span>BlueZ installed</span>
      </div>
      <div class="check-item" class:pass={status.bluetooth_service}>
        <span class="check-icon">{status.bluetooth_service ? '\u2713' : '\u2717'}</span>
        <span>Bluetooth service</span>
      </div>
    </div>

    {#if fixError}
      <div class="fix-error">{fixError}</div>
    {/if}

    {#if status.pkexec_available}
      <button class="fix-btn" onclick={fixAll} disabled={fixing}>
        {#if fixing}
          <span class="fix-spinner"></span>
          Fixing...
        {:else}
          Fix All
        {/if}
      </button>
    {:else}
      <div class="manual-instructions">
        <span class="manual-label">pkexec not available. Fix manually:</span>
        <code>sudo cp &lt;rules-file&gt; /etc/udev/rules.d/99-ant-usb.rules</code>
        <code>sudo systemctl enable --now bluetooth</code>
      </div>
    {/if}
  </div>
{/if}

<style>
  .setup-banner {
    padding: var(--space-lg);
    margin-bottom: var(--space-lg);
    background: var(--bg-surface);
    border: 1px solid rgba(255, 183, 77, 0.25);
    border-radius: var(--radius-md);
    animation: slide-up 200ms ease;
  }

  .banner-header {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    color: var(--warning);
    font-weight: 700;
    font-size: var(--text-base);
    margin-bottom: var(--space-md);
  }

  .checks {
    display: flex;
    gap: var(--space-lg);
    margin-bottom: var(--space-md);
  }

  .check-item {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    font-size: var(--text-sm);
    color: var(--danger);
    font-weight: 500;
  }

  .check-item.pass {
    color: var(--success);
  }

  .check-icon {
    font-weight: 700;
    font-size: var(--text-base);
  }

  .fix-error {
    padding: var(--space-sm) var(--space-md);
    margin-bottom: var(--space-md);
    background: rgba(244, 67, 54, 0.08);
    border: 1px solid rgba(244, 67, 54, 0.3);
    border-radius: var(--radius-sm);
    color: var(--danger);
    font-size: var(--text-sm);
    white-space: pre-line;
  }

  .fix-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-lg);
    border: 1px solid rgba(255, 183, 77, 0.4);
    border-radius: var(--radius-md);
    background: rgba(255, 183, 77, 0.1);
    color: var(--warning);
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .fix-btn:hover:not(:disabled) {
    background: rgba(255, 183, 77, 0.18);
    border-color: var(--warning);
  }

  .fix-btn:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .fix-spinner {
    display: inline-block;
    width: 12px;
    height: 12px;
    border: 2px solid rgba(255, 183, 77, 0.3);
    border-top-color: var(--warning);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  .manual-instructions {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .manual-label {
    font-weight: 600;
    color: var(--text-secondary);
  }

  .manual-instructions code {
    font-family: var(--font-data);
    font-size: var(--text-xs);
    padding: var(--space-xs) var(--space-sm);
    background: var(--bg-elevated);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
  }
</style>
