<script lang="ts">
  import { reconnectingDevices, reconnectDevice } from '$lib/stores/devices';
  import { extractError, isConnectCancelled } from '$lib/tauri';
  import Notice from '$lib/ui/Notice.svelte';

  const RANK = { disconnected: 0, reconnecting: 1, reconnected: 2 } as const;

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
      await reconnectDevice(deviceId);
    } catch (e) {
      // A connect cancelled by the user's own disconnect is not an error.
      if (!isConnectCancelled(e)) reconnectError = extractError(e);
    }
  }

  // One notice: the most urgent device, with a count of the others.
  let entries = $derived(Object.values($reconnectingDevices).sort((a, b) => RANK[a.status] - RANK[b.status]));
  let top = $derived(entries[0]);
  let others = $derived(entries.length - 1);
</script>

{#if top}
  <Notice
    tone={reconnectError || top.status === 'disconnected' ? 'error' : top.status === 'reconnected' ? 'success' : 'warning'}
    title={top.status === 'reconnecting'
      ? `${deviceTypeLabel(top.device_type)} reconnecting (attempt ${top.attempt})…`
      : top.status === 'reconnected'
        ? `${deviceTypeLabel(top.device_type)} reconnected`
        : `${deviceTypeLabel(top.device_type)} disconnected`}
  >
    {#if reconnectError}
      Reconnect failed: {reconnectError}
    {:else if top.status === 'disconnected'}
      Readings from it have stopped. Your ride keeps recording.
    {/if}
    {#if others > 0}
      <span>{others} more device{others === 1 ? '' : 's'} affected.</span>
    {/if}
    {#snippet actions()}
      {#if top.status === 'disconnected'}
        <button type="button" class="btn btn-secondary" onclick={() => manualReconnect(top.device_id)}>Reconnect</button>
      {/if}
    {/snippet}
  </Notice>
{/if}
