import { writable, get } from 'svelte/store';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { extractError } from '$lib/tauri';

/**
 * Real OS fullscreen for the main window. The store mirrors the window's actual
 * state: it re-checks after every resize, because the OS can enter or leave
 * fullscreen on its own (macOS green button or menu, GNOME shortcut).
 */
export const fullscreen = writable(false);
export const fullscreenError = writable('');

let unlisten: (() => void) | null = null;
let debounce: ReturnType<typeof setTimeout> | null = null;
let settle: ReturnType<typeof setTimeout> | null = null;

async function sync() {
  try {
    fullscreen.set(await getCurrentWindow().isFullscreen());
  } catch {
    // Leave the last known state; the next resize re-checks.
  }
}

export async function initFullscreen(): Promise<void> {
  await sync();
  const un = await getCurrentWindow().onResized(() => {
    if (debounce) clearTimeout(debounce);
    if (settle) clearTimeout(settle);
    debounce = setTimeout(sync, 150);
    // macOS can report a stale value mid-animation; check again once it settles.
    settle = setTimeout(sync, 700);
  });
  unlisten = un;
}

export function destroyFullscreen(): void {
  unlisten?.();
  unlisten = null;
  if (debounce) clearTimeout(debounce);
  if (settle) clearTimeout(settle);
}

export async function setFullscreen(on: boolean): Promise<void> {
  fullscreenError.set('');
  try {
    await getCurrentWindow().setFullscreen(on);
    fullscreen.set(on);
  } catch (e) {
    fullscreenError.set(`Couldn't ${on ? 'enter' : 'leave'} fullscreen (${extractError(e)}). Use your system's fullscreen shortcut instead.`);
    await sync();
  }
}

export function toggleFullscreen(): Promise<void> {
  return setFullscreen(!get(fullscreen));
}

/** Shortcut shown in tooltips: macOS binds Ctrl+Cmd+F in its app menu; elsewhere we handle F11. */
export function fullscreenShortcut(): string {
  return typeof navigator !== 'undefined' && /Mac/.test(navigator.userAgent) ? '⌃⌘F' : 'F11';
}
