import { writable } from 'svelte/store';

/** Whether retro audio SFX are muted (default: muted) */
export const retroMuted = writable(true);

/** Current retro game score */
export const retroScore = writable(0);
