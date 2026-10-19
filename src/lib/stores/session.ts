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
 * Resets sessionActive/sessionId/sessionPaused whether the stop succeeds or
 * fails; errors are rethrown for the caller to surface.
 */
export async function requestStop(): Promise<SessionSummary | null> {
  if (!get(sessionActive)) return null;

  if (stopInflight) return stopInflight;

  const promise = (async () => {
    try {
      return (await api.stopSession()) ?? null;
    } finally {
      // On error the backend has still ended the session (its data is kept on
      // disk), so the UI must not stay in an "active" state it can't stop.
      sessionActive.set(false);
      sessionId.set(null);
      sessionPaused.set(false);
    }
  })();
  stopInflight = promise;
  try {
    return await promise;
  } finally {
    stopInflight = null;
  }
}
