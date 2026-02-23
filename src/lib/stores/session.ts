import { writable, get } from 'svelte/store';
import { api, type SessionSummary } from '$lib/tauri';

export const sessionActive = writable(false);
export const sessionId = writable<string | null>(null);
export const sessionPaused = writable(false);
export const dashboardView = writable<'gauges' | 'graphs' | 'retro'>('gauges');

/** In-flight start promise — ensures only one start request at a time. */
let startInflight: Promise<string> | null = null;

/** In-flight stop promise — ensures only one stop request at a time. */
let stopInflight: Promise<SessionSummary | null> | null = null;

/**
 * Request a session start. If a start is already in-flight, piggybacks on
 * that request. If a session is already active, returns the existing ID.
 * Updates sessionActive/sessionId/sessionPaused atomically on success.
 */
export async function requestStart(): Promise<string> {
  const id = get(sessionId);
  if (get(sessionActive) && id) return id;

  if (startInflight) return startInflight;

  const promise = api.startSession();
  startInflight = promise;
  try {
    const newId = await promise;
    sessionId.set(newId);
    sessionActive.set(true);
    sessionPaused.set(false);
    return newId;
  } finally {
    startInflight = null;
  }
}

/**
 * Request a session stop. If a stop is already in-flight, piggybacks on
 * that request. If no session is active, returns null.
 * Updates sessionActive/sessionId/sessionPaused atomically on success.
 */
export async function requestStop(): Promise<SessionSummary | null> {
  if (!get(sessionActive)) return null;

  if (stopInflight) return stopInflight;

  const promise = api.stopSession();
  stopInflight = promise;
  try {
    const result = await promise;
    sessionActive.set(false);
    sessionId.set(null);
    sessionPaused.set(false);
    return result ?? null;
  } finally {
    stopInflight = null;
  }
}
