import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import type { SensorReading } from '$lib/tauri';

const handlers = vi.hoisted(() => new Map<string, (e: { payload: unknown }) => void>());
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
    handlers.set(name, cb);
    return () => handlers.delete(name);
  }),
}));
const lifecycle = vi.hoisted(() => ({
  endSession: vi.fn(async () => null),
  startSession: vi.fn(async () => 'id'),
}));
vi.mock('$lib/stores/sessionLifecycle', async () => {
  const { writable } = await import('svelte/store');
  return { ...lifecycle, lifecycleError: writable('') };
});

import { sessionActive, sessionPaused } from './session';
import { startSensorListening, stopSensorListening } from './sensor';
import { autoSessionEnabled, initAutoSession, destroyAutoSession, isRiderIdle } from './autoSession';

function emit(reading: SensorReading) {
  handlers.get('sensor_reading')!({ payload: reading });
}
const speed = (kmh: number): SensorReading => ({ Speed: { kmh, epoch_ms: 0, device_id: 'sim:speed' } });
const cadence = (rpm: number): SensorReading => ({ Cadence: { rpm, epoch_ms: 0, device_id: 'sim:cadence' } });
const power = (watts: number): SensorReading => ({ Power: { watts, epoch_ms: 0, device_id: 'sim:power' } });

/** Advance one auto-session tick, first re-emitting `readings` so they stay fresh. */
async function tick(...readings: SensorReading[]) {
  readings.forEach(emit);
  await vi.advanceTimersByTimeAsync(1000);
}

describe('auto-stop', () => {
  beforeEach(async () => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    await startSensorListening();
    sessionActive.set(true);
    sessionPaused.set(false);
    autoSessionEnabled.set(true);
    initAutoSession();
  });
  afterEach(() => {
    destroyAutoSession();
    stopSensorListening();
    autoSessionEnabled.set(false);
    sessionActive.set(false);
    sessionPaused.set(false);
    vi.useRealTimers();
  });

  it('paused_with_zero_speed_never_auto_stops', async () => {
    sessionPaused.set(true);
    for (let i = 0; i < 10; i++) await tick(speed(0));
    expect(lifecycle.endSession).not.toHaveBeenCalled();
  });

  it('zero_speed_for_three_ticks_ends_session_once', async () => {
    await tick(speed(0));
    await tick(speed(0));
    expect(lifecycle.endSession).not.toHaveBeenCalled();
    await tick(speed(0));
    expect(lifecycle.endSession).toHaveBeenCalledTimes(1);
  });

  it('two_idle_ticks_then_moving_resets_counter', async () => {
    await tick(speed(0));
    await tick(speed(0));
    await tick(speed(25));
    await tick(speed(0));
    await tick(speed(0));
    expect(lifecycle.endSession).not.toHaveBeenCalled();
  });

  it('fresh_speed_takes_precedence_over_zero_cadence', async () => {
    for (let i = 0; i < 5; i++) await tick(speed(20), cadence(0));
    expect(lifecycle.endSession).not.toHaveBeenCalled();
  });

  it('no_speed_source_falls_back_to_zero_cadence', async () => {
    await tick(cadence(0), power(0));
    await tick(cadence(0), power(0));
    await tick(cadence(0), power(0));
    expect(lifecycle.endSession).toHaveBeenCalledTimes(1);
  });

  it('stale_speed_falls_back_to_cadence', async () => {
    emit(speed(30)); // speed sensor then drops out
    for (let i = 0; i < 5; i++) await tick(cadence(85));
    // Speed now >5 s old: cadence decides, and 0 rpm is idle
    await tick(cadence(0));
    await tick(cadence(0));
    await tick(cadence(0));
    expect(lifecycle.endSession).toHaveBeenCalledTimes(1);
  });

  it('sensor_dropout_never_auto_stops', async () => {
    emit(speed(30));
    emit(cadence(80));
    // Both channels go silent (dropout) while power keeps arriving.
    for (let i = 0; i < 20; i++) await tick(power(150));
    expect(lifecycle.endSession).not.toHaveBeenCalled();
  });

  it('no_sensors_never_auto_stop', async () => {
    for (let i = 0; i < 20; i++) await tick();
    expect(lifecycle.endSession).not.toHaveBeenCalled();
  });
});

describe('isRiderIdle', () => {
  it.each([
    [{ speed: 0, speedFresh: true, cadence: 90, cadenceFresh: true }, true],
    [{ speed: 15, speedFresh: true, cadence: 0, cadenceFresh: true }, false],
    [{ speed: 0, speedFresh: false, cadence: 80, cadenceFresh: true }, false],
    [{ speed: 0, speedFresh: false, cadence: 0, cadenceFresh: true }, true],
    [{ speed: 0, speedFresh: false, cadence: 0, cadenceFresh: false }, null],
    [{ speed: null, speedFresh: false, cadence: null, cadenceFresh: false }, null],
  ])('is_rider_idle_%#', (input, expected) => {
    expect(isRiderIdle(input)).toBe(expected);
  });
});
