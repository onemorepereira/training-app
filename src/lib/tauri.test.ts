import { describe, it, expect } from 'vitest';
import { extractError } from './tauri';

describe('extractError', () => {
  it('extracts message from object', () => {
    expect(extractError({ message: 'fail' })).toBe('fail');
  });

  it('converts plain string', () => {
    expect(extractError('timeout')).toBe('timeout');
  });

  it('converts null', () => {
    expect(extractError(null)).toBe('null');
  });

  it('stringifies object without message', () => {
    expect(extractError({ code: 42 })).toBe('[object Object]');
  });
});
