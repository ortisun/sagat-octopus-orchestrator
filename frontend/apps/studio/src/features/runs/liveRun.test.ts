import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '../../api/errors';
import type { Follow, FollowState } from '../../api/follow';
import type { GraphChanges, GraphEvent, RuntimeGraph } from '../../api/types';
import { changesOf, graphEvent, graphOf, node } from '../../../test/runs';
import { LiveRunStore, followWords } from './liveRun';
import type { FollowRequest, LiveDeps } from './liveRun';

const edges = [
  { from: 'a', to: 'b', kind: 'next' as const },
  { from: 'b', to: 'c', kind: 'next' as const },
];
const initialGraph = (over: Partial<RuntimeGraph> = {}) => graphOf([node('a', 'SUCCEEDED', { attempt: 1 }), node('b', 'RUNNING', { attempt: 1 }), node('c', 'NOT_STARTED')], edges, { projectionSeq: 10, ...over });

function harness(graphs: RuntimeGraph[] | ((n: number) => RuntimeGraph | Promise<RuntimeGraph>) = [initialGraph()], over: Partial<LiveDeps> = {}) {
  const requests: FollowRequest[] = [];
  const closes: ReturnType<typeof vi.fn>[] = [];
  const events: (readonly GraphEvent[])[] = [];
  const announced: string[] = [];
  const resets = vi.fn();
  const signals: AbortSignal[] = [];
  let reads = 0;
  const loadGraph = vi.fn((signal: AbortSignal) => {
    signals.push(signal);
    reads += 1;
    if (typeof graphs === 'function') return Promise.resolve(graphs(reads));
    return Promise.resolve(graphs[Math.min(reads, graphs.length) - 1] as RuntimeGraph);
  });
  const store = new LiveRunStore({
    loadGraph,
    follow: (request) => {
      requests.push(request);
      const close = vi.fn();
      closes.push(close);
      const follow: Follow = { close };
      return follow;
    },
    onEvents: (e) => events.push(e),
    onReset: resets,
    announce: (m) => announced.push(m),
    ...over,
  });
  const push = (changes: GraphChanges) => requests.at(-1)?.onChanges(changes);
  const state = (s: FollowState, error?: ApiError) => requests.at(-1)?.onState(s, error);
  return { store, loadGraph, requests, closes, events, announced, resets, push, state, signals };
}

const settle = () => vi.advanceTimersByTimeAsync(0);

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('LiveRunStore: the first graph', () => {
  it('reads the graph once and follows the run from its sequence, never reading the whole graph again to follow', async () => {
    const h = harness([initialGraph({ projectionLagMs: 1800 })]);
    expect(h.store.getSnapshot().phase).toBe('loading');
    h.store.start();
    await settle();
    const snap = h.store.getSnapshot();
    expect(snap.phase).toBe('ready');
    expect(snap.model?.projectionSeq).toBe(10);
    expect(snap.order).toEqual(['a', 'b', 'c']);
    expect(snap.lagMs).toBe(1800);
    expect(h.requests).toHaveLength(1);
    expect(h.requests[0]?.since).toBe(10);
    expect(h.loadGraph).toHaveBeenCalledTimes(1);
  });

  it('does not follow a run that is over: nothing more can happen to it', async () => {
    const h = harness([initialGraph({ status: 'SUCCEEDED' })]);
    h.store.start();
    await settle();
    expect(h.requests).toHaveLength(0);
    expect(h.store.getSnapshot()).toMatchObject({ phase: 'ready', follow: 'ended' });
  });

  it('keeps the error when the graph cannot be read, and tries again when asked', async () => {
    const failure = new ApiError({ code: 'NOT_FOUND', status: 404, message: 'no such run' });
    let n = 0;
    const h = harness(() => {
      n += 1;
      if (n === 1) throw failure;
      return initialGraph();
    });
    h.store.start();
    await settle();
    // loadGraph threw synchronously inside the promise: the store must treat it as a failed read.
    expect(h.store.getSnapshot()).toMatchObject({ phase: 'error', error: failure });
    expect(h.requests).toHaveLength(0);
    void h.store.refresh();
    await settle();
    expect(h.store.getSnapshot().phase).toBe('ready');
    expect(h.requests).toHaveLength(1);
  });

  it('closes what it opened and ignores what arrives after', async () => {
    const h = harness();
    h.store.start();
    await settle();
    h.store.dispose();
    expect(h.closes[0]).toHaveBeenCalledOnce();
    const before = h.store.getSnapshot();
    h.push(changesOf(10, 11, [node('b', 'SUCCEEDED')]));
    expect(h.store.getSnapshot()).toBe(before);
    expect(h.signals[0]?.aborted).toBe(true);
  });

  it('ignores a graph that arrives after it was stopped, and can start again (a page mounted twice)', async () => {
    const h = harness();
    h.store.start();
    h.store.dispose();
    h.store.start();
    await settle();
    expect(h.requests).toHaveLength(1);
    expect(h.store.getSnapshot().phase).toBe('ready');
  });
});

describe('LiveRunStore: updates', () => {
  async function ready(graphs?: RuntimeGraph[], over?: Partial<LiveDeps>) {
    const h = harness(graphs, over);
    h.store.start();
    await settle();
    return h;
  }

  it('replaces the nodes an update names and keeps the very objects of the others', async () => {
    const h = await ready();
    const before = h.store.getSnapshot().model!;
    h.push(changesOf(10, 12, [node('b', 'SUCCEEDED', { attempt: 1 }), node('c', 'RUNNING', { attempt: 1 })], { status: 'RUNNING', events: [graphEvent(11, 'step.completed', 'b', 1), graphEvent(12, 'step.started', 'c', 1)] }));
    const after = h.store.getSnapshot();
    expect(after.model?.projectionSeq).toBe(12);
    expect(after.model?.nodes.get('a')).toBe(before.nodes.get('a'));
    expect(after.model?.nodes.get('b')?.status).toBe('SUCCEEDED');
    expect(after.model?.nodes.get('c')?.status).toBe('RUNNING');
    expect([...after.flash.ids].sort()).toEqual(['b', 'c']);
    expect(after.flash.gen).toBe(1);
    expect(h.loadGraph).toHaveBeenCalledTimes(1);
  });

  it('does not change the model it was given before (the page holds the old one until it renders)', async () => {
    const h = await ready();
    const before = h.store.getSnapshot().model!;
    h.push(changesOf(10, 11, [node('b', 'FAILED', { attempt: 1, failureCode: 'X' })]));
    expect(before.nodes.get('b')?.status).toBe('RUNNING');
    expect(before.projectionSeq).toBe(10);
  });

  it('does not highlight, or replace, a node an update names without changing anything it shows', async () => {
    const h = await ready();
    const b = h.store.getSnapshot().model!.nodes.get('b');
    h.push(changesOf(10, 11, [node('b', 'RUNNING', { attempt: 1 })], { events: [graphEvent(11, 'step.released', 'b')] }));
    const after = h.store.getSnapshot();
    expect(after.model?.nodes.get('b')).toBe(b);
    expect(after.flash.ids.size).toBe(0);
    expect(after.model?.projectionSeq).toBe(11);
  });

  it('hands the events of each update to the timeline, in order, and takes a repeat for what it is', async () => {
    const h = await ready();
    h.push(changesOf(10, 11, [node('b', 'RUNNING', { attempt: 2 })], { events: [graphEvent(11, 'step.retry_scheduled', 'b', 1)] }));
    h.push(changesOf(10, 11, [node('b', 'RUNNING', { attempt: 2 })], { events: [graphEvent(11, 'step.retry_scheduled', 'b', 1)] }));
    h.push(changesOf(11, 12, [node('b', 'SUCCEEDED', { attempt: 2 })], { events: [graphEvent(12, 'step.completed', 'b', 2)] }));
    expect(h.events.map((e) => e.map((x) => x.seq))).toEqual([[11], [12]]);
    expect(h.loadGraph).toHaveBeenCalledTimes(1);
  });

  it('puts a node the stream introduces in its place in the order', async () => {
    const h = await ready();
    h.push(changesOf(10, 11, [node('a2', 'READY')]));
    expect(h.store.getSnapshot().order).toEqual(['a', 'a2', 'b', 'c']);
  });

  it('a reset replaces the graph, highlights only what really changed, and tells the timeline it may have missed events', async () => {
    const h = await ready();
    const a = h.store.getSnapshot().model!.nodes.get('a');
    h.push(changesOf(0, 40, [node('a', 'SUCCEEDED', { attempt: 1 }), node('b', 'FAILED', { attempt: 4, failureCode: 'HTTP_500' }), node('z', 'READY')], { reset: true, edges: [{ from: 'a', to: 'z', kind: 'next' }], status: 'FAILED' }));
    const snap = h.store.getSnapshot();
    expect([...snap.model!.nodes.keys()].sort()).toEqual(['a', 'b', 'z']);
    expect(snap.model?.nodes.get('a')).toBe(a);
    expect([...snap.flash.ids].sort()).toEqual(['b', 'z']);
    expect(snap.order).toEqual(['a', 'b', 'z']);
    expect(snap.model?.edges).toHaveLength(1);
    expect(snap.model?.projectionSeq).toBe(40);
    expect(h.resets).toHaveBeenCalledOnce();
    expect(h.events).toEqual([]);
  });

  it('clears the lag notice once an update arrives', async () => {
    const h = await ready([initialGraph({ projectionLagMs: 900 })]);
    expect(h.store.getSnapshot().lagMs).toBe(900);
    h.push(changesOf(10, 11, [node('b', 'FAILED', { attempt: 1, failureCode: 'X' })]));
    expect(h.store.getSnapshot().lagMs).toBe(0);
  });
});

describe('LiveRunStore: a gap', () => {
  it('reads the whole graph again when an update does not follow on, then applies what came meanwhile', async () => {
    const reread = initialGraph({ projectionSeq: 30 });
    reread.nodes = [node('a', 'SUCCEEDED', { attempt: 1 }), node('b', 'SUCCEEDED', { attempt: 1 }), node('c', 'RUNNING', { attempt: 1 })];
    const h = harness([initialGraph(), reread], { minReloadGapMs: 2000 });
    h.store.start();
    await settle();
    h.push(changesOf(25, 31, [node('c', 'SUCCEEDED', { attempt: 1 })], { events: [graphEvent(31, 'step.completed', 'c', 1)] }));
    await settle();
    expect(h.loadGraph).toHaveBeenCalledTimes(2);
    const snap = h.store.getSnapshot();
    expect(snap.reloading).toBe(false);
    // The re-read graph is at 30; the queued update from 25 to 31 then follows on and applies.
    expect(snap.model?.projectionSeq).toBe(31);
    expect(snap.model?.nodes.get('b')?.status).toBe('SUCCEEDED');
    expect(snap.model?.nodes.get('c')?.status).toBe('SUCCEEDED');
    expect(h.resets).toHaveBeenCalledOnce();
  });

  it('drops a queued update the re-read graph already covers', async () => {
    const reread = initialGraph({ projectionSeq: 40 });
    const h = harness([initialGraph(), reread]);
    h.store.start();
    await settle();
    h.push(changesOf(25, 31, [node('c', 'RUNNING', { attempt: 1 })]));
    await settle();
    expect(h.store.getSnapshot().model?.projectionSeq).toBe(40);
    expect(h.store.getSnapshot().model?.nodes.get('c')?.status).toBe('NOT_STARTED');
  });

  it('does not read the graph again more than once every couple of seconds, however many updates do not follow on', async () => {
    const h = harness(() => initialGraph({ projectionSeq: 100 }), { minReloadGapMs: 2000 });
    h.store.start();
    await settle();
    // The platform's graph stays behind the updates it streams (a lagging projection): every update is a gap.
    for (let i = 0; i < 6; i++) {
      h.push(changesOf(500 + i * 10, 510 + i * 10, [node('c', 'RUNNING')]));
      await vi.advanceTimersByTimeAsync(500);
    }
    // 3 s passed: the first gap read at once (the first read was long ago), the next one was held back to the end of its window.
    expect(h.loadGraph.mock.calls.length).toBeLessThanOrEqual(3);
    expect(h.loadGraph.mock.calls.length).toBeGreaterThanOrEqual(2);
    await vi.advanceTimersByTimeAsync(10_000);
    const settled = h.loadGraph.mock.calls.length;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(h.loadGraph.mock.calls.length).toBe(settled);
  });

  it('by default waits two seconds between two automatic readings of the graph', async () => {
    const h = harness(() => initialGraph({ projectionSeq: 100 }));
    h.store.start();
    await settle();
    h.push(changesOf(500, 510, [node('c', 'RUNNING')]));
    await settle();
    expect(h.loadGraph).toHaveBeenCalledTimes(2);
    h.push(changesOf(520, 530, [node('c', 'RUNNING')]));
    await vi.advanceTimersByTimeAsync(1500);
    expect(h.loadGraph).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(600);
    expect(h.loadGraph).toHaveBeenCalledTimes(3);
  });

  it('keeps the graph it has, and says so, when reading it again fails', async () => {
    const failure = new ApiError({ code: 'UNAVAILABLE', status: 503, message: 'down', retryable: true });
    let n = 0;
    const h = harness(() => {
      n += 1;
      if (n === 2) throw failure;
      return initialGraph();
    });
    h.store.start();
    await settle();
    h.push(changesOf(25, 31, [node('c', 'RUNNING')]));
    await settle();
    expect(h.store.getSnapshot()).toMatchObject({ reloading: false, reloadError: failure });
    expect(h.store.getSnapshot().model?.projectionSeq).toBe(10);
  });

  it('reads the graph again at once when the person asks, and shows that it is doing so', async () => {
    let release: ((g: RuntimeGraph) => void) | undefined;
    const slow = new Promise<RuntimeGraph>((resolve) => (release = resolve));
    const h = harness((n) => (n === 1 ? initialGraph() : slow));
    h.store.start();
    await settle();
    void h.store.refresh();
    void h.store.refresh();
    await settle();
    expect(h.store.getSnapshot().reloading).toBe(true);
    expect(h.loadGraph).toHaveBeenCalledTimes(2);
    release?.(initialGraph({ projectionSeq: 55 }));
    await settle();
    expect(h.store.getSnapshot()).toMatchObject({ reloading: false });
    expect(h.store.getSnapshot().model?.projectionSeq).toBe(55);
  });
});

describe('LiveRunStore: where the updates come from', () => {
  it('says live, polling, reconnecting and ended in words, and tells assistive technology when it changes', async () => {
    const h = harness();
    h.store.start();
    await settle();
    for (const s of ['connecting', 'live', 'live', 'reconnecting', 'polling', 'live', 'ended'] as const) h.state(s);
    expect(h.store.getSnapshot().follow).toBe('ended');
    // Said together because one region says one thing at a time and the next message replaces the last.
    await vi.advanceTimersByTimeAsync(300);
    expect(h.announced).toEqual([
      'Live updates: Live. Changes arrive as they happen. Live updates: Reconnecting. The live stream broke; opening it again. Live updates: Polling. The live stream is not available; the Studio asks for changes every few seconds. Live updates: Live. Changes arrive as they happen. Live updates: Ended. The run is over and everything it did has been delivered.',
    ]);
    expect(followWords('polling').label).toBe('Polling');
  });

  it('keeps the error that made the stream give up', async () => {
    const h = harness();
    h.store.start();
    await settle();
    const error = new ApiError({ code: 'FORBIDDEN', status: 403, message: 'not yours' });
    h.state('failed', error);
    expect(h.store.getSnapshot()).toMatchObject({ follow: 'failed', followError: error });
  });
});

describe('LiveRunStore: what is announced', () => {
  it('tells when the run changes status and when a step starts needing a person, once per update', async () => {
    const h = harness();
    h.store.start();
    await settle();
    h.push(changesOf(10, 11, [node('b', 'FAILED', { attempt: 3, failureCode: 'HTTP_500' })], { status: 'FAILED' }));
    await vi.advanceTimersByTimeAsync(300);
    h.push(changesOf(11, 12, [node('c', 'WAITING', { waitingOn: 'operator' })], { status: 'MANUAL_REVIEW' }));
    await vi.advanceTimersByTimeAsync(300);
    expect(h.announced).toEqual(['Run status: Failed. A step needs attention: b.', 'Run status: Needs a person. A step needs attention: c.']);
    // What happens together is said together, not one message over the other.
    h.push(changesOf(12, 13, [node('b', 'RUNNING', { attempt: 4 })], { status: 'RUNNING' }));
    await vi.advanceTimersByTimeAsync(40);
    h.push(changesOf(13, 14, [node('c', 'FAILED', { attempt: 1 })], { status: 'FAILED' }));
    await vi.advanceTimersByTimeAsync(300);
    // (c was already waiting for a person; failing is not new attention.)
    expect(h.announced.at(-1)).toBe('Run status: Running. Run status: Failed.');
  });

  it('does not announce a step that was already failing, nor an update that changes nothing that matters', async () => {
    const h = harness([graphOf([node('a', 'FAILED', { attempt: 2 }), node('b', 'RUNNING')], [], { projectionSeq: 10 })]);
    h.store.start();
    await settle();
    h.push(changesOf(10, 11, [node('a', 'FAILED', { attempt: 2, failureCode: 'NOW_KNOWN' }), node('b', 'RUNNING', { attempt: 1 })]));
    await vi.advanceTimersByTimeAsync(300);
    expect(h.announced).toEqual([]);
  });

  it('names at most three steps', async () => {
    const h = harness();
    h.store.start();
    await settle();
    h.push(changesOf(10, 11, ['p', 'q', 'r', 's', 't'].map((id) => node(id, 'FAILED', { attempt: 1 }))));
    await vi.advanceTimersByTimeAsync(300);
    expect(h.announced.at(-1)).toBe('5 steps need attention: p, q, r and 2 more.');
  });
});
