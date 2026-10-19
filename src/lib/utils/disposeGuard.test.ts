import { describe, it, expect, vi } from 'vitest';
import { createDisposeGuard } from './disposeGuard';

describe('createDisposeGuard', () => {
  it('cleanup_added_before_dispose_runs_on_dispose', () => {
    const guard = createDisposeGuard();
    const unlisten = vi.fn();
    guard.add(unlisten);
    expect(unlisten).not.toHaveBeenCalled();
    guard.dispose();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it('cleanup_added_after_dispose_runs_immediately', async () => {
    // Component unmounts while `await listen(...)` is still pending
    const guard = createDisposeGuard();
    const unlisten = vi.fn();
    const listening = Promise.resolve(unlisten);
    guard.dispose();
    guard.add(await listening);
    expect(unlisten).toHaveBeenCalledTimes(1);
    expect(guard.disposed).toBe(true);
  });

  it('dispose_runs_cleanups_once_in_reverse_order', () => {
    const guard = createDisposeGuard();
    const order: string[] = [];
    guard.add(() => order.push('chart'));
    guard.add(() => order.push('observer'));
    guard.dispose();
    guard.dispose();
    expect(order).toEqual(['observer', 'chart']);
  });

  it('throwing_cleanup_does_not_skip_others', () => {
    const guard = createDisposeGuard();
    const other = vi.fn();
    const err = vi.spyOn(console, 'error').mockImplementation(() => {});
    guard.add(other);
    guard.add(() => { throw new Error('boom'); });
    guard.dispose();
    expect(other).toHaveBeenCalledTimes(1);
    expect(err).toHaveBeenCalled();
    err.mockRestore();
  });
});
