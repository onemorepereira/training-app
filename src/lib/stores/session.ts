import { writable } from 'svelte/store';

export const sessionActive = writable(false);
export const sessionId = writable<string | null>(null);
export const sessionPaused = writable(false);
export const dashboardView = writable<'gauges' | 'graphs'>('gauges');
