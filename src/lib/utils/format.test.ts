import { describe, it, expect } from 'vitest';
import { formatDuration, autoTitle } from './format';

describe('formatDuration', () => {
  it('formats 0 seconds', () => {
    expect(formatDuration(0)).toBe('0:00');
  });

  it('formats seconds only', () => {
    expect(formatDuration(45)).toBe('0:45');
  });

  it('formats exact minute', () => {
    expect(formatDuration(60)).toBe('1:00');
  });

  it('formats minutes and seconds', () => {
    expect(formatDuration(125)).toBe('2:05');
  });

  it('formats just under 1 hour', () => {
    expect(formatDuration(3599)).toBe('59:59');
  });

  it('formats exactly 1 hour', () => {
    expect(formatDuration(3600)).toBe('1:00:00');
  });

  it('formats hours minutes seconds', () => {
    expect(formatDuration(3661)).toBe('1:01:01');
  });

  it('formats multi-hour duration', () => {
    expect(formatDuration(7322)).toBe('2:02:02');
  });

  it('pads single-digit minutes in hour format', () => {
    expect(formatDuration(3605)).toBe('1:00:05');
  });
});

describe('autoTitle', () => {
  function isoAtHour(hour: number): string {
    const d = new Date(2025, 0, 15, hour, 0, 0);
    return d.toISOString();
  }

  it('returns Night Ride at midnight', () => {
    expect(autoTitle(isoAtHour(0))).toBe('Night Ride');
  });

  it('returns Night Ride at 4 AM', () => {
    expect(autoTitle(isoAtHour(4))).toBe('Night Ride');
  });

  it('returns Morning Ride at 5 AM (boundary)', () => {
    expect(autoTitle(isoAtHour(5))).toBe('Morning Ride');
  });

  it('returns Morning Ride at 11 AM', () => {
    expect(autoTitle(isoAtHour(11))).toBe('Morning Ride');
  });

  it('returns Afternoon Ride at 12 PM (boundary)', () => {
    expect(autoTitle(isoAtHour(12))).toBe('Afternoon Ride');
  });

  it('returns Afternoon Ride at 4 PM', () => {
    expect(autoTitle(isoAtHour(16))).toBe('Afternoon Ride');
  });

  it('returns Evening Ride at 5 PM (boundary)', () => {
    expect(autoTitle(isoAtHour(17))).toBe('Evening Ride');
  });

  it('returns Evening Ride at 8 PM', () => {
    expect(autoTitle(isoAtHour(20))).toBe('Evening Ride');
  });

  it('returns Night Ride at 9 PM (boundary)', () => {
    expect(autoTitle(isoAtHour(21))).toBe('Night Ride');
  });

  it('returns Night Ride at 11 PM', () => {
    expect(autoTitle(isoAtHour(23))).toBe('Night Ride');
  });
});
