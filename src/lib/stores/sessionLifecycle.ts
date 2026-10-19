import { writable, get } from 'svelte/store';
import { api, extractError, type SessionSummary } from '$lib/tauri';
import { requestStart, requestStop, sessionActive, sessionId, sessionPaused } from '$lib/stores/session';
import { metricHistory } from '$lib/stores/sensor';
import {
  zoneStatus,
  zoneRideResult,
  finishZoneRide,
  startZonePolling,
  stopZonePolling,
} from '$lib/stores/zoneRide';

/** Session summary awaiting the post-ride modal (dashboard renders it). */
export const postRideSession = writable<SessionSummary | null>(null);

/** Last session start/stop error, shown inline on the dashboard. */
export const lifecycleError = writable<string>('');

/**
 * Start a session. On success, resets per-session UI state: live chart history
 * and any zone ride result left over from the previous session.
 */
export async function startSession(): Promise<string> {
  const id = await requestStart();
  metricHistory.set([]);
  zoneRideResult.set(null);
  return id;
}

let endInflight: Promise<SessionSummary | null> | null = null;

/**
 * The single path that ends a session (Stop button, auto-stop):
 * finish any active zone ride, stop the session, persist the zone ride result
 * against the saved session and open the post-ride modal.
 * Errors are surfaced via `lifecycleError`; never throws.
 */
export function endSession(): Promise<SessionSummary | null> {
  if (endInflight) return endInflight;
  endInflight = doEndSession().finally(() => {
    endInflight = null;
  });
  return endInflight;
}

async function doEndSession(): Promise<SessionSummary | null> {
  lifecycleError.set('');

  const snap = get(zoneStatus);
  if (snap?.active) {
    await finishZoneRide(snap);
  } else {
    stopZonePolling();
  }

  let summary: SessionSummary | null;
  try {
    summary = await requestStop();
  } catch (e) {
    lifecycleError.set(extractError(e));
    return null;
  }
  if (!summary) return null;

  const result = get(zoneRideResult);
  if (result) {
    try {
      await api.saveZoneRideConfig(summary.id, JSON.stringify(result.config));
      // Keep a result whose zone-control stop failed so the error stays visible
      // until the next session start; otherwise it has been persisted.
      if (!result.error) zoneRideResult.set(null);
    } catch (e) {
      const saveError = `Zone ride not saved: ${extractError(e)}`;
      zoneRideResult.set({
        ...result,
        error: result.error ? `${result.error}; ${saveError}` : saveError,
      });
    }
  }

  postRideSession.set(summary);
  return summary;
}

/**
 * Restore session UI state from the backend (e.g. after a webview reload while a
 * ride is running), including an active zone ride so it can still be stopped and saved.
 */
export async function rehydrateSession(): Promise<void> {
  try {
    const status = await api.getSessionStatus();
    if (!status) return;
    sessionId.set(status.session_id);
    sessionPaused.set(status.paused);
    sessionActive.set(true);

    const zone = await api.getZoneControlStatus();
    if (zone.active) {
      zoneStatus.set(zone);
      startZonePolling();
    }
  } catch (e) {
    const msg = extractError(e);
    console.error('Session rehydration failed:', msg);
    lifecycleError.set(`Could not restore session state: ${msg}`);
  }
}
