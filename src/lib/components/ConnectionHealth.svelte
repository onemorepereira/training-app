<script lang="ts">
  import { reconnectingDevices } from '$lib/stores/devices';
  import { api, extractError } from '$lib/tauri';

  function deviceTypeLabel(type: string): string {
    switch (type) {
      case 'HeartRate': return 'HR strap';
      case 'Power': return 'Power meter';
      case 'CadenceSpeed': return 'Speed/cadence';
      case 'FitnessTrainer': return 'Trainer';
      default: return type;
    }
  }

  let reconnectError = $state('');

  async function manualReconnect(deviceId: string) {
    reconnectError = '';
    try {
      await api.connectDevice(deviceId);
    } catch (e) {
      reconnectError = extractError(e);
    }
  }

  let entries = $derived(Object.values($reconnectingDevices));
</script>

{#if entries.length > 0}
  <div class="health-banners">
    {#each entries as device (device.device_id)}
      <div
        class="health-banner"
        class:reconnecting={device.status === 'reconnecting'}
        class:reconnected={device.status === 'reconnected'}
        class:disconnected={device.status === 'disconnected'}
      >
        <span class="health-dot"></span>
        <span class="health-text">
          {#if device.status === 'reconnecting'}
            {deviceTypeLabel(device.device_type)} reconnecting (attempt {device.attempt})...
          {:else if device.status === 'reconnected'}
            {deviceTypeLabel(device.device_type)} reconnected
          {:else}
            {deviceTypeLabel(device.device_type)} disconnected
          {/if}
        </span>
        {#if device.status === 'disconnected'}
          <button class="health-action" onclick={() => manualReconnect(device.device_id)}>
            Reconnect
          </button>
        {/if}
      </div>
    {/each}
    {#if reconnectError}
      <div class="health-error">{reconnectError}</div>
    {/if}
  </div>
{/if}

<style>
  .health-banners {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }

  .health-banner {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-md);
    font-size: var(--text-sm);
    font-weight: 600;
    animation: slide-up 200ms ease;
  }

  .health-banner.reconnecting {
    background: rgba(255, 183, 77, 0.08);
    border: 1px solid rgba(255, 183, 77, 0.3);
    color: var(--warning);
  }

  .health-banner.reconnected {
    background: rgba(76, 175, 80, 0.08);
    border: 1px solid rgba(76, 175, 80, 0.3);
    color: var(--success);
    animation: slide-up 200ms ease, fade-out 1s ease 2s forwards;
  }

  .health-banner.disconnected {
    background: rgba(244, 67, 54, 0.08);
    border: 1px solid rgba(244, 67, 54, 0.3);
    color: var(--danger);
  }

  .health-dot {
    width: 8px;
    height: 8px;
    border-radius: var(--radius-full);
    flex-shrink: 0;
  }

  .reconnecting .health-dot {
    background: var(--warning);
    animation: pulse-dot 1.5s ease-in-out infinite;
  }

  .reconnected .health-dot {
    background: var(--success);
    box-shadow: 0 0 6px var(--success-glow);
  }

  .disconnected .health-dot {
    background: var(--danger);
  }

  .health-text {
    flex: 1;
  }

  .health-action {
    padding: 2px var(--space-sm);
    border: 1px solid currentColor;
    border-radius: var(--radius-sm);
    background: transparent;
    color: inherit;
    font-size: var(--text-xs);
    font-weight: 700;
    cursor: pointer;
    transition: all var(--transition-fast);
    flex-shrink: 0;
  }

  .health-action:hover {
    background: rgba(244, 67, 54, 0.15);
  }

  .health-error {
    font-size: var(--text-xs);
    color: var(--danger);
    padding: 0 var(--space-md);
  }

  @keyframes pulse-dot {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.3; }
  }

  @keyframes fade-out {
    from { opacity: 1; }
    to { opacity: 0; }
  }
</style>
