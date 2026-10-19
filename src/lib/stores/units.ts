import { writable, derived } from 'svelte/store';

export const unitSystem = writable<'metric' | 'imperial'>('metric');

export function kmhToMph(kmh: number): number {
  return kmh * 0.621371;
}

export function kgToLbs(kg: number): number {
  return kg * 2.20462;
}

export function lbsToKg(lbs: number): number {
  return lbs / 2.20462;
}

export const speedUnit = derived(unitSystem, (u) => (u === 'imperial' ? 'mph' : 'km/h'));
export const weightUnit = derived(unitSystem, (u) => (u === 'imperial' ? 'lbs' : 'kg'));

export function formatSpeed(kmh: number, units: 'metric' | 'imperial'): string {
  const val = units === 'imperial' ? kmhToMph(kmh) : kmh;
  return val.toFixed(1);
}

export function displayWeight(kg: number, units: 'metric' | 'imperial'): number {
  return units === 'imperial' ? Math.round(kgToLbs(kg) * 10) / 10 : kg;
}

/**
 * Convert a weight the user typed from one unit system to another (1 decimal).
 * Non-numeric input (e.g. an emptied field) is returned unchanged.
 */
export function convertDisplayedWeight(
  value: number,
  from: 'metric' | 'imperial',
  to: 'metric' | 'imperial',
): number {
  if (from === to || typeof value !== 'number' || !Number.isFinite(value)) return value;
  const converted = to === 'imperial' ? kgToLbs(value) : lbsToKg(value);
  return Math.round(converted * 10) / 10;
}

export function toStorageWeight(displayed: number, units: 'metric' | 'imperial'): number {
  return units === 'imperial' ? Math.round(lbsToKg(displayed) * 10) / 10 : displayed;
}
