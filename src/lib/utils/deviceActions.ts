import type { DeviceInfo, PrimaryDevices } from '$lib/tauri';

export interface DeviceAction {
  /** connect: start a connection; disconnect: drop a live one; stop: cancel a connect or reconnect loop. */
  kind: 'connect' | 'disconnect' | 'stop';
  label: string;
  disabled: boolean;
  /** An operation for this device is in flight (show a spinner). */
  busy: boolean;
  /** The backend is searching for the device (auto-reconnect). */
  searching: boolean;
}

/** Which connection action a device's button offers, from its status and local in-flight state. */
export function deviceAction(
  status: DeviceInfo['status'],
  opts: { connecting?: boolean; disconnecting?: boolean; inRange?: boolean },
): DeviceAction {
  if (opts.disconnecting) {
    return { kind: 'disconnect', label: 'Disconnecting…', disabled: true, busy: true, searching: false };
  }
  if (opts.connecting || status === 'Connecting') {
    // A disconnect cancels the in-flight connect (backend returns connect_cancelled).
    return { kind: 'stop', label: 'Cancel', disabled: false, busy: true, searching: false };
  }
  if (status === 'Reconnecting') {
    return { kind: 'stop', label: 'Stop', disabled: false, busy: false, searching: true };
  }
  if (status === 'Connected') {
    return { kind: 'disconnect', label: 'Disconnect', disabled: false, busy: false, searching: false };
  }
  return { kind: 'connect', label: 'Connect', disabled: opts.inRange === false, busy: false, searching: false };
}

/**
 * get_primary_devices keys a device is primary for. A cadence/speed sensor is
 * reported under the per-metric keys `Cadence`/`Speed`; older backends use the
 * legacy `CadenceSpeed` key, which is consulted only when no new key is present.
 */
function primaryKeys(deviceType: DeviceInfo['device_type'], primaries: PrimaryDevices): (keyof PrimaryDevices)[] {
  if (deviceType !== 'CadenceSpeed') return [deviceType];
  const hasNewKeys = primaries.Cadence != null || primaries.Speed != null;
  return hasNewKeys ? ['Cadence', 'Speed'] : ['CadenceSpeed'];
}

/** Whether the device is the primary source for any metric it provides. */
export function isPrimaryDevice(
  device: Pick<DeviceInfo, 'id' | 'device_type'>,
  primaries: PrimaryDevices,
): boolean {
  return primaryKeys(device.device_type, primaries).some((k) => primaries[k] === device.id);
}
