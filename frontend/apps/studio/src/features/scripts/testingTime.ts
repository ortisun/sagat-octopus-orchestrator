/**
 * Fake time for tests that need to say "a minute passes": the clock moves when the test moves it and
 * never by itself, so how loaded the machine is cannot change what a test sees. (A clock that also
 * ran with the real one, as `shouldAdvanceTime` makes it, drifted under load and made the tests
 * flaky.) `setImmediate` stays real, so promises and the fake `fetch` still resolve between steps.
 *
 * Testing Library's own waiting (`findBy*`, `waitFor`) moves a fake clock in steps while it waits, and
 * user-event goes through the same wrapper; that is why `eventually` is used instead of them, and why
 * `userEvent.setup({ delay: null })` is used with this clock.
 */
import { notifyManager } from '@tanstack/react-query';
import { act } from '@testing-library/react';
import { vi } from 'vitest';

type WithJest = { jest?: { advanceTimersByTime: (ms: number) => void } };

export function manualTime(): void {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'] });
  // TanStack Query hands its results to React with `setTimeout(0)`; on a clock that only moves when the test
  // moves it, that would be a wait for nothing (a zero-delay timer set while the clock ticks is due a
  // millisecond later). A microtask is the same hand-over without a clock.
  notifyManager.setScheduler((callback) => queueMicrotask(callback));
  // Testing Library moves a clock it takes for Jest's by this name; with it, user-event does not wait on a clock nobody moves.
  (globalThis as WithJest).jest = { advanceTimersByTime: (ms) => void vi.advanceTimersByTime(ms) };
}

export function restoreTime(): void {
  notifyManager.setScheduler((callback) => setTimeout(callback, 0));
  vi.useRealTimers();
  delete (globalThis as WithJest).jest;
}

/** Lets what is already due finish (promises, the fake platform, effects, timers set for "now") without moving the clock. */
export async function settle(rounds = 8): Promise<void> {
  await act(async () => {
    for (let i = 0; i < rounds; i += 1) {
      await new Promise<void>((resolve) => setImmediate(resolve));
      await vi.advanceTimersByTimeAsync(0);
    }
  });
}

/** Moves the clock by `ms`, running everything that falls due on the way, and lets the result settle. */
export async function jump(ms: number): Promise<void> {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
  await settle();
}

/**
 * Runs `check` until it stops throwing, letting the page settle in between but never moving the
 * clock: the replacement for `findBy*` and `waitFor` with this clock.
 */
export async function eventually<T>(check: () => T, rounds = 200): Promise<T> {
  let last: unknown;
  for (let i = 0; i < rounds; i += 1) {
    try {
      return check();
    } catch (error) {
      last = error;
    }
    await settle(2);
  }
  throw last;
}
