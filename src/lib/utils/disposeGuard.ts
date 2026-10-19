/**
 * Collects cleanups for a component whose setup is asynchronous (event
 * listeners, ECharts instances, ResizeObservers). A cleanup registered after
 * the component was disposed runs immediately, so resources that finish
 * initialising after unmount are not leaked.
 */
export function createDisposeGuard() {
  let disposed = false;
  const cleanups: (() => void)[] = [];

  function run(fn: () => void) {
    try {
      fn();
    } catch (e) {
      console.error('Cleanup failed:', e);
    }
  }

  return {
    get disposed() {
      return disposed;
    },
    add(fn: () => void) {
      if (disposed) run(fn);
      else cleanups.push(fn);
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      for (const fn of cleanups.splice(0).reverse()) run(fn);
    },
  };
}
