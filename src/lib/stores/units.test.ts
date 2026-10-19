import { describe, it, expect } from 'vitest';
import {
  kmhToMph,
  kgToLbs,
  lbsToKg,
  formatSpeed,
  displayWeight,
  toStorageWeight,
  convertDisplayedWeight,
} from './units';

describe('convertDisplayedWeight', () => {
  it('convert_75kg_to_imperial_is_165_3', () => {
    // 75 × 2.20462 = 165.35 → 165.3
    expect(convertDisplayedWeight(75, 'metric', 'imperial')).toBe(165.3);
  });

  it('convert_typed_80kg_to_imperial_is_176_4', () => {
    // 80 × 2.20462 = 176.37 → 176.4
    expect(convertDisplayedWeight(80, 'metric', 'imperial')).toBe(176.4);
  });

  it('convert_165_3lbs_to_metric_is_75_0', () => {
    // 165.3 / 2.20462 = 74.98 → 75.0
    expect(convertDisplayedWeight(165.3, 'imperial', 'metric')).toBe(75);
  });

  it('same_unit_conversion_is_identity', () => {
    expect(convertDisplayedWeight(72.4, 'metric', 'metric')).toBe(72.4);
  });

  it('non_finite_weight_passes_through', () => {
    // An emptied number input binds to null
    expect(convertDisplayedWeight(null as unknown as number, 'metric', 'imperial')).toBeNull();
    expect(convertDisplayedWeight(Number.NaN, 'metric', 'imperial')).toBeNaN();
  });
});

describe('kmhToMph', () => {
  it('converts 100 km/h', () => {
    expect(kmhToMph(100)).toBeCloseTo(62.1371, 4);
  });

  it('returns 0 for 0', () => {
    expect(kmhToMph(0)).toBe(0);
  });
});

describe('kgToLbs', () => {
  it('converts 75 kg', () => {
    expect(kgToLbs(75)).toBeCloseTo(165.3465, 4);
  });

  it('returns 0 for 0', () => {
    expect(kgToLbs(0)).toBe(0);
  });
});

describe('lbsToKg', () => {
  it('roundtrips through kgToLbs', () => {
    expect(lbsToKg(kgToLbs(75))).toBeCloseTo(75, 2);
  });
});

describe('formatSpeed', () => {
  it('formats metric speed', () => {
    expect(formatSpeed(30.5, 'metric')).toBe('30.5');
  });

  it('formats imperial speed', () => {
    expect(formatSpeed(30.5, 'imperial')).toBe('19.0');
  });

  it('formats zero', () => {
    expect(formatSpeed(0, 'metric')).toBe('0.0');
  });
});

describe('displayWeight', () => {
  it('passes through metric', () => {
    expect(displayWeight(75, 'metric')).toBe(75);
  });

  it('converts to imperial', () => {
    expect(displayWeight(75, 'imperial')).toBeCloseTo(165.3, 1);
  });
});

describe('toStorageWeight', () => {
  it('passes through metric', () => {
    expect(toStorageWeight(75, 'metric')).toBe(75);
  });

  it('roundtrips through displayWeight for imperial', () => {
    const displayed = displayWeight(75, 'imperial');
    expect(toStorageWeight(displayed, 'imperial')).toBeCloseTo(75, 1);
  });
});
