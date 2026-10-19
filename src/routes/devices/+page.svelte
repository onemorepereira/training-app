<script lang="ts">
  import type { DeviceInfo, DeviceDetails, SensorReading } from '$lib/tauri';
  import { api, extractError, isConnectCancelled, type PrimaryDevices } from '$lib/tauri';
  import { deviceAction, isPrimaryDevice } from '$lib/utils/deviceActions';
  import SetupBanner from '$lib/components/SetupBanner.svelte';
  import { listen } from '@tauri-apps/api/event';
  import { onMount, onDestroy } from 'svelte';
  import { createDisposeGuard } from '$lib/utils/disposeGuard';
  import {
    connectedDevices,
    scanDevices as storeScan,
    connectDevice as storeConnect,
    disconnectDevice as storeDisconnect,
    unlinkDevices as storeUnlink,
  } from '$lib/stores/devices';
  import { get } from 'svelte/store';
  import { unitSystem, formatSpeed } from '$lib/stores/units';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Notice from '$lib/ui/Notice.svelte';
  import StateView from '$lib/ui/StateView.svelte';

  let scanning = $state(false);
  let error = $state('');
  let sensorPreview = $state<Record<string, string>>({});
  let primaryDevices = $state<PrimaryDevices>({});
  let pedalLabels = $state<Record<string, string>>({});
  const guard = createDisposeGuard();

  let detailModal = $state<DeviceDetails | null>(null);
  let detailOpen = $state(false);
  let detailLoading = $state('');
  let connectingIds = $state<Set<string>>(new Set());
  let disconnectingIds = $state<Set<string>>(new Set());

  /** First load: 'loading' until the known-devices list arrives, 'error' if it can't be read. */
  let loadState = $state<'loading' | 'ready' | 'error'>('loading');
  let loadError = $state('');

  async function load() {
    loadError = '';
    try {
      connectedDevices.set(await api.getKnownDevices());
      loadState = 'ready';
    } catch (e) {
      loadError = extractError(e);
      loadState = 'error';
      return;
    }
    try {
      primaryDevices = await api.getPrimaryDevices();
    } catch { /* no primaries yet */ }
  }

  onMount(async () => {
    await load();

    if (guard.disposed) return;
    const unlisten = await listen<SensorReading>('sensor_reading', (event) => {
      const r = event.payload;
      if (r.Power) {
        sensorPreview['Power'] = `${r.Power.watts} W`;
        if (r.Power.device_id) {
          sensorPreview[`Power:${r.Power.device_id}`] = `${r.Power.watts} W`;
          if (r.Power.pedal_balance != null) {
            const bal = r.Power.pedal_balance;
            pedalLabels[r.Power.device_id] = bal > 80 ? 'Right pedal' : 'Combined (L+R)';
            pedalLabels = { ...pedalLabels };
          }
        }
      }
      if (r.HeartRate) sensorPreview['HeartRate'] = `${r.HeartRate.bpm} bpm`;
      if (r.Cadence) sensorPreview['Cadence'] = `${Math.round(r.Cadence.rpm)} rpm`;
      if (r.Speed) {
        const units = get(unitSystem);
        sensorPreview['Speed'] = `${formatSpeed(r.Speed.kmh, units)} ${units === 'imperial' ? 'mph' : 'km/h'}`;
      }
      sensorPreview = { ...sensorPreview };
    });
    // Runs immediately if the page was left while listen() was pending.
    guard.add(unlisten);
  });

  onDestroy(() => guard.dispose());

  async function scan() {
    scanning = true;
    error = '';
    try {
      await storeScan();
      // A scan that works means the device list is readable again: reload it fully (incl. primaries).
      if (loadState === 'error') await load();
    } catch (e) {
      error = extractError(e);
    } finally {
      scanning = false;
    }
  }

  function actionFor(device: DeviceInfo) {
    return deviceAction(device.status, {
      connecting: connectingIds.has(device.id),
      disconnecting: disconnectingIds.has(device.id),
      inRange: device.in_range,
    });
  }

  async function toggleConnection(device: DeviceInfo) {
    error = '';
    const action = actionFor(device);
    if (action.disabled) return;
    const isConnect = action.kind === 'connect';
    if (isConnect) {
      connectingIds = new Set([...connectingIds, device.id]);
    } else {
      // disconnect, or stop: cancels an in-flight connect / reconnect loop
      disconnectingIds = new Set([...disconnectingIds, device.id]);
    }
    try {
      if (isConnect) {
        await storeConnect(device.id);
      } else {
        await storeDisconnect(device.id);
      }
    } catch (e) {
      // A connect cancelled by the user's own Stop/Cancel is not an error.
      if (!isConnectCancelled(e)) error = extractError(e);
    } finally {
      const next = new Set(isConnect ? connectingIds : disconnectingIds);
      next.delete(device.id);
      if (isConnect) {
        connectingIds = next;
      } else {
        disconnectingIds = next;
      }
    }
  }

  function isPrimary(device: DeviceInfo): boolean {
    return isPrimaryDevice(device, primaryDevices);
  }

  async function setPrimary(device: DeviceInfo) {
    try {
      await api.setPrimaryDevice(device.device_type, device.id);
      // Re-read: the backend may map one device type to several metric keys.
      primaryDevices = await api.getPrimaryDevices();
    } catch (e) {
      error = extractError(e);
    }
  }

  async function unlinkDevice(device: DeviceInfo) {
    error = '';
    try {
      await storeUnlink(device.id, device.device_group);
    } catch (e) {
      error = extractError(e);
    }
  }

  function deviceTypeLabel(type: string): string {
    switch (type) {
      case 'HeartRate': return 'Heart Rate';
      case 'Power': return 'Power Meter';
      case 'CadenceSpeed': return 'Speed/Cadence';
      case 'FitnessTrainer': return 'Smart Trainer';
      default: return type;
    }
  }

  function rssiLabel(rssi: number): string {
    if (rssi > -60) return 'Strong';
    if (rssi > -70) return 'Good';
    if (rssi > -80) return 'Fair';
    return 'Weak';
  }

  async function showDetails(device: DeviceInfo) {
    if (device.status !== 'Connected') return;
    detailLoading = device.id;
    error = '';
    try {
      detailModal = await api.getDeviceDetails(device.id);
      detailOpen = true;
    } catch (e) {
      error = extractError(e);
    } finally {
      detailLoading = '';
    }
  }

  function closeModal() {
    detailOpen = false;
    detailModal = null;
  }

  const transportName = (d: DeviceInfo) => (d.transport === 'Ble' ? 'BLE' : 'ANT+');

  /** Button label for one transport of a (possibly linked) device. */
  function actionLabel(td: DeviceInfo, all: DeviceInfo[]): string {
    const action = actionFor(td);
    if (all.length < 2) return action.label;
    const otherConnected = all.some((d) => d.id !== td.id && d.status === 'Connected');
    if (action.kind === 'connect' && otherConnected) return `Also connect ${transportName(td)}`;
    return `${action.label} ${transportName(td)}`;
  }

  function actionClass(td: DeviceInfo, all: DeviceInfo[]): string {
    const action = actionFor(td);
    if (action.kind === 'disconnect') return 'btn-secondary btn-danger-text';
    if (action.kind === 'stop') return 'btn-secondary';
    return all.some((d) => d.status === 'Connected') ? 'btn-secondary' : 'btn-primary';
  }

  function statusOf(all: DeviceInfo[]): { label: string; tone: 'on' | 'busy' | 'off' } {
    if (all.some((d) => d.status === 'Connected')) return { label: 'Connected', tone: 'on' };
    if (all.some((d) => d.status === 'Reconnecting')) return { label: 'Searching…', tone: 'busy' };
    if (all.some((d) => d.status === 'Connecting' || connectingIds.has(d.id))) return { label: 'Connecting…', tone: 'busy' };
    if (all.every((d) => d.in_range === false)) return { label: 'Not in range', tone: 'off' };
    return { label: 'Available', tone: 'off' };
  }

  function shortUuid(uuid: string): string {
    // If it's a standard Bluetooth UUID, show just the 4-char short form
    if (uuid.endsWith('-0000-1000-8000-00805f9b34fb')) {
      return '0x' + uuid.substring(4, 8).toUpperCase();
    }
    return uuid;
  }

  interface DisplayDevice {
    primary: DeviceInfo;
    linked: DeviceInfo[];
  }

  const typeOrder = ['FitnessTrainer', 'Power', 'HeartRate', 'CadenceSpeed'];

  let groupedDevices = $derived.by(() => {
    // First, merge devices by device_group
    const groupMap = new Map<string, DeviceInfo[]>();
    const ungrouped: DeviceInfo[] = [];
    for (const d of $connectedDevices) {
      if (d.device_group) {
        const list = groupMap.get(d.device_group) ?? [];
        list.push(d);
        groupMap.set(d.device_group, list);
      } else {
        ungrouped.push(d);
      }
    }

    // Build display devices: grouped ones become a single entry
    const displayDevices: DisplayDevice[] = [];
    const seen = new Set<string>();
    for (const [, members] of groupMap) {
      // Primary is the BLE device (or connected one, or first)
      const sorted = [...members].sort((a, b) => {
        if (a.status === 'Connected' && b.status !== 'Connected') return -1;
        if (b.status === 'Connected' && a.status !== 'Connected') return 1;
        if (a.transport === 'Ble' && b.transport !== 'Ble') return -1;
        if (b.transport === 'Ble' && a.transport !== 'Ble') return 1;
        return 0;
      });
      displayDevices.push({ primary: sorted[0], linked: sorted.slice(1) });
      for (const m of members) seen.add(m.id);
    }
    for (const d of ungrouped) {
      if (!seen.has(d.id)) {
        displayDevices.push({ primary: d, linked: [] });
      }
    }

    // Group by device type
    const groups: Array<{ type: string; devices: DisplayDevice[] }> = [];
    const byType = new Map<string, DisplayDevice[]>();
    for (const dd of displayDevices) {
      const list = byType.get(dd.primary.device_type) ?? [];
      list.push(dd);
      byType.set(dd.primary.device_type, list);
    }
    for (const type of typeOrder) {
      const list = byType.get(type);
      if (list) groups.push({ type, devices: list });
      byType.delete(type);
    }
    for (const [type, list] of byType) {
      groups.push({ type, devices: list });
    }
    return groups;
  });
</script>

<div class="page-wide" data-page="devices">
  <header class="page-header">
    <h1 class="page-title">Devices</h1>
    <button type="button" class="btn btn-primary" onclick={scan} disabled={scanning || loadState === 'loading'}>
      {#if scanning}
        <span class="spinner" aria-hidden="true"></span>
        Scanning…
      {:else}
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <circle cx="11" cy="11" r="8"/>
          <line x1="21" y1="21" x2="16.65" y2="16.65"/>
        </svg>
        Scan for devices
      {/if}
    </button>
  </header>

  <SetupBanner />

  {#if error}
    <div class="notice-slot">
      <Notice tone="error" ondismiss={() => (error = '')}>{error}</Notice>
    </div>
  {/if}

  {#if loadState === 'loading'}
    <StateView kind="loading" title="Loading your devices…" />
  {:else if loadState === 'error'}
    <StateView kind="error" title="Couldn't load your devices" message="The app couldn't read its saved device list. Retry, and restart the app if it keeps happening." details={loadError} action={{ label: 'Retry', onclick: load }} />
  {:else if $connectedDevices.length === 0}
    {#if scanning}
      <StateView kind="loading" title="Looking for devices…" message="Make sure your sensors are awake: spin the cranks or put on the strap." />
    {:else}
      <StateView
        kind="empty"
        title="No devices yet"
        message="Turn on your sensors and trainer, then scan for Bluetooth and ANT+ devices nearby."
        action={{ label: 'Scan for devices', onclick: scan }}
      />
    {/if}
  {:else}
    {#each groupedDevices as group (group.type)}
      <section class="device-group" aria-labelledby="group-{group.type}">
        <h2 class="group-header" id="group-{group.type}">
          <span class="group-icon" aria-hidden="true">
            {#if group.type === 'HeartRate'}
              <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20.84 4.61a5.5 5.5 0 00-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 00-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 000-7.78z"/></svg>
            {:else if group.type === 'Power'}
              <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg>
            {:else if group.type === 'CadenceSpeed'}
              <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0114.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0020.49 15"/></svg>
            {:else if group.type === 'FitnessTrainer'}
              <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="5" cy="18" r="3"/><circle cx="19" cy="18" r="3"/><path d="M12 2l-3.5 7H15l-2 6"/><line x1="8" y1="18" x2="16" y2="18"/></svg>
            {:else}
              <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
            {/if}
          </span>
          {deviceTypeLabel(group.type)}
          <span class="group-count">{group.devices.length}</span>
        </h2>

        <ul class="device-list">
          {#each group.devices as dd (dd.primary.id)}
            {@const device = dd.primary}
            {@const allTransports = [device, ...dd.linked]}
            {@const st = statusOf(allTransports)}
            {@const preview = device.status === 'Connected' ? (sensorPreview[`${device.device_type}:${device.id}`] ?? sensorPreview[device.device_type]) : undefined}
            <li class="device-row" class:connected={st.tone === 'on'}>
              <div class="device-main">
                <div class="device-title">
                  <span class="device-name">{device.name ?? 'Unknown device'}</span>
                  {#each allTransports as td (td.id)}
                    <span class="badge transport" class:transport-on={td.status === 'Connected'} title={td.status === 'Connected' ? `Connected over ${td.transport === 'Ble' ? 'Bluetooth' : 'ANT+'}` : undefined}>
                      {#if td.transport === 'AntPlus'}
                        <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true"><line x1="13" y1="13" x2="7.5" y2="5" stroke="currentColor" stroke-width="2"/><line x1="13" y1="13" x2="4.5" y2="18.5" stroke="currentColor" stroke-width="2"/><circle cx="13" cy="13" r="5" fill="currentColor"/><circle cx="7.5" cy="5" r="2.5" fill="currentColor"/><circle cx="4.5" cy="18.5" r="1.8" fill="currentColor"/></svg>
                        ANT+
                      {:else}
                        <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true"><path d="M7 7l10 10-5 5V2l5 5L7 17" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
                        BLE
                      {/if}
                    </span>
                  {/each}
                  {#if device.status === 'Connected' && isPrimary(device)}
                    <span class="badge primary">Primary</span>
                  {/if}
                </div>

                <div class="device-meta">
                  <span class="meta status-{st.tone}"><span class="dot" aria-hidden="true"></span>{st.label}</span>
                  {#if device.battery_level != null}
                    <span class="meta" title="Battery">
                      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="7" width="17" height="10" rx="2"/><line x1="22" y1="11" x2="22" y2="13"/><rect x="4" y="9" width={Math.max(1, Math.round(13 * device.battery_level / 100))} height="6" fill="currentColor" stroke="none"/></svg>
                      <span class="sr-only">Battery</span>{device.battery_level}%
                    </span>
                  {/if}
                  {#if device.rssi != null}
                    <span class="meta" title="{device.rssi} dBm">
                      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><line x1="4" y1="20" x2="4" y2="16"/><line x1="10" y1="20" x2="10" y2="12"/><line x1="16" y1="20" x2="16" y2="8" opacity={device.rssi > -80 ? 1 : 0.3}/><line x1="22" y1="20" x2="22" y2="4" opacity={device.rssi > -60 ? 1 : 0.3}/></svg>
                      <span class="sr-only">Signal</span>{rssiLabel(device.rssi)} signal
                    </span>
                  {/if}
                  {#if dd.linked.length > 0}
                    <span class="meta">Linked BLE + ANT+</span>
                  {/if}
                  {#if pedalLabels[device.id]}
                    <span class="meta">{pedalLabels[device.id]}</span>
                  {/if}
                  {#if preview}
                    <span class="meta live data" title="Live reading">{preview}</span>
                  {/if}
                </div>
              </div>

              <div class="device-actions">
                {#each allTransports as td (td.id)}
                  {#if td.status === 'Connected'}
                    <button type="button" class="btn btn-ghost" onclick={() => showDetails(td)} disabled={detailLoading === td.id}>
                      {#if detailLoading === td.id}<span class="spinner" aria-hidden="true"></span>{/if}
                      Details{#if dd.linked.length > 0}{' '}({transportName(td)}){/if}
                    </button>
                    {#if !isPrimary(td)}
                      <button type="button" class="btn btn-ghost" onclick={() => setPrimary(td)}>Set as primary</button>
                    {/if}
                  {/if}
                {/each}
                {#if dd.linked.length > 0}
                  <button type="button" class="btn btn-ghost" onclick={() => unlinkDevice(device)}>Unlink</button>
                {/if}
                {#each allTransports as td (td.id)}
                  {@const action = actionFor(td)}
                  <button
                    type="button"
                    class="btn {actionClass(td, allTransports)}"
                    disabled={action.disabled}
                    title={action.disabled && action.kind === 'connect' ? 'Not in range: wake the sensor and scan again' : undefined}
                    onclick={() => toggleConnection(td)}
                  >
                    {#if action.busy}<span class="spinner" aria-hidden="true"></span>{/if}
                    {actionLabel(td, allTransports)}
                  </button>
                {/each}
              </div>
            </li>
          {/each}
        </ul>
      </section>
    {/each}
  {/if}
</div>

{#if detailModal}
  <Dialog bind:open={detailOpen} title={detailModal.name ?? 'Unknown device'} description={deviceTypeLabel(detailModal.device_type)} onclose={closeModal} width="lg">
    <section class="detail-section">
      <h3>Connection</h3>
      <dl class="detail-grid">
        <dt>ID</dt><dd class="data">{detailModal.id}</dd>
        <dt>Transport</dt><dd>{detailModal.transport === 'Ble' ? 'Bluetooth LE' : 'ANT+'}</dd>
        {#if detailModal.rssi != null}<dt>Signal</dt><dd class="data">{detailModal.rssi} dBm ({rssiLabel(detailModal.rssi)})</dd>{/if}
        {#if detailModal.battery_level != null}<dt>Battery</dt><dd class="data">{detailModal.battery_level}%</dd>{/if}
      </dl>
    </section>

    {#if detailModal.manufacturer || detailModal.model_number || detailModal.serial_number || detailModal.firmware_revision || detailModal.hardware_revision || detailModal.software_revision}
      <section class="detail-section">
        <h3>Device information</h3>
        <dl class="detail-grid">
          {#if detailModal.manufacturer}<dt>Manufacturer</dt><dd>{detailModal.manufacturer}</dd>{/if}
          {#if detailModal.model_number}<dt>Model</dt><dd>{detailModal.model_number}</dd>{/if}
          {#if detailModal.serial_number}<dt>Serial</dt><dd class="data">{detailModal.serial_number}</dd>{/if}
          {#if detailModal.firmware_revision}<dt>Firmware</dt><dd class="data">{detailModal.firmware_revision}</dd>{/if}
          {#if detailModal.hardware_revision}<dt>Hardware</dt><dd class="data">{detailModal.hardware_revision}</dd>{/if}
          {#if detailModal.software_revision}<dt>Software</dt><dd class="data">{detailModal.software_revision}</dd>{/if}
        </dl>
      </section>
    {/if}

    {#if detailModal.services.length > 0}
      <section class="detail-section">
        <h3>GATT services</h3>
        <ul class="services">
          {#each detailModal.services as service (service.uuid)}
            <li class="service">
              <div class="service-head">
                <span class="service-name">{service.name ?? 'Unknown service'}</span>
                <span class="uuid data">{shortUuid(service.uuid)}</span>
              </div>
              {#if service.characteristics.length > 0}
                <ul class="chars">
                  {#each service.characteristics as char (char.uuid)}
                    <li class="char">
                      <span class="char-name">{char.name ?? shortUuid(char.uuid)}</span>
                      <span class="uuid data">{shortUuid(char.uuid)}</span>
                      <span class="props">
                        {#each char.properties as prop}<span class="prop">{prop}</span>{/each}
                      </span>
                    </li>
                  {/each}
                </ul>
              {/if}
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    {#snippet footer()}
      <button type="button" class="btn btn-secondary" onclick={() => (detailOpen = false)}>Close</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .notice-slot {
    margin-bottom: var(--space-lg);
  }

  .spinner {
    width: 14px;
    height: 14px;
    border: 2px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  .device-group {
    margin-bottom: var(--space-xl);
  }

  .group-header {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    margin: 0 0 var(--space-sm);
    padding-bottom: var(--space-sm);
    border-bottom: 1px solid var(--border-subtle);
    font-size: var(--text-sm);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-secondary);
  }

  .group-icon {
    display: flex;
    color: var(--text-muted);
  }

  .group-count {
    min-width: 22px;
    padding: 0 6px;
    border-radius: var(--radius-full);
    background: var(--bg-elevated);
    color: var(--text-secondary);
    font-size: var(--text-xs);
    text-align: center;
    letter-spacing: 0;
  }

  .device-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }

  .device-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-md) var(--space-xl);
    padding: var(--space-md) var(--space-lg);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-lg);
    background: var(--bg-surface);
  }

  .device-row.connected {
    border-color: rgba(76, 175, 80, 0.3);
  }

  .device-main {
    flex: 1 1 320px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .device-title {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-sm);
  }

  .device-name {
    font-size: var(--text-base);
    font-weight: 650;
    color: var(--text-primary);
    overflow-wrap: anywhere;
  }

  .badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 8px;
    border-radius: var(--radius-full);
    border: 1px solid var(--border-default);
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-secondary);
    white-space: nowrap;
  }

  .badge.transport-on {
    border-color: rgba(100, 181, 246, 0.45);
    color: var(--info);
  }

  .badge.primary {
    border-color: rgba(255, 183, 77, 0.45);
    color: var(--warning);
  }

  .device-meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px var(--space-lg);
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }

  .meta {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
  }

  .meta svg {
    color: var(--text-muted);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-faint);
  }

  .status-on .dot {
    background: var(--success);
  }

  .status-on {
    color: var(--text-primary);
  }

  .status-busy .dot {
    background: var(--warning);
    animation: pulse-glow 1.2s ease infinite;
  }

  .live {
    color: var(--text-primary);
    font-weight: 600;
  }

  .device-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-xs);
    margin-left: auto;
  }

  /* ── Details dialog ── */
  .detail-section + .detail-section {
    margin-top: var(--space-xl);
  }

  .detail-section h3 {
    margin: 0 0 var(--space-sm);
    font-size: var(--text-sm);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-secondary);
  }

  .detail-grid {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 6px var(--space-xl);
    margin: 0;
    font-size: var(--text-sm);
  }

  .detail-grid dt {
    color: var(--text-muted);
  }

  .detail-grid dd {
    margin: 0;
    color: var(--text-primary);
    overflow-wrap: anywhere;
  }

  .services,
  .chars {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .services {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }

  .service {
    padding: var(--space-sm) var(--space-md);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
  }

  .service-head,
  .char {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 4px var(--space-md);
  }

  .service-name {
    font-weight: 600;
    font-size: var(--text-sm);
  }

  .chars {
    margin-top: 6px;
    padding-left: var(--space-md);
    border-left: 2px solid var(--border-subtle);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .char-name {
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }

  .uuid {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .props {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-left: auto;
  }

  .prop {
    padding: 0 6px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border-default);
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }
</style>
