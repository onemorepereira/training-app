/**
 * Single-flight sender that coalesces to the latest value: while a send is in
 * flight, further submits overwrite one pending slot; when the send settles
 * (success or failure), the pending value — if any — is sent exactly once.
 */
export function createLatestWinsSender<T>(
  send: (value: T) => Promise<void>,
  onError: (e: unknown) => void,
): { submit: (value: T) => void } {
  let inFlight = false;
  let pending: { value: T } | null = null;

  async function run(value: T) {
    inFlight = true;
    try {
      await send(value);
    } catch (e) {
      onError(e);
    } finally {
      inFlight = false;
    }
    if (pending) {
      const next = pending.value;
      pending = null;
      void run(next);
    }
  }

  return {
    submit(value: T) {
      if (inFlight) {
        pending = { value };
        return;
      }
      void run(value);
    },
  };
}
