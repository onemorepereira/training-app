import { describe, it, expect } from 'vitest';
import { displayTitle } from './session';
import { summary } from '$lib/test/fixtures';

// 2025-06-15T19:00:00Z is 14:00 CDT (tests run with TZ=America/Chicago) → "Afternoon Ride"
const START = '2025-06-15T19:00:00Z';

describe('displayTitle', () => {
  it('display_title_uses_stored_title', () => {
    expect(displayTitle(summary('a', { start_time: START, title: 'Hill repeats' }))).toBe('Hill repeats');
  });

  it('display_title_missing_falls_back_to_auto_title', () => {
    expect(displayTitle(summary('a', { start_time: START }))).toBe('Afternoon Ride');
  });

  it('display_title_empty_falls_back_to_auto_title', () => {
    expect(displayTitle(summary('a', { start_time: START, title: '' }))).toBe('Afternoon Ride');
  });

  it('display_title_whitespace_falls_back_to_auto_title', () => {
    expect(displayTitle(summary('a', { start_time: START, title: '   ' }))).toBe('Afternoon Ride');
  });
});
