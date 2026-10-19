import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { get } from 'svelte/store';
import { activeZoneStatus, selfStoppedZoneStatus } from '$lib/test/fixtures';

const api = vi.hoisted(() => ({
  getZoneControlStatus: vi.fn(),
  stopZoneControl: vi.fn(),
  startZoneControl: vi.fn(),
}));
vi.mock('$lib/tauri', async (importOriginal) => ({
  ...(await importOriginal<typeof import('$lib/tauri')>()),
  api,
}));

import {
  zoneStatus,
  zoneRideResult,
  startZoneRide,
  stopZoneRide,
  stopZonePolling,
  completionSnapshot,
  buildZoneRideConfig,
  describeZoneStop,
} from './zoneRide';

/** Advance one poll tick and let the async poll callback (and anything it awaits) settle. */
async function pollTick() {
  await vi.advanceTimersByTimeAsync(1000);
}

describe('zone ride completion', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    api.startZoneControl.mockResolvedValue(undefined);
    api.stopZoneControl.mockResolvedValue('DurationComplete');
    zoneRideResult.set(null);
  });
  afterEach(() => {
    stopZonePolling();
    vi.useRealTimers();
  });

  it('self_completion_collects_reason_and_stops_polling', async () => {
    api.getZoneControlStatus.mockResolvedValueOnce(activeZoneStatus());
    api.getZoneControlStatus.mockResolvedValueOnce(selfStoppedZoneStatus({ time_in_zone_secs: 590 }));
    await startZoneRide({ mode: 'Power', zone: 3, lower_bound: 150, upper_bound: 180, duration_secs: 600 });

    await pollTick(); // active
    await pollTick(); // backend stopped itself

    expect(api.stopZoneControl).toHaveBeenCalledTimes(1);
    expect(get(zoneRideResult)?.reason).toBe('DurationComplete');
    expect(get(zoneStatus)).toBeNull();

    await pollTick();
    await pollTick();
    expect(api.getZoneControlStatus).toHaveBeenCalledTimes(2);
  });

  it('self_completion_holds_config_with_final_time_in_zone', async () => {
    api.getZoneControlStatus.mockResolvedValueOnce(activeZoneStatus({ time_in_zone_secs: 580 }));
    api.getZoneControlStatus.mockResolvedValueOnce(selfStoppedZoneStatus({ time_in_zone_secs: 590 }));
    await startZoneRide({ mode: 'Power', zone: 3, lower_bound: 150, upper_bound: 180, duration_secs: 600 });
    await pollTick();
    await pollTick();

    expect(get(zoneRideResult)?.config).toEqual({
      mode: 'Power',
      zone: 3,
      lower_bound: 150,
      upper_bound: 180,
      duration_secs: 600,
      time_in_zone_secs: 590,
      commanded_power_series: [165],
      time_to_zone_secs: null,
    });
  });

  it('self_completed_ride_saves_non_empty_commanded_power_series', async () => {
    api.getZoneControlStatus.mockResolvedValueOnce(activeZoneStatus({ commanded_power: 172 }));
    // Backend reports commanded_power = null once inactive.
    api.getZoneControlStatus.mockResolvedValueOnce(selfStoppedZoneStatus());
    await startZoneRide({ mode: 'Power', zone: 3, lower_bound: 150, upper_bound: 180, duration_secs: 600 });
    await pollTick();
    await pollTick();

    expect(get(zoneRideResult)?.config.commanded_power_series).toEqual([172]);
  });

  it('self_completion_shows_safety_note', async () => {
    api.stopZoneControl.mockResolvedValue('SensorLost');
    api.getZoneControlStatus.mockResolvedValueOnce(activeZoneStatus({ mode: 'HeartRate' }));
    api.getZoneControlStatus.mockResolvedValueOnce(
      selfStoppedZoneStatus({ mode: 'HeartRate', safety_note: 'HR sensor lost' }),
    );
    await startZoneRide({ mode: 'HeartRate', zone: 2, lower_bound: 120, upper_bound: 140, duration_secs: null });
    await pollTick();
    await pollTick();

    const result = get(zoneRideResult)!;
    expect(describeZoneStop(result)).toBe('Zone ride stopped: sensor lost (HR sensor lost)');
  });

  it('zone_ride_result_kept_when_stop_fails', async () => {
    api.stopZoneControl.mockRejectedValue({ message: 'lock poisoned' });
    api.getZoneControlStatus.mockResolvedValueOnce(activeZoneStatus());
    api.getZoneControlStatus.mockResolvedValueOnce(selfStoppedZoneStatus());
    await startZoneRide({ mode: 'Power', zone: 3, lower_bound: 150, upper_bound: 180, duration_secs: 600 });
    await pollTick();
    await pollTick();

    const result = get(zoneRideResult)!;
    expect(result.reason).toBeNull();
    expect(result.error).toBe('Zone control stop failed: lock poisoned');
    expect(result.config.zone).toBe(3);
  });

  it('manual_zone_stop_records_user_stopped_result', async () => {
    api.stopZoneControl.mockResolvedValue('UserStopped');
    api.getZoneControlStatus.mockResolvedValue(activeZoneStatus());
    await startZoneRide({ mode: 'Power', zone: 3, lower_bound: 150, upper_bound: 180, duration_secs: 600 });
    await pollTick();

    await stopZoneRide();

    expect(get(zoneRideResult)?.reason).toBe('UserStopped');
    expect(get(zoneStatus)).toBeNull();
  });

  it('new_zone_ride_result_replaces_previous', async () => {
    api.stopZoneControl.mockResolvedValue('UserStopped');
    api.getZoneControlStatus.mockResolvedValueOnce(activeZoneStatus({ target_zone: 2 }));
    await startZoneRide({ mode: 'Power', zone: 2, lower_bound: 110, upper_bound: 150, duration_secs: null });
    await pollTick();
    await stopZoneRide();

    api.getZoneControlStatus.mockResolvedValueOnce(activeZoneStatus({ target_zone: 4 }));
    await startZoneRide({ mode: 'Power', zone: 4, lower_bound: 180, upper_bound: 210, duration_secs: null });
    await pollTick();
    await stopZoneRide();

    expect(get(zoneRideResult)?.config.zone).toBe(4);
  });
});

describe('completionSnapshot', () => {
  it('completion_snapshot_falls_back_to_previous_target', () => {
    const prev = activeZoneStatus();
    const next = selfStoppedZoneStatus({
      mode: null,
      target_zone: null,
      lower_bound: null,
      upper_bound: null,
      duration_secs: null,
      time_in_zone_secs: 130,
    });
    const snap = completionSnapshot(prev, next);
    expect(snap.mode).toBe('Power');
    expect(snap.target_zone).toBe(3);
    expect(snap.lower_bound).toBe(150);
    expect(snap.upper_bound).toBe(180);
    expect(snap.duration_secs).toBe(600);
    expect(snap.commanded_power).toBe(165);
    expect(snap.time_in_zone_secs).toBe(130);
    expect(snap.active).toBe(false);
  });

  it('build_config_returns_null_without_mode', () => {
    expect(buildZoneRideConfig(activeZoneStatus({ mode: null }))).toBeNull();
  });
});

describe('describeZoneStop', () => {
  it.each([
    ['DurationComplete', 'Zone ride complete'],
    ['SafetyStop', 'Zone ride stopped: safety stop'],
    ['SensorLost', 'Zone ride stopped: sensor lost'],
    ['TrainerDisconnected', 'Zone ride stopped: trainer disconnected'],
    ['UserStopped', 'Zone ride stopped'],
    [null, 'Zone ride stopped'],
  ] as const)('describe_zone_stop_%s', (reason, text) => {
    expect(describeZoneStop({ reason, safetyNote: null })).toBe(text);
  });
});
