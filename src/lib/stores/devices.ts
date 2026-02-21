import { writable, derived } from 'svelte/store';
import type { DeviceInfo } from '$lib/tauri';
import { api } from '$lib/tauri';

export const connectedDevices = writable<DeviceInfo[]>([]);

export const activeDevices = derived(connectedDevices, ($devices) =>
  $devices.filter((d) => d.status === 'Connected')
);

export const trainerConnected = derived(activeDevices, ($active) =>
  $active.some((d) => d.device_type === 'FitnessTrainer')
);

export async function refreshDevices() {
  try {
    const devices = await api.getKnownDevices();
    connectedDevices.set(devices);
  } catch {
    // No devices available yet
  }
}

// --- Reconnection state ---

export interface ReconnectingDevice {
  device_id: string;
  device_type: string;
  attempt: number;
  status: 'reconnecting' | 'reconnected' | 'disconnected';
  timestamp: number;
}

export const reconnectingDevices = writable<Record<string, ReconnectingDevice>>({});

export function handleDeviceReconnecting(payload: {
  device_id: string;
  device_type: string;
  attempt: number;
}) {
  reconnectingDevices.update((d) => ({
    ...d,
    [payload.device_id]: { ...payload, status: 'reconnecting', timestamp: Date.now() },
  }));
}

export function handleDeviceReconnected(deviceId: string) {
  // Update connectedDevices store to reflect reconnected status
  connectedDevices.update((devices) =>
    devices.map((d) => (d.id === deviceId ? { ...d, status: 'Connected' as const } : d))
  );

  reconnectingDevices.update((d) => ({
    ...d,
    [deviceId]: {
      ...d[deviceId],
      device_id: deviceId,
      device_type: d[deviceId]?.device_type ?? '',
      attempt: d[deviceId]?.attempt ?? 0,
      status: 'reconnected',
      timestamp: Date.now(),
    },
  }));
  // Auto-clear after 3s
  setTimeout(() => {
    reconnectingDevices.update((d) => {
      const next = { ...d };
      delete next[deviceId];
      return next;
    });
  }, 3000);
}

export function handleDeviceDisconnected(deviceId: string) {
  // Update connectedDevices store
  connectedDevices.update((devices) =>
    devices.map((d) => (d.id === deviceId ? { ...d, status: 'Disconnected' as const } : d))
  );

  reconnectingDevices.update((d) => {
    if (d[deviceId]) {
      return {
        ...d,
        [deviceId]: { ...d[deviceId], status: 'disconnected', timestamp: Date.now() },
      };
    }
    return d;
  });
}
