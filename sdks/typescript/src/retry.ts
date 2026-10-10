import { SagatError } from './errors.js';

/** How many times, and how patiently, a call that is safe to repeat is repeated (SPEC §5). */
export interface RetryPolicy {
  /** Attempts in all, the first included. */
  maxAttempts: number;
  initialDelayMs: number;
  maxDelayMs: number;
}

export const DEFAULT_RETRY: RetryPolicy = { maxAttempts: 4, initialDelayMs: 200, maxDelayMs: 5000 };

/** `min(max, initial * 2^n)` scaled by a random factor in [0.5, 1.0]; `n` is the attempt that just failed, from 0. */
export function backoffDelayMs(policy: RetryPolicy, n: number, random: () => number = Math.random): number {
  const ceiling = Math.min(policy.maxDelayMs, policy.initialDelayMs * 2 ** n);
  return Math.round(ceiling * (0.5 + random() * 0.5));
}

/** The wait before the next attempt: the backoff, or what the server asked for if that is longer. */
export function delayAfter(policy: RetryPolicy, n: number, error: SagatError, random: () => number = Math.random): number {
  return Math.max(backoffDelayMs(policy, n, random), error.retryAfterMs ?? 0);
}

/** Resolves after `ms`, or rejects with `CANCELLED` as soon as `signal` aborts. */
export function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(new SagatError({ code: 'CANCELLED', message: 'cancelled while waiting' }));
      return;
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener('abort', onAbort);
      resolve();
    }, ms);
    const onAbort = () => {
      clearTimeout(timer);
      reject(new SagatError({ code: 'CANCELLED', message: 'cancelled while waiting' }));
    };
    signal?.addEventListener('abort', onAbort, { once: true });
  });
}

/** A UUID v4, for idempotency keys and event ids. */
export { randomUUID } from 'node:crypto';
