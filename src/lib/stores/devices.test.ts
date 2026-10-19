import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { get } from 'svelte/store';

const api = vi.hoisted(() => ({ connectDevice: vi.fn() }));
vi.mock('$lib/tauri', async (importOriginal) => ({
  ...(await importOriginal<typeof import('$lib/tauri')>()),
  api,
}));

import {
  reconnectingDevices,
  handleDeviceReconnecting,
  handleDeviceDisconnected,
  reconnectDevice,
} from './devices';

describe('manual reconnect', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    reconnectingDevices.set({});
    handleDeviceReconnecting({ device_id: 'hr-1', device_type: 'HeartRate', attempt: 3 });
    handleDeviceDisconnected('hr-1'); // auto-reconnect gave up → "disconnected" banner
  });
  afterEach(() => vi.useRealTimers());

  it('manual_reconnect_marks_reconnected_then_clears_after_3s', async () => {
    api.connectDevice.mockResolvedValue({ id: 'hr-1' });

    await reconnectDevice('hr-1');
    expect(get(reconnectingDevices)['hr-1']?.status).toBe('reconnected');

    await vi.advanceTimersByTimeAsync(3000);
    expect(get(reconnectingDevices)['hr-1']).toBeUndefined();
  });

  it('failed_manual_reconnect_keeps_disconnected_banner', async () => {
    api.connectDevice.mockRejectedValue({ message: 'not found' });

    await expect(reconnectDevice('hr-1')).rejects.toEqual({ message: 'not found' });
    await vi.advanceTimersByTimeAsync(5000);
    expect(get(reconnectingDevices)['hr-1']?.status).toBe('disconnected');
  });
});
