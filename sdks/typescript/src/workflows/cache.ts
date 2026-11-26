/*
 * The worker's cache of runs (design 044, Technical Plan 4.2 and 11.1): a run's context is reused only when the next
 * lease's window starts exactly where the cached state stopped, S(k − 1); otherwise it is dropped and the run is replayed
 * from the start. A failure, a deadline or a closed run drops it too. At most 256 runs by default, evicted by least
 * recent use; an eviction is reported so that the worker can release a sticky preference (Task 044-14).
 */
import type { RunHandle } from './threaded.js';

export const DEFAULT_CACHE_SIZE = 256;

export class WorkflowRunCache {
  readonly #capacity: number;
  readonly #runs = new Map<string, RunHandle>();
  readonly #onEvict: ((runId: string) => void) | undefined;

  constructor(capacity = DEFAULT_CACHE_SIZE, onEvict?: (runId: string) => void) {
    if (!Number.isSafeInteger(capacity) || capacity < 0) throw new RangeError('the cache size is a count of runs');
    this.#capacity = capacity;
    this.#onEvict = onEvict;
  }

  get size(): number { return this.#runs.size; }

  /** The cached run if its state is at the lease's window start; any other cached state of the run is dropped. */
  take(runId: string, windowStart: number): RunHandle | undefined {
    const run = this.#runs.get(runId);
    if (!run) return undefined;
    this.#runs.delete(runId);
    if (run.broken || run.closed || run.windowStart !== windowStart) {
      run.dispose();
      return undefined;
    }
    return run;
  }

  /** Keeps a run after its completion was answered; evicts the least recently used beyond the capacity. */
  put(runId: string, run: RunHandle): void {
    const previous = this.#runs.get(runId);
    if (previous && previous !== run) previous.dispose();
    this.#runs.delete(runId);
    if (run.broken || run.closed || this.#capacity === 0) {
      run.dispose();
      return;
    }
    this.#runs.set(runId, run);
    while (this.#runs.size > this.#capacity) {
      const [oldest, evicted] = this.#runs.entries().next().value as [string, RunHandle];
      this.#runs.delete(oldest);
      evicted.dispose();
      this.#onEvict?.(oldest);
    }
  }

  /** Drops one run (a failure, a timeout or a `REVIEW` answer). */
  drop(runId: string): void {
    const run = this.#runs.get(runId);
    if (!run) return;
    this.#runs.delete(runId);
    run.dispose();
  }
}
