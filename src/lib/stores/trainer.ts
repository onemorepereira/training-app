import { writable } from 'svelte/store';

export type TrainerMode = 'erg' | 'resistance' | 'simulation';

export interface TrainerState {
  mode: TrainerMode;
  ergTarget: number;
  resistanceLevel: number;
  simGrade: number;
  simCrr: number;
  simCw: number;
}

export const trainerState = writable<TrainerState>({
  mode: 'erg',
  ergTarget: 150,
  resistanceLevel: 50,
  simGrade: 0,
  simCrr: 0.004,
  simCw: 0.51,
});

export const trainerError = writable<string>('');
