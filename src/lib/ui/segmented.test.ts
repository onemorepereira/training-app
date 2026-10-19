import { describe, it, expect } from 'vitest';
import { nextIndex } from './segmented';

describe('nextIndex', () => {
  it('ArrowRight moves to the next option', () => {
    expect(nextIndex('ArrowRight', 0, 3)).toBe(1);
  });

  it('ArrowRight on the last option wraps to the first', () => {
    expect(nextIndex('ArrowRight', 2, 3)).toBe(0);
  });

  it('ArrowLeft on the first option wraps to the last', () => {
    expect(nextIndex('ArrowLeft', 0, 3)).toBe(2);
  });

  it('ArrowDown and ArrowUp behave like Right and Left', () => {
    expect(nextIndex('ArrowDown', 1, 3)).toBe(2);
    expect(nextIndex('ArrowUp', 1, 3)).toBe(0);
  });

  it('Home and End jump to the ends', () => {
    expect(nextIndex('Home', 2, 4)).toBe(0);
    expect(nextIndex('End', 0, 4)).toBe(3);
  });

  it('with nothing selected, arrows start from the first option', () => {
    expect(nextIndex('ArrowRight', -1, 3)).toBe(1);
  });

  it('other keys and empty groups do nothing', () => {
    expect(nextIndex('Enter', 0, 3)).toBeNull();
    expect(nextIndex('ArrowRight', 0, 0)).toBeNull();
  });
});
