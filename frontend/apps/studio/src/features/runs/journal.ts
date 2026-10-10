import type { GraphEvent, JournalEvent, Page } from '../../api/types';
import { eventTone } from '../../model/events';

/** A line of the timeline. */
export interface Entry {
  seq: number;
  eventType: string;
  occurredAt: string;
  stepId: string | null;
  attempt: number | null;
  /**
   * `journal`: read from the journal, with who did it and the trace. `live`: seen in a live update,
   * which carries neither (they appear when the journal is read again).
   */
  detail: 'journal' | 'live';
  actor: string | undefined;
  traceId: string | null;
  payloadRef: string | null;
}

export function fromJournal(event: JournalEvent): Entry {
  return {
    seq: event.seq,
    eventType: event.eventType,
    occurredAt: event.occurredAt,
    stepId: event.stepId ?? null,
    attempt: event.attempt ?? null,
    detail: 'journal',
    actor: event.actor,
    traceId: event.traceId ?? null,
    payloadRef: event.payloadRef ?? null,
  };
}

export function fromLive(event: GraphEvent): Entry {
  return {
    seq: event.seq,
    eventType: event.eventType,
    occurredAt: event.occurredAt,
    stepId: event.stepId ?? null,
    attempt: event.attempt ?? null,
    detail: 'live',
    actor: undefined,
    traceId: null,
    payloadRef: null,
  };
}

export interface JournalSnapshot {
  /** What can be shown, in sequence order, without repeats. */
  entries: readonly Entry[];
  /** Live events newer than the pages read so far: they show once the pages catch up. */
  hidden: number;
  /** The first page was asked for. */
  started: boolean;
  loading: boolean;
  /** Every page of the journal was read. */
  complete: boolean;
  /** `loadAll` stopped at its limit before the end. */
  capped: boolean;
  /** Older live events were let go to bound memory; reading the journal again shows them. */
  droppedLive: boolean;
  error: unknown;
}

export interface JournalOptions {
  /** Pages `loadAll` reads at most. */
  maxPages?: number;
  /** Live events kept at most. */
  liveCap?: number;
}

const MAX_PAGES = 50;
const LIVE_CAP = 5000;

type FetchPage = (cursor: string | undefined, signal: AbortSignal) => Promise<Page<JournalEvent>>;

/**
 * The journal of a run as the timeline and the attempts of a step read it: pages loaded
 * progressively from the platform, plus the events the live updates bring (deduplicated by
 * sequence). Plain TypeScript with a subscribe/snapshot surface so it is tested without React.
 */
export class JournalStore {
  readonly #fetchPage: FetchPage;
  readonly #maxPages: number;
  readonly #liveCap: number;
  readonly #listeners = new Set<() => void>();
  #full: Entry[] = [];
  #live = new Map<number, Entry>();
  #cursor: string | undefined;
  #complete = false;
  #started = false;
  #loading = false;
  #capped = false;
  #droppedLive = false;
  #error: unknown;
  #abort = new AbortController();
  #inflight: Promise<void> | undefined;
  #snapshot: JournalSnapshot;

  constructor(fetchPage: FetchPage, options: JournalOptions = {}) {
    this.#fetchPage = fetchPage;
    this.#maxPages = options.maxPages ?? MAX_PAGES;
    this.#liveCap = options.liveCap ?? LIVE_CAP;
    this.#snapshot = this.#build();
  }

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  getSnapshot = (): JournalSnapshot => this.#snapshot;

  /** Reads the first page if nobody has. */
  start(): void {
    if (this.#started) return;
    this.#started = true;
    void this.loadMore();
  }

  /** Reads the next page. A page being read is waited for, not asked for twice. */
  loadMore(): Promise<void> {
    this.#started = true;
    if (this.#inflight) return this.#inflight;
    if (this.#complete) return Promise.resolve();
    const generation = this.#abort;
    this.#loading = true;
    this.#error = undefined;
    this.#publish();
    const run = (async () => {
      try {
        const page = await this.#fetchPage(this.#cursor, generation.signal);
        if (generation.signal.aborted) return;
        const last = this.#full.at(-1)?.seq ?? 0;
        const fresh = page.items.filter((item) => item.seq > last).map(fromJournal);
        this.#full = fresh.length === 0 ? this.#full : this.#full.concat(fresh);
        this.#cursor = page.nextCursor ?? undefined;
        this.#complete = page.nextCursor === null || page.nextCursor === undefined;
        // What the journal now covers does not need its live copy.
        const covered = this.#full.at(-1)?.seq ?? 0;
        for (const seq of this.#live.keys()) if (seq <= covered) this.#live.delete(seq);
      } catch (error) {
        if (generation.signal.aborted) return;
        this.#error = error;
      } finally {
        if (!generation.signal.aborted) {
          this.#loading = false;
          this.#inflight = undefined;
          this.#publish();
        }
      }
    })();
    this.#inflight = run;
    return run;
  }

  /** Reads pages until the end of the journal, or the limit. */
  async loadAll(): Promise<void> {
    this.#capped = false;
    for (let pages = 0; !this.#complete; pages++) {
      if (pages >= this.#maxPages) {
        this.#capped = true;
        this.#publish();
        return;
      }
      await this.loadMore();
      if (this.#error !== undefined) return;
    }
  }

  /** Takes in the events of a live update. */
  pushLive(events: readonly GraphEvent[]): void {
    if (events.length === 0) return;
    const covered = this.#full.at(-1)?.seq ?? 0;
    let changed = false;
    for (const event of events) {
      if (event.seq <= covered || this.#live.has(event.seq)) continue;
      this.#live.set(event.seq, fromLive(event));
      changed = true;
    }
    if (!changed) return;
    // The oldest go first; they are in the journal and come back when it is read.
    while (this.#live.size > this.#liveCap) {
      const oldest = this.#live.keys().next();
      if (oldest.done) break;
      this.#live.delete(oldest.value);
      this.#droppedLive = true;
    }
    this.#publish();
  }

  /** Forgets everything read and reads again from the start if it had been reading. */
  reload(): void {
    this.#abort.abort();
    this.#abort = new AbortController();
    this.#inflight = undefined;
    this.#full = [];
    this.#live = new Map();
    this.#cursor = undefined;
    this.#complete = false;
    this.#loading = false;
    this.#capped = false;
    this.#droppedLive = false;
    this.#error = undefined;
    const again = this.#started;
    this.#started = false;
    this.#publish();
    if (again) this.start();
  }

  /** Stops reading. The store can be started again (a page mounted twice does that). */
  dispose(): void {
    this.#abort.abort();
    this.#abort = new AbortController();
    this.#inflight = undefined;
    this.#loading = false;
    this.#started = false;
  }

  #publish(): void {
    this.#snapshot = this.#build();
    for (const listener of [...this.#listeners]) listener();
  }

  #build(): JournalSnapshot {
    const newer = [...this.#live.values()];
    const shown = this.#complete ? newer : [];
    return {
      entries: shown.length === 0 ? this.#full : this.#full.concat(shown),
      hidden: this.#complete ? 0 : newer.length,
      started: this.#started,
      loading: this.#loading,
      complete: this.#complete,
      capped: this.#capped,
      droppedLive: this.#droppedLive,
      error: this.#error,
    };
  }
}

export interface EntryFilter {
  /** Failures, retries and decisions that wait for a person. */
  problems: boolean;
  /** What an operator decided: pause, resume, retry, skip, resolve, stop. */
  decisions: boolean;
  /** One node, by its exact id. */
  stepId: string;
}

export const NO_FILTER: EntryFilter = { problems: false, decisions: false, stepId: '' };

/** The entries that pass the filters. Asking for problems and decisions together shows both. */
export function filterEntries(entries: readonly Entry[], filter: EntryFilter): readonly Entry[] {
  const step = filter.stepId.trim();
  if (!filter.problems && !filter.decisions && step === '') return entries;
  return entries.filter((entry) => {
    if (step !== '' && entry.stepId !== step) return false;
    if (!filter.problems && !filter.decisions) return true;
    const tone = eventTone(entry.eventType);
    return (filter.problems && tone === 'problem') || (filter.decisions && tone === 'decision');
  });
}
