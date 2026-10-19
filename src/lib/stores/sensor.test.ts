import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { get } from 'svelte/store';
import type { SensorReading } from '$lib/tauri';

const handlers = vi.hoisted(() => new Map<string, (e: { payload: unknown }) => void>());
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
    handlers.set(name, cb);
    return () => handlers.delete(name);
  }),
}));

import { sessionActive, sessionPaused } from './session';
import { metricHistory, startSensorListening, stopSensorListening, MAX_HISTORY } from './sensor';
import { appendSample, buildSample } from '$lib/utils/metricHistory';

const emit = (r: SensorReading) => handlers.get('sensor_reading')!({ payload: r });
const power = (watts: number): SensorReading => ({ Power: { watts, epoch_ms: 0, device_id: 'p' } });
const hr = (bpm: number): SensorReading => ({ HeartRate: { bpm, epoch_ms: 0, device_id: 'h' } });

describe('1 Hz chart history', () => {
  beforeEach(async () => {
    vi.useFakeTimers();
    await startSensorListening();
    metricHistory.set([]);
    sessionActive.set(true);
    sessionPaused.set(false);
  });
  afterEach(() => {
    stopSensorListening();
    sessionActive.set(false);
    sessionPaused.set(false);
    vi.useRealTimers();
  });

  it('ten_events_in_one_second_produce_one_sample', async () => {
    for (let i = 0; i < 10; i++) {
      emit(i % 2 ? power(200 + i) : hr(140));
      await vi.advanceTimersByTimeAsync(99);
    }
    await vi.advanceTimersByTimeAsync(10); // reach the 1 s tick
    const h = get(metricHistory);
    expect(h).toHaveLength(1);
    expect(h[0].power).toBe(209);
    expect(h[0].hr).toBe(140);
  });

  it('one_sample_per_second_regardless_of_sensor_count', async () => {
    for (let s = 0; s < 5; s++) {
      emit(power(200));
      emit(hr(140));
      await vi.advanceTimersByTimeAsync(1000);
    }
    expect(get(metricHistory)).toHaveLength(5);
  });

  it('history_caps_at_600_samples', async () => {
    expect(MAX_HISTORY).toBe(600);
    for (let s = 0; s < 700; s++) {
      emit(power(s));
      await vi.advanceTimersByTimeAsync(1000);
    }
    const h = get(metricHistory);
    expect(h).toHaveLength(600);
    expect(h[599].power).toBe(699);
    expect(h[0].power).toBe(100);
  });

  it('no_samples_while_paused', async () => {
    sessionPaused.set(true);
    for (let s = 0; s < 5; s++) {
      emit(power(200));
      await vi.advanceTimersByTimeAsync(1000);
    }
    expect(get(metricHistory)).toHaveLength(0);
  });

  it('no_samples_without_active_session', async () => {
    sessionActive.set(false);
    emit(power(200));
    await vi.advanceTimersByTimeAsync(3000);
    expect(get(metricHistory)).toHaveLength(0);
  });

  it('stale_metric_sampled_as_null', async () => {
    emit(hr(150)); // HR strap then goes silent
    for (let s = 0; s < 7; s++) {
      emit(power(200));
      await vi.advanceTimersByTimeAsync(1000);
    }
    const h = get(metricHistory);
    expect(h[4].hr).toBe(150); // 5 s after last reading: still fresh
    expect(h[5].hr).toBeNull(); // 6 s: stale
    expect(h[6].power).toBe(200);
  });
});

describe('metricHistory helpers', () => {
  it('append_sample_drops_oldest_beyond_max', () => {
    const h = [1, 2, 3].map((t) => ({ t, power: t, hr: null, cadence: null, speed: null }));
    const next = appendSample(h, { t: 4, power: 4, hr: null, cadence: null, speed: null }, 3);
    expect(next.map((e) => e.t)).toEqual([2, 3, 4]);
  });

  it('build_sample_nulls_never_seen_channel', () => {
    const s = buildSample(
      { power: 210, hr: null, cadence: null, speed: null },
      { power: 9_000, hr: 0, cadence: 0, speed: 0 },
      10_000,
      5000,
    );
    expect(s).toEqual({ t: 10_000, power: 210, hr: null, cadence: null, speed: null });
  });
});
