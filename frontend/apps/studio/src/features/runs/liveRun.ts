import type { ApiError } from '../../api/errors';
import { isTerminal } from '../../api/follow';
import type { Follow, FollowState } from '../../api/follow';
import type { GraphChanges, GraphEvent, RuntimeGraph, RuntimeNode } from '../../api/types';
import { applyChanges, modelFromGraph } from '../../model/runGraph';
import type { RunModel } from '../../model/runGraph';
import { runStatus } from '../../model/status';
import { isAttention } from './display';
import { idOrder } from './table';

/** Where the live updates of the page are: a followed run's state, or `idle` before there is a graph to follow from. */
export type LiveFollow = FollowState | 'idle';

export interface LiveSnapshot {
  phase: 'loading' | 'ready' | 'error';
  error: unknown;
  model: RunModel | undefined;
  /** The ids of the nodes in a reader's order; recomputed only when the set of nodes changes. */
  order: readonly string[];
  follow: LiveFollow;
  followError: ApiError | undefined;
  /** How far the graph's projection was behind the journal when the graph was read; 0 once updates arrive. */
  lagMs: number;
  /** The nodes the last update changed, and the number of that update (it restarts the highlight). */
  flash: { gen: number; ids: ReadonlySet<string> };
  /** The whole graph is being read again (a gap, or the person asked). */
  reloading: boolean;
  reloadError: unknown;
}

export interface FollowRequest {
  since: number;
  onChanges: (changes: GraphChanges) => void;
  onState: (state: FollowState, error?: ApiError) => void;
}

export interface LiveDeps {
  loadGraph(signal: AbortSignal): Promise<RuntimeGraph>;
  follow(request: FollowRequest): Follow;
  /** The events of each update, for the timeline. */
  onEvents?: (events: readonly GraphEvent[]) => void;
  /** An update replaced the graph: events may have been missed, so the timeline must be read again. */
  onReset?: () => void;
  /** A polite message for assistive technology. */
  announce?: (message: string) => void;
  /** Least time between two automatic reloads of the graph (a gap that repeats must not hammer the platform). */
  minReloadGapMs?: number;
  now?: () => number;
}

const EMPTY_FLASH = { gen: 0, ids: new Set<string>() } as const;
const QUEUE_LIMIT = 200;
const RELOAD_GAP_MS = 2000;
const SAY_AFTER_MS = 150;

const INITIAL: LiveSnapshot = {
  phase: 'loading',
  error: undefined,
  model: undefined,
  order: [],
  follow: 'idle',
  followError: undefined,
  lagMs: 0,
  flash: EMPTY_FLASH,
  reloading: false,
  reloadError: undefined,
};

/** What the person sees of a node; two nodes with the same of these are the same to them. */
function sameNode(a: RuntimeNode, b: RuntimeNode): boolean {
  return (
    a.status === b.status &&
    a.attempt === b.attempt &&
    a.generation === b.generation &&
    a.cancelRequested === b.cancelRequested &&
    a.failureCode === b.failureCode &&
    a.nextAttemptAt === b.nextAttemptAt &&
    a.dueAt === b.dueAt &&
    a.waitingOn === b.waitingOn &&
    a.childRunId === b.childRunId &&
    a.kind === b.kind &&
    a.forward === b.forward &&
    a.type === b.type &&
    JSON.stringify(a.saga ?? null) === JSON.stringify(b.saga ?? null)
  );
}

const WORDS: Record<LiveFollow, { label: string; detail: string }> = {
  idle: { label: 'Not following', detail: 'Waiting for the graph.' },
  connecting: { label: 'Connecting', detail: 'Opening the live updates. Until something happens to the run the platform sends nothing, so this can last.' },
  live: { label: 'Live', detail: 'Changes arrive as they happen.' },
  polling: { label: 'Polling', detail: 'The live stream is not available; the Studio asks for changes every few seconds.' },
  reconnecting: { label: 'Reconnecting', detail: 'The live stream broke; opening it again.' },
  ended: { label: 'Ended', detail: 'The run is over and everything it did has been delivered.' },
  unauthorized: { label: 'Signed out', detail: 'Your session is no longer valid.' },
  failed: { label: 'Not following', detail: 'The run cannot be followed.' },
};

export function followWords(state: LiveFollow): { label: string; detail: string } {
  return WORDS[state];
}

/**
 * The run page's view of one run, held outside React (so the rules are tested without it): the
 * graph read once, then kept up to date with the updates `followRun` delivers. An update names the
 * nodes it touched with their whole state; nothing else is replaced. If an update does not follow
 * on from what is held (`gap`) the graph is read again, at most every couple of seconds.
 */
export class LiveRunStore {
  readonly #deps: LiveDeps;
  readonly #listeners = new Set<() => void>();
  #snapshot: LiveSnapshot = INITIAL;
  #model: RunModel | undefined;
  #order: readonly string[] = [];
  #abort = new AbortController();
  #follow: Follow | undefined;
  #queue: GraphChanges[] = [];
  #reloading = false;
  #lastReload = Number.NEGATIVE_INFINITY;
  #timer: ReturnType<typeof setTimeout> | undefined;
  #flashGen = 0;
  #announcedState: FollowState | undefined;
  #saying: string[] = [];
  #sayTimer: ReturnType<typeof setTimeout> | undefined;

  constructor(deps: LiveDeps) {
    this.#deps = deps;
  }

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  getSnapshot = (): LiveSnapshot => this.#snapshot;

  start(): void {
    this.#abort = new AbortController();
    this.#model = undefined;
    this.#order = [];
    this.#queue = [];
    this.#reloading = false;
    this.#flashGen = 0;
    this.#announcedState = undefined;
    this.#saying = [];
    clearTimeout(this.#sayTimer);
    this.#sayTimer = undefined;
    this.#snapshot = INITIAL;
    this.#emit();
    void this.#loadFirst();
  }

  dispose(): void {
    this.#abort.abort();
    this.#follow?.close();
    this.#follow = undefined;
    clearTimeout(this.#timer);
    this.#timer = undefined;
    clearTimeout(this.#sayTimer);
    this.#sayTimer = undefined;
  }

  /** Reads the whole graph again (a person asked, or a decision found the run had moved on). Resolves when it is read. */
  refresh(): Promise<void> {
    if (this.#model === undefined) return this.#snapshot.phase === 'error' ? this.#loadFirst() : Promise.resolve();
    return this.#reloadNow();
  }

  async #loadFirst(): Promise<void> {
    const signal = this.#abort.signal;
    this.#patch({ phase: 'loading', error: undefined });
    let graph: RuntimeGraph;
    try {
      graph = await this.#deps.loadGraph(signal);
    } catch (error) {
      if (!signal.aborted) this.#patch({ phase: 'error', error });
      return;
    }
    if (signal.aborted) return;
    const model = modelFromGraph(graph);
    this.#model = model;
    this.#order = idOrder(model.nodes);
    this.#patch({ phase: 'ready', model, order: this.#order, lagMs: graph.projectionLagMs ?? 0, flash: EMPTY_FLASH });
    if (isTerminal(model.status)) {
      // Nothing more can happen to a finished run: no stream to open.
      this.#patch({ follow: 'ended' });
      return;
    }
    this.#follow = this.#deps.follow({
      since: model.projectionSeq,
      onChanges: (changes) => this.#changes(changes),
      onState: (state, error) => this.#followState(state, error),
    });
  }

  #changes(changes: GraphChanges): void {
    if (this.#abort.signal.aborted) return;
    if (this.#reloading) {
      this.#enqueue(changes);
      return;
    }
    this.#apply(changes, true);
  }

  #enqueue(changes: GraphChanges): void {
    this.#queue.push(changes);
    // Beyond this the oldest are let go: the next update that does not follow on reads the graph again.
    if (this.#queue.length > QUEUE_LIMIT) this.#queue.splice(0, this.#queue.length - QUEUE_LIMIT);
  }

  #apply(changes: GraphChanges, mayReload: boolean): void {
    const before = this.#model;
    const result = applyChanges(before, changes);
    if (result.kind === 'stale') return;
    if (result.kind === 'gap') {
      if (mayReload) {
        this.#enqueue(changes);
        this.#reloadSoon();
      }
      return;
    }
    const model = result.model;
    const changed = new Set<string>();
    // A node whose update shows nothing new keeps its object (nothing redraws) and is not highlighted.
    const incoming = changes.reset ? [...model.nodes.values()] : changes.nodes;
    for (const node of incoming) {
      const old = before?.nodes.get(node.id);
      if (old !== undefined && sameNode(old, node)) (model.nodes as Map<string, RuntimeNode>).set(node.id, old);
      else changed.add(node.id);
    }
    this.#model = model;
    if (changes.reset || model.nodes.size !== this.#order.length) this.#order = idOrder(model.nodes);
    this.#flashGen += 1;
    this.#patch({ model, order: this.#order, lagMs: 0, flash: { gen: this.#flashGen, ids: changed }, reloadError: undefined });
    if (changes.reset) this.#deps.onReset?.();
    else this.#deps.onEvents?.(changes.events);
    this.#tell(before, model, changed);
  }

  #tell(before: RunModel | undefined, model: RunModel, changed: ReadonlySet<string>): void {
    const parts: string[] = [];
    if (before !== undefined && before.status !== model.status) parts.push(`Run status: ${runStatus(model.status).label}.`);
    const needing: string[] = [];
    for (const id of changed) {
      const now = model.nodes.get(id);
      const old = before?.nodes.get(id);
      if (now !== undefined && isAttention(now) && !(old !== undefined && isAttention(old))) needing.push(id);
    }
    if (needing.length > 0) {
      const shown = needing.slice(0, 3).join(', ');
      parts.push(`${needing.length === 1 ? 'A step needs' : `${needing.length} steps need`} attention: ${shown}${needing.length > 3 ? ` and ${needing.length - 3} more` : ''}.`);
    }
    if (parts.length > 0) this.#say(parts.join(' '));
  }

  /**
   * One polite region says one thing at a time, and a message replaces the one before it: what
   * happens together is said together.
   */
  #say(message: string): void {
    if (this.#deps.announce === undefined) return;
    this.#saying.push(message);
    if (this.#sayTimer !== undefined) return;
    this.#sayTimer = setTimeout(() => {
      this.#sayTimer = undefined;
      const text = this.#saying.join(' ');
      this.#saying = [];
      this.#deps.announce?.(text);
    }, SAY_AFTER_MS);
  }

  #reloadSoon(): void {
    if (this.#reloading || this.#timer !== undefined) return;
    const now = (this.#deps.now ?? Date.now)();
    const wait = this.#lastReload + (this.#deps.minReloadGapMs ?? RELOAD_GAP_MS) - now;
    if (wait <= 0) {
      void this.#reloadNow();
      return;
    }
    this.#timer = setTimeout(() => {
      this.#timer = undefined;
      void this.#reloadNow();
    }, wait);
  }

  async #reloadNow(): Promise<void> {
    if (this.#reloading) return;
    const signal = this.#abort.signal;
    this.#reloading = true;
    this.#lastReload = (this.#deps.now ?? Date.now)();
    this.#patch({ reloading: true, reloadError: undefined });
    try {
      const graph = await this.#deps.loadGraph(signal);
      if (signal.aborted) return;
      const before = this.#model;
      const model = modelFromGraph(graph);
      const changed = new Set<string>();
      for (const [id, node] of model.nodes) {
        const old = before?.nodes.get(id);
        if (old !== undefined && sameNode(old, node)) (model.nodes as Map<string, RuntimeNode>).set(id, old);
        else changed.add(id);
      }
      this.#model = model;
      this.#order = idOrder(model.nodes);
      this.#flashGen += 1;
      this.#patch({ model, order: this.#order, lagMs: graph.projectionLagMs ?? 0, flash: { gen: this.#flashGen, ids: changed } });
      this.#deps.onReset?.();
      this.#tell(before, model, changed);
    } catch (error) {
      if (signal.aborted) return;
      this.#patch({ reloadError: error });
    } finally {
      this.#reloading = false;
      if (!signal.aborted) {
        this.#patch({ reloading: false });
        // What arrived while reading: the old ones are stale and ignored, the rest follows on.
        const queued = this.#queue;
        this.#queue = [];
        for (const changes of queued) this.#apply(changes, false);
      }
    }
  }

  #followState(state: FollowState, error: ApiError | undefined): void {
    if (this.#abort.signal.aborted) return;
    this.#patch({ follow: state, followError: error });
    if (state !== this.#announcedState && state !== 'connecting') {
      this.#announcedState = state;
      this.#say(`Live updates: ${WORDS[state].label}. ${WORDS[state].detail}`);
    }
  }

  #patch(patch: Partial<LiveSnapshot>): void {
    this.#snapshot = { ...this.#snapshot, ...patch };
    this.#emit();
  }

  #emit(): void {
    for (const listener of [...this.#listeners]) listener();
  }
}
