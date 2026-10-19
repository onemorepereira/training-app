import type { SessionConfig } from '$lib/tauri';
import { toStorageWeight } from '$lib/stores/units';

/** Empty text inputs bind as '' — treat them like an unset value. */
const norm = <T>(v: T | '' | undefined): T | null => (v === '' || v === undefined ? null : v);

/**
 * True when the Settings form (`current` plus the weight as typed, in
 * `current.units`) would save the same values as `saved`. The weight is compared
 * after the same storage rounding Save applies, so toggling units back and forth
 * doesn't count as a change.
 */
export function configEquals(saved: SessionConfig, current: SessionConfig, weightDisplay: number): boolean {
  if (Math.abs(toStorageWeight(weightDisplay, current.units) - saved.weight_kg) >= 0.05) return false;
  if (saved.units !== current.units) return false;
  if (saved.ftp !== current.ftp) return false;
  if (saved.hr_zones.some((z, i) => z !== current.hr_zones[i])) return false;
  if (saved.power_zones.some((z, i) => z !== current.power_zones[i])) return false;
  if (norm(saved.date_of_birth) !== norm(current.date_of_birth)) return false;
  if (norm(saved.sex) !== norm(current.sex)) return false;
  if (norm(saved.resting_hr) !== norm(current.resting_hr)) return false;
  if (norm(saved.max_hr) !== norm(current.max_hr)) return false;
  return true;
}
