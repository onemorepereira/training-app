import { it, expect } from 'vitest';

// Guard: local-calendar tests hand-compute expectations for America/Chicago.
// If the TZ pin in vite.config.ts is lost, this fails (e.g. in UTC CI) before
// the date tests fail confusingly.
it('timezone_is_pinned_to_chicago', () => {
  expect(new Date('2025-01-15T12:00:00Z').getTimezoneOffset()).toBe(360); // CST, UTC−6
  expect(new Date('2025-07-15T12:00:00Z').getTimezoneOffset()).toBe(300); // CDT, UTC−5
});
