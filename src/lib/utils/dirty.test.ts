import { describe, it, expect } from 'vitest';
import { configEquals } from './dirty';
import { convertDisplayedWeight } from '$lib/stores/units';
import type { SessionConfig } from '$lib/tauri';

const saved: SessionConfig = {
  ftp: 250,
  weight_kg: 75,
  hr_zones: [120, 140, 160, 175, 190],
  units: 'metric',
  power_zones: [55, 75, 90, 105, 120, 150],
  date_of_birth: null,
  sex: 'male',
  resting_hr: null,
  max_hr: 190,
};
const copy = (over: Partial<SessionConfig> = {}): SessionConfig => ({
  ...saved,
  hr_zones: [...saved.hr_zones],
  power_zones: [...saved.power_zones],
  ...over,
});

describe('configEquals', () => {
  it('an untouched form is not dirty', () => {
    expect(configEquals(saved, copy(), 75)).toBe(true);
  });

  it('toggling metric -> imperial -> metric without edits is not dirty', () => {
    const lbs = convertDisplayedWeight(75, 'metric', 'imperial'); // 165.3
    const back = convertDisplayedWeight(lbs, 'imperial', 'metric'); // 75.0
    expect(configEquals(saved, copy(), back)).toBe(true);
  });

  it('switching units is a change', () => {
    expect(configEquals(saved, copy({ units: 'imperial' }), 165.3)).toBe(false);
  });

  it('an imperial profile shown in lbs compares by its stored kg (165.3 lbs = 75.0 kg)', () => {
    const imperialSaved = copy({ units: 'imperial' });
    expect(configEquals(imperialSaved, copy({ units: 'imperial' }), 165.3)).toBe(true);
  });

  it('editing the weight is a change', () => {
    expect(configEquals(saved, copy(), 76)).toBe(false);
  });

  it('editing one HR zone is a change', () => {
    const c = copy();
    c.hr_zones[2] = 161;
    expect(configEquals(saved, c, 75)).toBe(false);
  });

  it('editing one power zone is a change', () => {
    const c = copy();
    c.power_zones[5] = 151;
    expect(configEquals(saved, c, 75)).toBe(false);
  });

  it('an emptied date field ("") equals no date (null)', () => {
    expect(configEquals(saved, copy({ date_of_birth: '' }), 75)).toBe(true);
  });

  it('filling an empty optional field is a change', () => {
    expect(configEquals(saved, copy({ resting_hr: 50 }), 75)).toBe(false);
  });

  it('clearing an optional field is a change', () => {
    expect(configEquals(saved, copy({ max_hr: null }), 75)).toBe(false);
  });
});
