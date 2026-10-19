import { describe, it, expect, vi } from 'vitest';
import { createLatestWinsSender } from './latestWins';
import { deferred } from '$lib/test/fixtures';
import { extractError } from '$lib/tauri';

/** Send mock whose calls each return a promise the test resolves/rejects. */
function controlledSend() {
  const calls: { value: number; d: ReturnType<typeof deferred<void>> }[] = [];
  const send = vi.fn((value: number) => {
    const d = deferred<void>();
    calls.push({ value, d });
    return d.promise;
  });
  return { send, calls };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe('createLatestWinsSender', () => {
  it('idle_submit_sends_immediately', () => {
    const { send } = controlledSend();
    const sender = createLatestWinsSender(send, vi.fn());
    sender.submit(150);
    expect(send).toHaveBeenCalledWith(150);
  });

  it('rapid_submits_during_flight_send_only_latest_once', async () => {
    const { send, calls } = controlledSend();
    const sender = createLatestWinsSender(send, vi.fn());
    sender.submit(150);
    sender.submit(155);
    sender.submit(160);
    sender.submit(165);
    expect(send).toHaveBeenCalledTimes(1);

    calls[0].d.resolve();
    await flush();
    expect(send).toHaveBeenCalledTimes(2);
    expect(send).toHaveBeenLastCalledWith(165);

    calls[1].d.resolve();
    await flush();
    expect(send).toHaveBeenCalledTimes(2);
  });

  it('no_follow_up_when_nothing_pending', async () => {
    const { send, calls } = controlledSend();
    const sender = createLatestWinsSender(send, vi.fn());
    sender.submit(150);
    calls[0].d.resolve();
    await flush();
    expect(send).toHaveBeenCalledTimes(1);
  });

  it('rejected_send_reports_error_and_still_sends_pending', async () => {
    const { send, calls } = controlledSend();
    const onError = vi.fn();
    const sender = createLatestWinsSender(send, onError);
    sender.submit(150);
    sender.submit(170);
    calls[0].d.reject({ message: 'timeout' });
    await flush();
    expect(onError).toHaveBeenCalledWith({ message: 'timeout' });
    expect(send).toHaveBeenLastCalledWith(170);
  });

  it('rejected_send_surfaces_backend_error_text', async () => {
    // Area B: manual trainer commands are rejected while a zone ride runs.
    const { send, calls } = controlledSend();
    const errors: string[] = [];
    const sender = createLatestWinsSender(send, (e) => errors.push(extractError(e)));
    sender.submit(150);
    calls[0].d.reject({ code: 'Session', message: 'Zone ride active — stop it first' });
    await flush();
    expect(errors).toEqual(['Zone ride active — stop it first']);
    expect(send).toHaveBeenCalledTimes(1);
  });

  it('submit_after_idle_sends_again', async () => {
    const { send, calls } = controlledSend();
    const sender = createLatestWinsSender(send, vi.fn());
    sender.submit(150);
    calls[0].d.resolve();
    await flush();
    sender.submit(200);
    expect(send).toHaveBeenCalledTimes(2);
    expect(send).toHaveBeenLastCalledWith(200);
  });
});
