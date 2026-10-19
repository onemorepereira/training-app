import { describe, it, expect } from 'vitest';
import { deviceAction, isPrimaryDevice } from './deviceActions';
import { isConnectCancelled } from '$lib/tauri';

describe('deviceAction', () => {
  it('disconnected_in_range_offers_connect', () => {
    expect(deviceAction('Disconnected', { inRange: true })).toEqual({
      kind: 'connect', label: 'Connect', disabled: false, busy: false, searching: false,
    });
  });

  it('disconnected_out_of_range_connect_disabled', () => {
    expect(deviceAction('Disconnected', { inRange: false }).disabled).toBe(true);
  });

  it('connected_offers_disconnect', () => {
    const a = deviceAction('Connected', {});
    expect(a.kind).toBe('disconnect');
    expect(a.label).toBe('Disconnect');
  });

  it('reconnecting_offers_stop_not_connect', () => {
    // Previously any non-Connected status meant "connect".
    expect(deviceAction('Reconnecting', { inRange: false })).toEqual({
      kind: 'stop', label: 'Stop', disabled: false, busy: false, searching: true,
    });
  });

  it('local_connect_in_flight_offers_cancel', () => {
    expect(deviceAction('Disconnected', { connecting: true })).toEqual({
      kind: 'stop', label: 'Cancel', disabled: false, busy: true, searching: false,
    });
  });

  it('backend_connecting_offers_cancel', () => {
    expect(deviceAction('Connecting', {}).kind).toBe('stop');
  });

  it('disconnect_in_flight_is_disabled', () => {
    const a = deviceAction('Connected', { disconnecting: true });
    expect(a.disabled).toBe(true);
    expect(a.busy).toBe(true);
  });
});

describe('isPrimaryDevice', () => {
  const csc = { id: 'csc-1', device_type: 'CadenceSpeed' as const };

  it('cadence_speed_primary_via_new_cadence_key', () => {
    expect(isPrimaryDevice(csc, { Cadence: 'csc-1' })).toBe(true);
  });

  it('cadence_speed_primary_via_new_speed_key', () => {
    expect(isPrimaryDevice(csc, { Speed: 'csc-1', Cadence: 'other' })).toBe(true);
  });

  it('cadence_speed_primary_via_legacy_key', () => {
    expect(isPrimaryDevice(csc, { CadenceSpeed: 'csc-1' })).toBe(true);
  });

  it('new_keys_win_over_stale_legacy_key', () => {
    // Backend reports the new keys pointing elsewhere: the legacy key is ignored.
    expect(isPrimaryDevice(csc, { Cadence: 'other', Speed: 'other', CadenceSpeed: 'csc-1' })).toBe(false);
  });

  it('not_primary_when_no_key_matches', () => {
    expect(isPrimaryDevice(csc, { HeartRate: 'csc-1' })).toBe(false);
  });

  it('heart_rate_uses_its_own_key', () => {
    expect(isPrimaryDevice({ id: 'hr-1', device_type: 'HeartRate' }, { HeartRate: 'hr-1' })).toBe(true);
  });
});

describe('isConnectCancelled', () => {
  it('connect_cancelled_error_is_recognised', () => {
    expect(isConnectCancelled({ code: 'connect_cancelled', message: 'Connect cancelled' })).toBe(true);
  });

  it('other_errors_are_not_cancellations', () => {
    expect(isConnectCancelled({ code: 'ble_error', message: 'timeout' })).toBe(false);
    expect(isConnectCancelled('connect_cancelled')).toBe(false);
    expect(isConnectCancelled(null)).toBe(false);
  });
});
