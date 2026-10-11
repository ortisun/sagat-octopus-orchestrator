import { isApiError } from '../../api/errors';

/**
 * Polling that cannot run away: one request at a time, the pause between them grows after the first
 * few, it stops on a final answer, when the time given for it is up, after too many failures in a
 * row, and when the caller stops it (unmount). The caller never has to remember a timer.
 */
export type PollOutcome<T> =
  | { kind: 'done'; value: T }
  /** The time given ran out before the answer became final. `last` is the last answer, if any. */
  | { kind: 'expired'; last: T | undefined }
  | { kind: 'failed'; error: unknown };

export interface Timers {
  setTimeout: (callback: () => void, ms: number) => unknown;
  clearTimeout: (handle: unknown) => void;
  now: () => number;
}

const browserTimers: Timers = {
  setTimeout: (callback, ms) => globalThis.setTimeout(callback, ms),
  clearTimeout: (handle) => globalThis.clearTimeout(handle as ReturnType<typeof setTimeout>),
  now: () => Date.now(),
};

/**
 * The pause before poll number `attempt` (0 is the first): every second for the first ten, then
 * 1.5 times longer each time, never more than five seconds.
 */
export function pollDelay(attempt: number): number {
  if (attempt < 10) return 1000;
  return Math.min(5000, Math.round(1000 * 1.5 ** (attempt - 9)));
}

export interface PollOptions<T> {
  fetch: (signal: AbortSignal) => Promise<T>;
  /** Whether the answer is final: nothing more will change. */
  isDone: (value: T) => boolean;
  /** Every answer, final or not. */
  onValue?: (value: T) => void;
  /** Called once, when polling ends for any reason but `stop()`. */
  onSettled: (outcome: PollOutcome<T>) => void;
  /** How long the whole thing may last. */
  maxDurationMs: number;
  /** Failures in a row that are tolerated (the network may blink). */
  maxFailures?: number;
  delay?: (attempt: number) => number;
  /** Whether a failure is worth another try. By default: the platform said so, or it was not reachable. */
  isRetryable?: (error: unknown) => boolean;
  timers?: Timers;
}

export interface Poller {
  stop: () => void;
}

function defaultRetryable(error: unknown): boolean {
  return isApiError(error) && (error.retryable || error.code === 'TRANSPORT');
}

export function startPolling<T>(options: PollOptions<T>): Poller {
  const timers = options.timers ?? browserTimers;
  const delay = options.delay ?? pollDelay;
  const maxFailures = options.maxFailures ?? 5;
  const retryable = options.isRetryable ?? defaultRetryable;
  const startedAt = timers.now();
  const controller = new AbortController();
  let handle: unknown;
  let stopped = false;
  let attempt = 0;
  let failures = 0;
  let last: T | undefined;

  const settle = (outcome: PollOutcome<T>) => {
    if (stopped) return;
    stopped = true;
    timers.clearTimeout(handle);
    options.onSettled(outcome);
  };

  const schedule = (ms: number) => {
    if (stopped) return;
    // Never wait past the end of the time given: the last look is at the end.
    const left = options.maxDurationMs - (timers.now() - startedAt);
    if (left <= 0) {
      settle({ kind: 'expired', last });
      return;
    }
    handle = timers.setTimeout(() => void poll(), Math.min(ms, left));
  };

  const poll = async () => {
    if (stopped) return;
    try {
      const value = await options.fetch(controller.signal);
      if (stopped) return;
      failures = 0;
      last = value;
      options.onValue?.(value);
      if (options.isDone(value)) {
        settle({ kind: 'done', value });
        return;
      }
      attempt += 1;
      schedule(delay(attempt));
    } catch (error) {
      if (stopped) return;
      failures += 1;
      if (!retryable(error) || failures >= maxFailures) {
        settle({ kind: 'failed', error });
        return;
      }
      attempt += 1;
      const wait = isApiError(error) && error.retryAfterMs !== undefined ? Math.max(delay(attempt), error.retryAfterMs) : delay(attempt);
      schedule(wait);
    }
  };

  schedule(delay(0));

  return {
    stop: () => {
      if (stopped) return;
      stopped = true;
      timers.clearTimeout(handle);
      controller.abort();
    },
  };
}
