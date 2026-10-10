import type { Page, Run } from '../../api/types';

/** Newest first: by when the run was created, then by id (the platform's ids are time-ordered). */
export function newerFirst(a: Run, b: Run): number {
  if (a.createdAt !== b.createdAt) return a.createdAt < b.createdAt ? 1 : -1;
  return a.runId < b.runId ? 1 : a.runId > b.runId ? -1 : 0;
}

/**
 * What the list holds after the newest page is read again. The page is the truth for the time it
 * covers: a run in it replaces its older copy, and a run that was in that time and is not in it any
 * more (it left a filter by changing status) is dropped. Older runs, loaded earlier, stay. A page
 * without a next cursor is the whole list.
 */
export function mergeNewest(held: ReadonlyMap<string, Run>, page: Page<Run>): Map<string, Run> {
  const next = new Map<string, Run>();
  const oldest = page.items.at(-1);
  const full = page.nextCursor !== null && page.nextCursor !== undefined;
  if (full && oldest !== undefined) {
    for (const run of held.values()) if (newerFirst(run, oldest) > 0) next.set(run.runId, run);
  }
  for (const run of page.items) next.set(run.runId, run);
  return next;
}

export interface RunListSnapshot {
  items: readonly Run[];
  /** The first page: loading, in, or failed with nothing to show. */
  phase: 'loading' | 'ready' | 'error';
  error: unknown;
  /** The last automatic refresh failed (the list shown is the one before). */
  refreshError: unknown;
  /** The person asked to start over and the answer is on its way (the list shown is the old one). */
  refreshing: boolean;
  loadingMore: boolean;
  moreError: unknown;
  hasMore: boolean;
  /** More than `maxItems` were asked for. */
  capped: boolean;
  /** When the newest page was last read, on the browser's clock (shown, never used to decide). */
  updatedAt: number | undefined;
}

export interface RunListDeps {
  fetchPage: (cursor: string | undefined, signal: AbortSignal) => Promise<Page<Run>>;
  refreshMs?: number;
  maxItems?: number;
  visible?: () => boolean;
  now?: () => number;
  onLoaded?: (event: { kind: 'first' | 'more' | 'manual'; count: number; total: number; hasMore: boolean }) => void;
}

const REFRESH_MS = 5000;
const MAX_ITEMS = 5000;

/**
 * The runs of a namespace as the list page shows them: the newest page, read again every few
 * seconds while the tab is visible (and not at all while it is hidden), and older pages added on
 * request. Refreshing reads only the newest page, so an hour on the page costs one small request
 * every few seconds however many pages were loaded; the refresh button starts over.
 */
export class RunListStore {
  readonly #deps: RunListDeps;
  readonly #listeners = new Set<() => void>();
  #held = new Map<string, Run>();
  #tail: string | null | undefined;
  #pages = 0;
  #snapshot: RunListSnapshot = { items: [], phase: 'loading', error: undefined, refreshError: undefined, refreshing: false, loadingMore: false, moreError: undefined, hasMore: false, capped: false, updatedAt: undefined };
  #abort = new AbortController();
  #timer: ReturnType<typeof setTimeout> | undefined;
  #busy = false;
  #stale = false;
  #again = false;
  #lastHead = Number.NEGATIVE_INFINITY;
  #onVisible = () => this.#visibility();

  constructor(deps: RunListDeps) {
    this.#deps = deps;
  }

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  getSnapshot = (): RunListSnapshot => this.#snapshot;

  start(): void {
    this.#abort = new AbortController();
    this.#held = new Map();
    this.#tail = undefined;
    this.#pages = 0;
    this.#busy = false;
    this.#again = false;
    this.#stale = false;
    this.#snapshot = { ...this.#snapshot, items: [], phase: 'loading', error: undefined, refreshError: undefined, refreshing: false, moreError: undefined, hasMore: false, loadingMore: false, capped: false };
    this.#emit();
    document.addEventListener('visibilitychange', this.#onVisible);
    void this.#head('first');
  }

  dispose(): void {
    this.#abort.abort();
    clearTimeout(this.#timer);
    this.#timer = undefined;
    document.removeEventListener('visibilitychange', this.#onVisible);
  }

  /** Starts over from the newest page (what was loaded stays on show until the answer comes). */
  refresh(): Promise<void> {
    clearTimeout(this.#timer);
    this.#timer = undefined;
    return this.#head('manual');
  }

  async loadMore(): Promise<void> {
    const cursor = this.#tail;
    if (cursor === null || cursor === undefined || this.#snapshot.loadingMore || this.#busy) return;
    const signal = this.#abort.signal;
    this.#patch({ loadingMore: true, moreError: undefined });
    try {
      const page = await this.#deps.fetchPage(cursor, signal);
      if (signal.aborted) return;
      for (const run of page.items) this.#held.set(run.runId, run);
      this.#tail = page.nextCursor ?? null;
      this.#pages += 1;
      this.#publish({ loadingMore: false });
      this.#deps.onLoaded?.({ kind: 'more', count: page.items.length, total: this.#held.size, hasMore: this.#tail !== null });
    } catch (error) {
      if (!signal.aborted) this.#patch({ loadingMore: false, moreError: error });
    }
  }

  async #head(kind: 'first' | 'manual' | 'auto'): Promise<void> {
    if (this.#busy) {
      // A person asked to start over while a read is on its way: do it as soon as that one is done.
      if (kind === 'manual') this.#again = true;
      return;
    }
    this.#busy = true;
    const signal = this.#abort.signal;
    if (kind === 'manual') this.#patch({ refreshing: true, refreshError: undefined });
    try {
      const page = await this.#deps.fetchPage(undefined, signal);
      if (signal.aborted) return;
      if (kind === 'manual') {
        this.#held = new Map();
        this.#tail = undefined;
        this.#pages = 0;
      }
      this.#held = mergeNewest(this.#held, page);
      // With nothing older loaded, "more" is whatever the newest page says is after it.
      if (this.#pages === 0) this.#tail = page.nextCursor ?? null;
      this.#pages = Math.max(this.#pages, 1);
      this.#lastHead = (this.#deps.now ?? Date.now)();
      this.#publish({ phase: 'ready', error: undefined, refreshError: undefined, refreshing: false, updatedAt: this.#lastHead });
      if (kind !== 'auto') this.#deps.onLoaded?.({ kind, count: page.items.length, total: this.#held.size, hasMore: this.#tail !== null });
    } catch (error) {
      if (signal.aborted) return;
      // Nothing to show yet: the page shows the failure. Something to show: it stays, with the failure beside it.
      if (this.#snapshot.items.length === 0 && kind !== 'auto') this.#patch({ phase: 'error', error, refreshing: false });
      else this.#patch({ refreshError: error, refreshing: false });
    } finally {
      this.#busy = false;
      if (!signal.aborted) {
        if (this.#again) {
          this.#again = false;
          void this.#head('manual');
        } else {
          this.#schedule();
        }
      }
    }
  }

  #schedule(): void {
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => {
      this.#timer = undefined;
      if (!this.#isVisible()) {
        // Nothing is asked while the tab is hidden; coming back asks at once.
        this.#stale = true;
        return;
      }
      void this.#head('auto');
    }, this.#deps.refreshMs ?? REFRESH_MS);
  }

  #isVisible(): boolean {
    return (this.#deps.visible ?? (() => document.visibilityState !== 'hidden'))();
  }

  #visibility(): void {
    if (this.#abort.signal.aborted || !this.#isVisible()) return;
    const age = (this.#deps.now ?? Date.now)() - this.#lastHead;
    if (this.#stale || age >= (this.#deps.refreshMs ?? REFRESH_MS)) {
      this.#stale = false;
      void this.#head('auto');
    }
  }

  #publish(patch: Partial<RunListSnapshot>): void {
    const max = this.#deps.maxItems ?? MAX_ITEMS;
    const all = [...this.#held.values()].sort(newerFirst);
    const capped = all.length > max;
    this.#patch({ items: capped ? all.slice(0, max) : all, hasMore: this.#tail !== null && this.#tail !== undefined && !capped, capped, ...patch });
  }

  #patch(patch: Partial<RunListSnapshot>): void {
    this.#snapshot = { ...this.#snapshot, ...patch };
    this.#emit();
  }

  #emit(): void {
    for (const listener of [...this.#listeners]) listener();
  }
}
