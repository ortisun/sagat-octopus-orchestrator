import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '../../api/errors';
import { pollDelay, startPolling } from './polling';
import type { PollOutcome } from './polling';

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

const unavailable = () => new ApiError({ code: 'UNAVAILABLE', status: 503, message: 'down', retryable: true });

function setup(answers: (n: number) => Promise<string> | string, options: { maxDurationMs?: number; maxFailures?: number } = {}) {
  const outcomes: PollOutcome<string>[] = [];
  const values: string[] = [];
  let calls = 0;
  const poller = startPolling<string>({
    fetch: async () => answers(calls++),
    isDone: (value) => value === 'done',
    onValue: (value) => values.push(value),
    onSettled: (outcome) => outcomes.push(outcome),
    maxDurationMs: options.maxDurationMs ?? 60_000,
    ...(options.maxFailures === undefined ? {} : { maxFailures: options.maxFailures }),
  });
  return { poller, outcomes, values, calls: () => calls };
}

describe('pollDelay', () => {
  it('is a second at first, then backs off to five seconds and no more', () => {
    expect(Array.from({ length: 10 }, (_, i) => pollDelay(i))).toEqual(Array(10).fill(1000));
    expect(pollDelay(10)).toBe(1500);
    expect(pollDelay(11)).toBe(2250);
    expect(pollDelay(50)).toBe(5000);
  });
});

describe('startPolling', () => {
  it('does not ask at once; it asks after the first pause, and stops when the answer is final', async () => {
    const run = setup((n) => (n < 2 ? 'running' : 'done'));
    expect(run.calls()).toBe(0);
    await vi.advanceTimersByTimeAsync(999);
    expect(run.calls()).toBe(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(run.calls()).toBe(1);
    await vi.advanceTimersByTimeAsync(2000);
    expect(run.calls()).toBe(3);
    expect(run.values).toEqual(['running', 'running', 'done']);
    expect(run.outcomes).toEqual([{ kind: 'done', value: 'done' }]);
    // Nothing more is asked once it is final.
    await vi.advanceTimersByTimeAsync(60_000);
    expect(run.calls()).toBe(3);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('never has two requests in flight: the next pause starts when the answer is in', async () => {
    let release: (value: string) => void = () => undefined;
    let calls = 0;
    startPolling<string>({
      fetch: () => {
        calls += 1;
        return new Promise<string>((resolve) => {
          release = resolve;
        });
      },
      isDone: () => false,
      onSettled: () => undefined,
      maxDurationMs: 600_000,
    });
    await vi.advanceTimersByTimeAsync(10_000);
    expect(calls).toBe(1);
    release('running');
    await vi.advanceTimersByTimeAsync(1000);
    expect(calls).toBe(2);
  });

  it('stops when the caller says so (the page was left) and ignores an answer that arrives after', async () => {
    let release: (value: string) => void = () => undefined;
    const outcomes: PollOutcome<string>[] = [];
    const values: string[] = [];
    let aborted = false;
    const poller = startPolling<string>({
      fetch: (signal) => {
        signal.addEventListener('abort', () => {
          aborted = true;
        });
        return new Promise<string>((resolve) => {
          release = resolve;
        });
      },
      isDone: (value) => value === 'done',
      onValue: (value) => values.push(value),
      onSettled: (outcome) => outcomes.push(outcome),
      maxDurationMs: 600_000,
    });
    await vi.advanceTimersByTimeAsync(1000);
    poller.stop();
    expect(aborted).toBe(true);
    release('done');
    await vi.advanceTimersByTimeAsync(10_000);
    expect(values).toEqual([]);
    expect(outcomes).toEqual([]);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('stops before the first request if it is stopped at once', async () => {
    const run = setup(() => 'running');
    run.poller.stop();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(run.calls()).toBe(0);
    expect(run.outcomes).toEqual([]);
  });

  it('gives up when the time given is up, with the last answer it had, and asks no more', async () => {
    const run = setup(() => 'running', { maxDurationMs: 5000 });
    await vi.advanceTimersByTimeAsync(20_000);
    expect(run.outcomes).toEqual([{ kind: 'expired', last: 'running' }]);
    const asked = run.calls();
    expect(asked).toBeGreaterThanOrEqual(4);
    expect(asked).toBeLessThanOrEqual(6);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(run.calls()).toBe(asked);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('backs off between asks after the tenth', async () => {
    const times: number[] = [];
    startPolling<string>({
      fetch: () => {
        times.push(Date.now());
        return Promise.resolve('running');
      },
      isDone: () => false,
      onSettled: () => undefined,
      maxDurationMs: 10 * 60_000,
    });
    await vi.advanceTimersByTimeAsync(60_000);
    const gaps = times.slice(1).map((t, i) => t - (times[i] ?? 0));
    expect(gaps.slice(0, 9).every((gap) => gap === 1000)).toBe(true);
    expect(gaps[gaps.length - 1]).toBe(5000);
    expect(Math.max(...gaps)).toBe(5000);
  });

  it('puts up with a failure that is worth another try, and starts counting again after an answer', async () => {
    const run = setup((n) => {
      if (n === 0 || n === 1) throw unavailable();
      if (n === 2) return 'running';
      if (n === 3) throw unavailable();
      return 'done';
    });
    await vi.advanceTimersByTimeAsync(30_000);
    expect(run.outcomes).toEqual([{ kind: 'done', value: 'done' }]);
  });

  it('gives up after too many failures in a row, with the error', async () => {
    const run = setup(() => {
      throw unavailable();
    }, { maxFailures: 3 });
    await vi.advanceTimersByTimeAsync(60_000);
    expect(run.calls()).toBe(3);
    expect(run.outcomes).toHaveLength(1);
    expect(run.outcomes[0]?.kind).toBe('failed');
  });

  it('does not retry a failure that another try cannot fix (forbidden, not found)', async () => {
    const run = setup(() => {
      throw new ApiError({ code: 'NOT_FOUND', status: 404, message: 'gone' });
    });
    await vi.advanceTimersByTimeAsync(10_000);
    expect(run.calls()).toBe(1);
    expect(run.outcomes[0]?.kind).toBe('failed');
  });

  it('waits at least as long as the platform asked after a rate limit', async () => {
    const times: number[] = [];
    const run = startPolling<string>({
      fetch: () => {
        times.push(Date.now());
        if (times.length === 1) return Promise.reject(new ApiError({ code: 'RATE_LIMITED', status: 429, message: 'slow down', retryable: true, retryAfterMs: 20_000 }));
        return Promise.resolve('done');
      },
      isDone: (value) => value === 'done',
      onSettled: () => undefined,
      maxDurationMs: 600_000,
    });
    await vi.advanceTimersByTimeAsync(40_000);
    expect(times).toHaveLength(2);
    expect((times[1] ?? 0) - (times[0] ?? 0)).toBe(20_000);
    run.stop();
  });
});
