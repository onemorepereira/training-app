import { describe, it, expect, vi, beforeEach } from 'vitest';
import { get } from 'svelte/store';
import { activeZoneStatus, summary, deferred } from '$lib/test/fixtures';

const api = vi.hoisted(() => ({
  startSession: vi.fn(),
  stopSession: vi.fn(),
  stopZoneControl: vi.fn(),
  getZoneControlStatus: vi.fn(),
  saveZoneRideConfig: vi.fn(),
  getSessionStatus: vi.fn(),
}));
vi.mock('$lib/tauri', async (importOriginal) => ({
  ...(await importOriginal<typeof import('$lib/tauri')>()),
  api,
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));

import { sessionActive, sessionId, sessionPaused } from './session';
import { metricHistory } from './sensor';
import { zoneStatus, zoneRideResult, stopZonePolling } from './zoneRide';
import { endSession, startSession, postRideSession, lifecycleError } from './sessionLifecycle';

const ZONE_CONFIG = {
  mode: 'Power' as const,
  zone: 3,
  lower_bound: 150,
  upper_bound: 180,
  duration_secs: 600,
  time_in_zone_secs: 100,
  commanded_power_series: [165],
  time_to_zone_secs: null,
};

function activeSession(id = 'abc-1') {
  sessionActive.set(true);
  sessionId.set(id);
  sessionPaused.set(false);
}

beforeEach(() => {
  vi.clearAllMocks();
  stopZonePolling();
  zoneRideResult.set(null);
  postRideSession.set(null);
  lifecycleError.set('');
  metricHistory.set([]);
  sessionActive.set(false);
  sessionId.set(null);
  sessionPaused.set(false);
  api.stopSession.mockResolvedValue(summary('abc-1'));
  api.stopZoneControl.mockResolvedValue('UserStopped');
  api.saveZoneRideConfig.mockResolvedValue(undefined);
});

describe('endSession', () => {
  it('end_session_stops_active_zone_ride_before_session', async () => {
    activeSession();
    zoneStatus.set(activeZoneStatus());
    const order: string[] = [];
    api.stopZoneControl.mockImplementation(async () => { order.push('zone'); return 'UserStopped'; });
    api.stopSession.mockImplementation(async () => { order.push('session'); return summary('abc-1'); });

    await endSession();

    expect(order).toEqual(['zone', 'session']);
    expect(get(zoneStatus)).toBeNull();
  });

  it('end_session_saves_zone_config_with_summary_id', async () => {
    activeSession('abc-1');
    zoneStatus.set(activeZoneStatus());
    api.stopSession.mockResolvedValue(summary('def-2'));

    await endSession();

    expect(api.saveZoneRideConfig).toHaveBeenCalledTimes(1);
    expect(api.saveZoneRideConfig).toHaveBeenCalledWith('def-2', JSON.stringify(ZONE_CONFIG));
  });

  it('end_session_saves_result_of_previously_completed_zone_ride', async () => {
    activeSession();
    zoneRideResult.set({ config: ZONE_CONFIG, reason: 'DurationComplete', safetyNote: null, error: null });

    await endSession();

    expect(api.stopZoneControl).not.toHaveBeenCalled();
    expect(api.saveZoneRideConfig).toHaveBeenCalledWith('abc-1', JSON.stringify(ZONE_CONFIG));
  });

  it('zone_ride_result_cleared_after_successful_save', async () => {
    activeSession();
    zoneRideResult.set({ config: ZONE_CONFIG, reason: 'DurationComplete', safetyNote: null, error: null });

    await endSession();

    expect(get(zoneRideResult)).toBeNull();
  });

  it('end_session_without_zone_ride_saves_no_config', async () => {
    activeSession();
    await endSession();
    expect(api.saveZoneRideConfig).not.toHaveBeenCalled();
  });

  it('end_session_sets_post_ride_session', async () => {
    activeSession();
    await endSession();
    expect(get(postRideSession)?.id).toBe('abc-1');
    expect(get(sessionActive)).toBe(false);
  });

  it('concurrent_end_session_calls_stop_once', async () => {
    activeSession();
    const stop = deferred<ReturnType<typeof summary>>();
    api.stopSession.mockReturnValue(stop.promise);

    const a = endSession();
    const b = endSession();
    stop.resolve(summary('abc-1'));
    await Promise.all([a, b]);

    expect(api.stopSession).toHaveBeenCalledTimes(1);
  });

  it('end_session_save_failure_still_opens_modal', async () => {
    activeSession();
    zoneStatus.set(activeZoneStatus());
    api.saveZoneRideConfig.mockRejectedValue({ message: 'disk full' });

    await endSession();

    expect(get(postRideSession)?.id).toBe('abc-1');
  });

  it('zone_ride_result_kept_when_save_fails', async () => {
    activeSession();
    zoneStatus.set(activeZoneStatus());
    api.saveZoneRideConfig.mockRejectedValue({ message: 'disk full' });

    await endSession();

    const result = get(zoneRideResult);
    expect(result?.config).toEqual(ZONE_CONFIG);
    expect(result?.error).toBe('Zone ride not saved: disk full');
  });

  it('stop_failure_resets_session_stores_and_surfaces_backend_error', async () => {
    activeSession();
    sessionPaused.set(true);
    api.stopSession.mockRejectedValue({ code: 'Database', message: 'Database error: locked' });

    const result = await endSession();

    expect(result).toBeNull();
    expect(get(sessionActive)).toBe(false);
    expect(get(sessionPaused)).toBe(false);
    expect(get(sessionId)).toBeNull();
    expect(get(lifecycleError)).toBe('Database error: locked');
    expect(get(postRideSession)).toBeNull();
  });

  it('zone_ride_result_kept_when_session_stop_fails', async () => {
    activeSession();
    zoneRideResult.set({ config: ZONE_CONFIG, reason: 'DurationComplete', safetyNote: null, error: null });
    api.stopSession.mockRejectedValue({ message: 'boom' });

    await endSession();

    expect(get(zoneRideResult)?.config).toEqual(ZONE_CONFIG);
  });
});

describe('startSession', () => {
  it('zone_ride_result_cleared_on_session_start', async () => {
    zoneRideResult.set({ config: ZONE_CONFIG, reason: 'DurationComplete', safetyNote: null, error: 'Zone ride not saved: x' });
    api.startSession.mockResolvedValue('new-1');

    await startSession();

    expect(get(zoneRideResult)).toBeNull();
    expect(get(sessionId)).toBe('new-1');
  });

  it('session_start_clears_chart_history', async () => {
    metricHistory.set([{ t: 1, power: 200, hr: 140, cadence: 90, speed: 30 }]);
    api.startSession.mockResolvedValue('new-1');

    await startSession();

    expect(get(metricHistory)).toEqual([]);
  });

  it('failed_session_start_keeps_previous_zone_result', async () => {
    const held = { config: ZONE_CONFIG, reason: 'DurationComplete' as const, safetyNote: null, error: null };
    zoneRideResult.set(held);
    api.startSession.mockRejectedValue({ message: 'no' });

    await expect(startSession()).rejects.toEqual({ message: 'no' });
    expect(get(zoneRideResult)).toEqual(held);
  });
});
