<script lang="ts">
  import type { DeviceInfo, DeviceDetails, SensorReading } from '$lib/tauri';
  import { api, extractError } from '$lib/tauri';
  import SetupBanner from '$lib/components/SetupBanner.svelte';
  import { listen } from '@tauri-apps/api/event';
  import { onMount, onDestroy } from 'svelte';
  import { connectedDevices } from '$lib/stores/devices';
  import { get } from 'svelte/store';
  import { unitSystem, formatSpeed } from '$lib/stores/units';

  let devices = $state<DeviceInfo[]>([]);
  let scanning = $state(false);
  let error = $state('');
  let sensorPreview = $state<Record<string, string>>({});
  let primaryDevices = $state<Record<string, string>>({});
  let pedalLabels = $state<Record<string, string>>({});
  let unlisten: (() => void) | null = null;
  let unsubDevices: (() => void) | null = null;

  let detailModal = $state<DeviceDetails | null>(null);
  let detailLoading = $state('');
  let connectingIds = $state<Set<string>>(new Set());
  let disconnectingIds = $state<Set<string>>(new Set());

  function syncStore() {
    connectedDevices.set(devices);
  }

  onMount(async () => {
    try {
      devices = await api.getKnownDevices();
      syncStore();
    } catch { /* no known devices yet */ }
    try {
      primaryDevices = await api.getPrimaryDevices();
    } catch { /* no primaries yet */ }

    // Subscribe to global store updates (disconnect/reconnect events handled in layout)
    unsubDevices = connectedDevices.subscribe((storeDevices) => {
      // Merge store status changes into local devices array
      for (const sd of storeDevices) {
        const idx = devices.findIndex((d) => d.id === sd.id);
        if (idx >= 0 && devices[idx].status !== sd.status) {
          devices[idx] = { ...devices[idx], status: sd.status };
          devices = [...devices];
        }
      }
    });

    unlisten = await listen<SensorReading>('sensor_reading', (event) => {
      const r = event.payload;
      if (r.Power) {
        sensorPreview['Power'] = `${r.Power.watts}W`;
        if (r.Power.device_id) {
          sensorPreview[`Power:${r.Power.device_id}`] = `${r.Power.watts}W`;
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
  });

  onDestroy(() => { unlisten?.(); unsubDevices?.(); });

  async function scan() {
    scanning = true;
    error = '';
    try {
      devices = await api.scanDevices();
      syncStore();
    } catch (e) {
      error = extractError(e);
    } finally {
      scanning = false;
    }
  }

  async function toggleConnection(device: DeviceInfo) {
    error = '';
    const isConnect = device.status !== 'Connected';
    if (isConnect) {
      connectingIds = new Set([...connectingIds, device.id]);
    } else {
      disconnectingIds = new Set([...disconnectingIds, device.id]);
    }
    try {
      if (!isConnect) {
        await api.disconnectDevice(device.id);
        device.status = 'Disconnected';
      } else {
        const updated = await api.connectDevice(device.id);
        const idx = devices.findIndex((d) => d.id === device.id);
        if (idx >= 0) devices[idx] = updated;
      }
      devices = [...devices];
      syncStore();
    } catch (e) {
      error = extractError(e);
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
    return primaryDevices[device.device_type] === device.id;
  }

  async function setPrimary(device: DeviceInfo) {
    try {
      await api.setPrimaryDevice(device.device_type, device.id);
      primaryDevices = { ...primaryDevices, [device.device_type]: device.id };
    } catch (e) {
      error = extractError(e);
    }
  }

  async function unlinkDevice(device: DeviceInfo) {
    error = '';
    try {
      await api.unlinkDevices(device.id);
      // Clear device_group locally for immediate UI update
      for (const d of devices) {
        if (d.device_group === device.device_group) {
          d.device_group = null;
        }
      }
      devices = [...devices];
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
    } catch (e) {
      error = extractError(e);
    } finally {
      detailLoading = '';
    }
  }

  function closeModal() {
    detailModal = null;
  }

  function handleModalKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') closeModal();
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
    for (const d of devices) {
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

<div class="page">
  <div class="page-header">
    <h1>Devices</h1>
    <button class="scan-btn" onclick={scan} disabled={scanning}>
      {#if scanning}
        <span class="scan-spinner"></span>
        Scanning...
      {:else}
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="11" cy="11" r="8"/>
          <line x1="21" y1="21" x2="16.65" y2="16.65"/>
        </svg>
        Scan
      {/if}
    </button>
  </div>

  <SetupBanner />

  {#if error}
    <div class="error">{error}</div>
  {/if}

  {#if devices.length === 0 && !scanning}
    <div class="empty-state">
      <div class="empty-icon">
        <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M12 20v-6"/>
          <path d="M12 10c-3.3 0-6-2.7-6-6"/>
          <path d="M12 10c3.3 0 6-2.7 6-6"/>
          <path d="M12 10c-1.7 0-3-1.3-3-3"/>
          <path d="M12 10c1.7 0 3-1.3 3-3"/>
        </svg>
      </div>
      <p class="empty-text">No devices found</p>
      <p class="empty-hint">Click Scan to search for BLE and ANT+ devices</p>
    </div>
  {/if}

  {#each groupedDevices as group}
    <div class="device-group">
      <div class="group-header">
        <span class="group-icon">
          {#if group.type === 'HeartRate'}
            <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M20.84 4.61a5.5 5.5 0 00-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 00-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 000-7.78z"/>
            </svg>
          {:else if group.type === 'Power'}
            <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/>
            </svg>
          {:else if group.type === 'CadenceSpeed'}
            <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="23 4 23 10 17 10"/>
              <polyline points="1 20 1 14 7 14"/>
              <path d="M3.51 9a9 9 0 0114.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0020.49 15"/>
            </svg>
          {:else if group.type === 'FitnessTrainer'}
            <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="5" cy="18" r="3"/>
              <circle cx="19" cy="18" r="3"/>
              <path d="M12 2l-3.5 7H15l-2 6"/>
              <line x1="8" y1="18" x2="16" y2="18"/>
            </svg>
          {:else}
            <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="10"/>
              <line x1="12" y1="8" x2="12" y2="12"/>
              <line x1="12" y1="16" x2="12.01" y2="16"/>
            </svg>
          {/if}
        </span>
        <span class="group-label">{deviceTypeLabel(group.type)}</span>
        <span class="group-count">{group.devices.length}</span>
      </div>
      <div class="device-list">
        {#each group.devices as dd}
          {@const device = dd.primary}
          {@const allTransports = [device, ...dd.linked]}
          <div class="device-card" class:connected={allTransports.some(d => d.status === 'Connected')}>
            <div class="device-info">
              <div class="device-header">
                <span class="device-name">{device.name ?? 'Unknown Device'}</span>
                {#if pedalLabels[device.id]}
                  <span class="pedal-label">{pedalLabels[device.id]}</span>
                {/if}
                {#each allTransports as td}
                  {#if td.transport === 'AntPlus'}
                    <span class="transport-badge ant" class:transport-connected={td.status === 'Connected'}>
                      <svg viewBox="0 0 24 24" width="16" height="16" aria-label="ANT+">
                        <line x1="13" y1="13" x2="7.5" y2="5" stroke="currentColor" stroke-width="2"/>
                        <line x1="13" y1="13" x2="4.5" y2="18.5" stroke="currentColor" stroke-width="2"/>
                        <circle cx="13" cy="13" r="5" fill="currentColor"/>
                        <circle cx="7.5" cy="5" r="2.5" fill="currentColor"/>
                        <circle cx="4.5" cy="18.5" r="1.8" fill="currentColor"/>
                        <line x1="19" y1="2" x2="19" y2="7" stroke="currentColor" stroke-width="1.8"/>
                        <line x1="16.5" y1="4.5" x2="21.5" y2="4.5" stroke="currentColor" stroke-width="1.8"/>
                      </svg>
                    </span>
                  {:else}
                    <span class="transport-badge ble" class:transport-connected={td.status === 'Connected'}>
                      <svg viewBox="0 0 24 24" width="16" height="16" aria-label="Bluetooth">
                        <circle cx="12" cy="12" r="12" fill="#4285F4"/>
                        <path d="M12.5 5L16 8.5l-3 3 3 3L12.5 18v-4.5L9.5 16.5l-1-1 3.5-3.5-3.5-3.5 1-1 3 3V5z" fill="white" stroke="white" stroke-width="0.6" stroke-linejoin="round"/>
                      </svg>
                    </span>
                  {/if}
                {/each}
                {#if allTransports.some(d => d.status === 'Connected')}
                  <span class="connected-dot"></span>
                {/if}
                {#if device.status === 'Connected' && isPrimary(device)}
                  <span class="primary-badge">Primary</span>
                {/if}
              </div>
              <div class="device-meta">
                {#if device.battery_level != null}
                  <span class="meta-item battery">{device.battery_level}%</span>
                {/if}
                {#if device.rssi != null}
                  <span class="meta-item" title="{device.rssi} dBm">
                    {rssiLabel(device.rssi)}
                  </span>
                {/if}
                {#if dd.linked.length > 0}
                  <span class="meta-item linked-badge">Linked</span>
                {/if}
              </div>
            </div>
            {#if device.status === 'Connected' && (sensorPreview[`${device.device_type}:${device.id}`] || sensorPreview[device.device_type])}
              <div class="sensor-preview">{sensorPreview[`${device.device_type}:${device.id}`] ?? sensorPreview[device.device_type]}</div>
            {/if}
            <div class="device-actions">
              {#if dd.linked.length > 0}
                <button class="action-btn ghost" onclick={() => unlinkDevice(device)} aria-label="Unlink devices">
                  Unlink
                </button>
              {/if}
              {#each allTransports as td}
                {#if td.status === 'Connected'}
                  <button
                    class="action-btn ghost info-btn"
                    onclick={() => showDetails(td)}
                    disabled={detailLoading === td.id}
                    aria-label="Device details"
                  >
                    {#if detailLoading === td.id}
                      <span class="btn-spinner"></span>
                    {:else}
                      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <circle cx="12" cy="12" r="10"/>
                        <line x1="12" y1="16" x2="12" y2="12"/>
                        <line x1="12" y1="8" x2="12.01" y2="8"/>
                      </svg>
                    {/if}
                  </button>
                {/if}
                {#if td.status === 'Connected' && !isPrimary(td)}
                  <button class="action-btn ghost" onclick={() => setPrimary(td)}>
                    Set Primary
                  </button>
                {/if}
                <button
                  class="action-btn"
                  class:danger={td.status === 'Connected' && !disconnectingIds.has(td.id)}
                  disabled={connectingIds.has(td.id) || disconnectingIds.has(td.id)}
                  onclick={() => toggleConnection(td)}
                >
                  {#if connectingIds.has(td.id)}
                    <span class="btn-spinner"></span>
                    {td.transport === 'Ble' ? 'BLE' : 'ANT+'}…
                  {:else if disconnectingIds.has(td.id)}
                    <span class="btn-spinner"></span>
                    {td.transport === 'Ble' ? 'BLE' : 'ANT+'}…
                  {:else if dd.linked.length > 0}
                    {td.status === 'Connected' ? 'Disconnect' : 'Connect'} {td.transport === 'Ble' ? 'BLE' : 'ANT+'}
                  {:else}
                    {td.status === 'Connected' ? 'Disconnect' : 'Connect'}
                  {/if}
                </button>
              {/each}
            </div>
          </div>
        {/each}
      </div>
    </div>
  {/each}
</div>

{#if detailModal}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <!-- svelte-ignore a11y_interactive_supports_focus -->
  <div class="modal-backdrop" role="dialog" aria-modal="true" onclick={closeModal} onkeydown={handleModalKeydown}>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="modal" onclick={(e) => e.stopPropagation()} onkeydown={handleModalKeydown}>
      <div class="modal-header">
        <h2>{detailModal.name ?? 'Unknown Device'}</h2>
        <button class="modal-close" onclick={closeModal} aria-label="Close">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
          </svg>
        </button>
      </div>

      <div class="modal-body">
        <div class="detail-section">
          <h3>Connection</h3>
          <div class="detail-grid">
            <span class="detail-label">ID</span>
            <span class="detail-value mono">{detailModal.id}</span>
            <span class="detail-label">Transport</span>
            <span class="detail-value">{detailModal.transport === 'Ble' ? 'Bluetooth LE' : 'ANT+'}</span>
            <span class="detail-label">Type</span>
            <span class="detail-value">{deviceTypeLabel(detailModal.device_type)}</span>
            {#if detailModal.rssi != null}
              <span class="detail-label">RSSI</span>
              <span class="detail-value mono">{detailModal.rssi} dBm</span>
            {/if}
            {#if detailModal.battery_level != null}
              <span class="detail-label">Battery</span>
              <span class="detail-value">{detailModal.battery_level}%</span>
            {/if}
          </div>
        </div>

        {#if detailModal.manufacturer || detailModal.model_number || detailModal.serial_number || detailModal.firmware_revision || detailModal.hardware_revision || detailModal.software_revision}
          <div class="detail-section">
            <h3>Device Information</h3>
            <div class="detail-grid">
              {#if detailModal.manufacturer}
                <span class="detail-label">Manufacturer</span>
                <span class="detail-value">{detailModal.manufacturer}</span>
              {/if}
              {#if detailModal.model_number}
                <span class="detail-label">Model</span>
                <span class="detail-value">{detailModal.model_number}</span>
              {/if}
              {#if detailModal.serial_number}
                <span class="detail-label">Serial</span>
                <span class="detail-value mono">{detailModal.serial_number}</span>
              {/if}
              {#if detailModal.firmware_revision}
                <span class="detail-label">Firmware</span>
                <span class="detail-value mono">{detailModal.firmware_revision}</span>
              {/if}
              {#if detailModal.hardware_revision}
                <span class="detail-label">Hardware</span>
                <span class="detail-value mono">{detailModal.hardware_revision}</span>
              {/if}
              {#if detailModal.software_revision}
                <span class="detail-label">Software</span>
                <span class="detail-value mono">{detailModal.software_revision}</span>
              {/if}
            </div>
          </div>
        {/if}

        {#if detailModal.services.length > 0}
          <div class="detail-section">
            <h3>GATT Services</h3>
            <div class="services-list">
              {#each detailModal.services as service}
                <div class="service-item">
                  <div class="service-header">
                    <span class="service-name">{service.name ?? 'Unknown Service'}</span>
                    <span class="service-uuid">{shortUuid(service.uuid)}</span>
                  </div>
                  {#if service.characteristics.length > 0}
                    <div class="chars-list">
                      {#each service.characteristics as char}
                        <div class="char-item">
                          <span class="char-name">{char.name ?? shortUuid(char.uuid)}</span>
                          <span class="char-uuid">{shortUuid(char.uuid)}</span>
                          <div class="char-props">
                            {#each char.properties as prop}
                              <span class="char-prop">{prop}</span>
                            {/each}
                          </div>
                        </div>
                      {/each}
                    </div>
                  {/if}
                </div>
              {/each}
            </div>
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .page {
    max-width: 100%;
  }

  .page-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-xl);
  }

  h1 {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: 700;
  }

  .scan-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-lg);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-lg);
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-size: var(--text-base);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--transition-fast);
  }

  .scan-btn:hover {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .scan-btn:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .scan-spinner {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  .btn-spinner {
    display: inline-block;
    width: 12px;
    height: 12px;
    border: 2px solid var(--border-strong);
    border-top-color: currentColor;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    flex-shrink: 0;
  }

  .error {
    padding: var(--space-md);
    margin-bottom: var(--space-lg);
    background: rgba(244, 67, 54, 0.08);
    border: 1px solid rgba(244, 67, 54, 0.3);
    border-radius: var(--radius-md);
    color: var(--danger);
    font-size: var(--text-base);
    animation: slide-up 200ms ease;
  }

  .empty-state {
    text-align: center;
    padding: var(--space-2xl) var(--space-lg);
    color: var(--text-muted);
  }

  .empty-icon {
    margin-bottom: var(--space-md);
    opacity: 0.4;
  }

  .empty-text {
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--text-secondary);
    margin: 0 0 var(--space-xs);
  }

  .empty-hint {
    font-size: var(--text-sm);
    color: var(--text-muted);
    margin: 0;
  }

  .device-group {
    margin-bottom: var(--space-xl);
  }

  .group-header {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    margin-bottom: var(--space-md);
    padding-bottom: var(--space-sm);
    border-bottom: 1px solid var(--border-subtle);
  }

  .group-icon {
    display: flex;
    align-items: center;
    color: var(--text-secondary);
  }

  .group-label {
    font-size: var(--text-sm);
    font-weight: 700;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .group-count {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-muted);
    background: var(--bg-elevated);
    padding: 1px 6px;
    border-radius: var(--radius-full);
  }

  .device-list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(400px, 1fr));
    gap: var(--space-sm);
  }

  .device-card {
    display: flex;
    align-items: center;
    gap: var(--space-lg);
    padding: var(--space-md) var(--space-lg);
    background: var(--bg-surface);
    border-radius: var(--radius-md);
    border: 1px solid var(--border-subtle);
    transition: all var(--transition-fast);
  }

  .device-card:hover {
    border-color: var(--border-default);
    background: var(--bg-elevated);
  }

  .device-card.connected {
    border-color: rgba(76, 175, 80, 0.15);
  }

  .connected-dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--success);
    box-shadow: 0 0 4px var(--success-glow);
    flex-shrink: 0;
  }

  .device-info {
    flex: 1;
    min-width: 0;
  }

  .device-header {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    margin-bottom: 2px;
  }

  .device-name {
    font-weight: 600;
    font-size: var(--text-base);
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .transport-badge {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
    line-height: 0;
    opacity: 0.4;
  }

  .transport-badge.transport-connected {
    opacity: 1;
  }

  .linked-badge {
    color: var(--info);
    font-weight: 600;
  }

  .device-meta {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    font-size: var(--text-xs);
    color: var(--text-faint);
  }

  .device-meta:empty {
    display: none;
  }

  .meta-item.battery {
    color: var(--warning);
  }

  .sensor-preview {
    font-family: var(--font-data);
    font-size: var(--text-lg);
    font-weight: 700;
    color: var(--info);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .device-actions {
    display: flex;
    gap: var(--space-sm);
    align-items: center;
    flex-shrink: 0;
  }

  .action-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    padding: var(--space-sm) var(--space-md);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-size: var(--text-sm);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--transition-fast);
    white-space: nowrap;
  }

  .action-btn:hover:not(:disabled) {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .action-btn:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .action-btn.danger {
    border-color: rgba(244, 67, 54, 0.3);
    color: var(--danger);
  }

  .action-btn.danger:hover:not(:disabled) {
    background: rgba(244, 67, 54, 0.1);
    border-color: var(--danger);
  }

  .action-btn.ghost {
    background: transparent;
    border-color: var(--border-default);
    color: var(--text-muted);
    font-size: var(--text-xs);
    padding: 3px var(--space-sm);
  }

  .action-btn.ghost:hover {
    border-color: var(--warning);
    color: var(--warning);
    background: rgba(255, 183, 77, 0.08);
  }

  .primary-badge {
    font-size: var(--text-xs);
    font-weight: 700;
    color: var(--warning);
    padding: 1px 6px;
    border: 1px solid rgba(255, 183, 77, 0.3);
    border-radius: var(--radius-full);
    background: rgba(255, 183, 77, 0.08);
    flex-shrink: 0;
  }

  .pedal-label {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-muted);
    padding: 1px 6px;
    border: 1px solid var(--border-default);
    border-radius: var(--radius-full);
    background: var(--bg-elevated);
    flex-shrink: 0;
  }

  .info-btn {
    padding: 4px 6px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .info-btn:hover {
    border-color: var(--info) !important;
    color: var(--info) !important;
    background: rgba(100, 181, 246, 0.08) !important;
  }

  /* Modal */
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    backdrop-filter: blur(4px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    padding: var(--space-lg);
    animation: fade-in 150ms ease;
  }

  .modal {
    background: var(--bg-surface);
    border: 1px solid var(--border-default);
    border-radius: var(--radius-lg);
    width: 100%;
    max-width: 560px;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
    animation: slide-up 200ms ease;
  }

  .modal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-lg) var(--space-xl);
    border-bottom: 1px solid var(--border-subtle);
    flex-shrink: 0;
  }

  .modal-header h2 {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 700;
    color: var(--text-primary);
  }

  .modal-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: none;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    border-radius: var(--radius-md);
    transition: all var(--transition-fast);
  }

  .modal-close:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .modal-body {
    overflow-y: auto;
    padding: var(--space-lg) var(--space-xl);
  }

  .detail-section {
    margin-bottom: var(--space-xl);
  }

  .detail-section:last-child {
    margin-bottom: 0;
  }

  .detail-section h3 {
    margin: 0 0 var(--space-md);
    font-size: var(--text-xs);
    font-weight: 700;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }

  .detail-grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: var(--space-xs) var(--space-lg);
    align-items: baseline;
  }

  .detail-label {
    font-size: var(--text-sm);
    color: var(--text-faint);
    font-weight: 500;
  }

  .detail-value {
    font-size: var(--text-sm);
    color: var(--text-primary);
    font-weight: 500;
    word-break: break-all;
  }

  .detail-value.mono {
    font-family: var(--font-data);
    font-size: var(--text-xs);
  }

  .services-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }

  .service-item {
    background: var(--bg-body);
    border-radius: var(--radius-md);
    border: 1px solid var(--border-subtle);
    overflow: hidden;
  }

  .service-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-sm) var(--space-md);
    gap: var(--space-sm);
  }

  .service-name {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-secondary);
  }

  .service-uuid {
    font-family: var(--font-data);
    font-size: var(--text-xs);
    color: var(--text-faint);
    flex-shrink: 0;
  }

  .chars-list {
    border-top: 1px solid var(--border-subtle);
  }

  .char-item {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-xs) var(--space-md) var(--space-xs) var(--space-xl);
    font-size: var(--text-xs);
  }

  .char-item + .char-item {
    border-top: 1px solid rgba(255, 255, 255, 0.03);
  }

  .char-name {
    color: var(--text-secondary);
    font-weight: 500;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .char-uuid {
    font-family: var(--font-data);
    color: var(--text-faint);
    flex-shrink: 0;
  }

  .char-props {
    display: flex;
    gap: 3px;
    flex-shrink: 0;
  }

  .char-prop {
    font-size: 0.625rem;
    font-weight: 600;
    color: var(--text-muted);
    padding: 1px 4px;
    border: 1px solid var(--border-subtle);
    border-radius: 3px;
    background: var(--bg-elevated);
  }

  @keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }
</style>
