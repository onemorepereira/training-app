import { describe, it, expect } from 'vitest';
import { hrZoneForBpm } from './zones';

const ZONES: [number, number, number, number, number] = [120, 140, 160, 180, 200];

// Mirrors Rust MetricsCalculator::hr_zone (metrics.rs): first zone with bpm <= upper.
describe('hrZoneForBpm', () => {
  it('bpm_below_z1_upper_is_zone1', () => {
    expect(hrZoneForBpm(80, ZONES)).toBe(1);
  });

  it('bpm_equal_to_z1_upper_is_zone1', () => {
    expect(hrZoneForBpm(120, ZONES)).toBe(1);
  });

  it('bpm_one_above_z1_upper_is_zone2', () => {
    expect(hrZoneForBpm(121, ZONES)).toBe(2);
  });

  it('bpm_equal_to_z4_upper_is_zone4', () => {
    expect(hrZoneForBpm(180, ZONES)).toBe(4);
  });

  it('bpm_above_all_is_zone5', () => {
    expect(hrZoneForBpm(210, ZONES)).toBe(5);
  });
});
