import { invoke } from '@tauri-apps/api/core';

export interface DeviceInfo {
  id: string;
  name: string | null;
  device_type: 'HeartRate' | 'Power' | 'CadenceSpeed' | 'FitnessTrainer';
  status: 'Disconnected' | 'Connecting' | 'Connected' | 'Reconnecting';
  transport: 'Ble' | 'AntPlus';
  rssi: number | null;
  battery_level: number | null;
  last_seen: string | null;
  manufacturer?: string | null;
  model_number?: string | null;
  serial_number?: string | null;
  device_group?: string | null;
}

export interface SensorReading {
  Power?: { watts: number; epoch_ms: number; device_id: string; pedal_balance?: number };
  HeartRate?: { bpm: number; epoch_ms: number; device_id: string };
  Cadence?: { rpm: number; epoch_ms: number; device_id: string };
  Speed?: { kmh: number; epoch_ms: number; device_id: string };
}

export interface LiveMetrics {
  elapsed_secs: number;
  current_power: number | null;
  avg_power_3s: number | null;
  avg_power_10s: number | null;
  avg_power_30s: number | null;
  normalized_power: number | null;
  tss: number | null;
  intensity_factor: number | null;
  current_hr: number | null;
  current_cadence: number | null;
  current_speed: number | null;
  hr_zone: number | null;
  power_zone: number | null;
  stale_power: boolean;
  stale_hr: boolean;
  stale_cadence: boolean;
  stale_speed: boolean;
}

export interface SessionSummary {
  id: string;
  start_time: string;
  duration_secs: number;
  ftp: number | null;
  avg_power: number | null;
  max_power: number | null;
  normalized_power: number | null;
  tss: number | null;
  intensity_factor: number | null;
  avg_hr: number | null;
  max_hr: number | null;
  avg_cadence: number | null;
  avg_speed: number | null;
  title?: string;
  activity_type?: string;
  rpe?: number;
  notes?: string;
}

export interface SessionConfig {
  ftp: number;
  weight_kg: number;
  hr_zones: [number, number, number, number, number];
  units: 'metric' | 'imperial';
  power_zones: [number, number, number, number, number, number];
  date_of_birth: string | null;
  sex: string | null;
  resting_hr: number | null;
  max_hr: number | null;
}

export interface CharacteristicInfo {
  uuid: string;
  name: string | null;
  properties: string[];
}

export interface ServiceInfo {
  uuid: string;
  name: string | null;
  characteristics: CharacteristicInfo[];
}

export interface DeviceDetails {
  id: string;
  name: string | null;
  device_type: string;
  transport: 'Ble' | 'AntPlus';
  rssi: number | null;
  battery_level: number | null;
  manufacturer: string | null;
  model_number: string | null;
  serial_number: string | null;
  firmware_revision: string | null;
  hardware_revision: string | null;
  software_revision: string | null;
  services: ServiceInfo[];
}

export interface PrereqStatus {
  udev_rules: boolean;
  bluez_installed: boolean;
  bluetooth_service: boolean;
  all_met: boolean;
  pkexec_available: boolean;
}

export interface FixResult {
  success: boolean;
  message: string;
  status: PrereqStatus;
}

/** Extract human-readable message from Tauri command errors.
 *  Backend returns `{ code, message }` objects; this handles both that and plain strings. */
export function extractError(e: unknown): string {
  if (e && typeof e === 'object' && 'message' in e) return String((e as { message: string }).message);
  return String(e);
}

export const api = {
  getKnownDevices: () => invoke<DeviceInfo[]>('get_known_devices'),
  scanDevices: () => invoke<DeviceInfo[]>('scan_devices'),
  connectDevice: (deviceId: string) => invoke<DeviceInfo>('connect_device', { deviceId }),
  getDeviceDetails: (deviceId: string) => invoke<DeviceDetails>('get_device_details', { deviceId }),
  disconnectDevice: (deviceId: string) => invoke<void>('disconnect_device', { deviceId }),
  startSession: () => invoke<string>('start_session'),
  stopSession: () => invoke<SessionSummary | null>('stop_session'),
  pauseSession: () => invoke<void>('pause_session'),
  resumeSession: () => invoke<void>('resume_session'),
  getLiveMetrics: () => invoke<LiveMetrics | null>('get_live_metrics'),
  listSessions: () => invoke<SessionSummary[]>('list_sessions'),
  getUserConfig: () => invoke<SessionConfig>('get_user_config'),
  saveUserConfig: (config: SessionConfig) => invoke<void>('save_user_config', { config }),
  setTrainerPower: (watts: number) => invoke<void>('set_trainer_power', { watts }),
  setTrainerResistance: (level: number) => invoke<void>('set_trainer_resistance', { level }),
  setTrainerSimulation: (grade: number, crr: number, cw: number) =>
    invoke<void>('set_trainer_simulation', { grade, crr, cw }),
  startTrainer: () => invoke<void>('start_trainer'),
  stopTrainer: () => invoke<void>('stop_trainer'),
  exportSessionFit: (sessionId: string) => invoke<string>('export_session_fit', { sessionId }),
  setPrimaryDevice: (deviceType: string, deviceId: string) =>
    invoke<void>('set_primary_device', { deviceType, deviceId }),
  getPrimaryDevices: () => invoke<Record<string, string>>('get_primary_devices'),
  unlinkDevices: (deviceId: string) => invoke<void>('unlink_devices', { deviceId }),
  updateSessionMetadata: (
    sessionId: string,
    title: string | null,
    activityType: string | null,
    rpe: number | null,
    notes: string | null,
  ) =>
    invoke<void>('update_session_metadata', {
      sessionId,
      title,
      activityType,
      rpe,
      notes,
    }),
  deleteSession: (sessionId: string) => invoke<void>('delete_session', { sessionId }),
  checkPrerequisites: () => invoke<PrereqStatus>('check_prerequisites'),
  fixPrerequisites: () => invoke<FixResult>('fix_prerequisites'),
};
