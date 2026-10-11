import { describe, expect, it, vi } from 'vitest';
import type { GraphEvent, JournalEvent, Page } from '../../api/types';
import { ApiError } from '../../api/errors';
import { JournalStore, NO_FILTER, filterEntries, fromJournal, fromLive } from './journal';
import type { Entry } from './journal';

const at = (n: number) => new Date(Date.UTC(2026, 9, 10, 10, 0, n)).toISOString();
const journalEvent = (seq: number, eventType = 'step.started', over: Partial<JournalEvent> = {}): JournalEvent => ({ seq, eventType, occurredAt: at(seq), stepId: 'pay', attempt: 1, actor: 'worker:w1', traceId: 'a'.repeat(32), payloadRef: null, ...over });
const liveEvent = (seq: number, eventType = 'step.started', stepId: string | null = 'pay'): GraphEvent => ({ seq, eventType, occurredAt: at(seq), stepId, attempt: 1 });

/** A journal of `total` events served in pages of `size`, with the cursor "after:<seq>". */
function serve(total: number, size: number) {
  const calls: (string | undefined)[] = [];
  const all = Array.from({ length: total }, (_, i) => journalEvent(i + 1));
  const fetchPage = vi.fn((cursor: string | undefined): Promise<Page<JournalEvent>> => {
    calls.push(cursor);
    const from = cursor === undefined ? 0 : Number(cursor.replace('after:', ''));
    const items = all.slice(from, from + size);
    const end = from + size;
    return Promise.resolve({ items, nextCursor: end < total ? `after:${end}` : null });
  });
  return { fetchPage, calls, all };
}

describe('JournalStore', () => {
  it('reads the journal a page at a time, following the cursor, until the end', async () => {
    const { fetchPage, calls } = serve(450, 200);
    const store = new JournalStore(fetchPage);
    expect(store.getSnapshot()).toMatchObject({ started: false, entries: [], complete: false });
    store.start();
    expect(store.getSnapshot()).toMatchObject({ started: true, loading: true });
    await store.loadMore();
    expect(store.getSnapshot()).toMatchObject({ loading: false, complete: false });
    expect(store.getSnapshot().entries).toHaveLength(200);
    await store.loadMore();
    await store.loadMore();
    expect(store.getSnapshot().entries).toHaveLength(450);
    expect(store.getSnapshot().complete).toBe(true);
    expect(calls).toEqual([undefined, 'after:200', 'after:400']);
    // At the end there is nothing to ask for.
    await store.loadMore();
    expect(fetchPage).toHaveBeenCalledTimes(3);
  });

  it('does not ask for a page twice while one is on its way, and starts only once', async () => {
    const { fetchPage } = serve(50, 20);
    const store = new JournalStore(fetchPage);
    store.start();
    store.start();
    void store.loadMore();
    await store.loadMore();
    expect(fetchPage).toHaveBeenCalledTimes(1);
  });

  it('tells subscribers about each change and lets them stop listening', async () => {
    const { fetchPage } = serve(10, 5);
    const store = new JournalStore(fetchPage);
    const listener = vi.fn();
    const stop = store.subscribe(listener);
    await store.loadMore();
    const calls = listener.mock.calls.length;
    expect(calls).toBeGreaterThanOrEqual(2);
    stop();
    await store.loadMore();
    expect(listener).toHaveBeenCalledTimes(calls);
  });

  it('keeps live events out of sight until the journal pages reach them, and counts them meanwhile', async () => {
    const { fetchPage } = serve(300, 200);
    const store = new JournalStore(fetchPage);
    await store.loadMore();
    store.pushLive([liveEvent(301), liveEvent(302)]);
    expect(store.getSnapshot().hidden).toBe(2);
    expect(store.getSnapshot().entries).toHaveLength(200);
    await store.loadMore();
    const { entries, hidden } = store.getSnapshot();
    expect(hidden).toBe(0);
    // 300 journal events; 301 and 302 are newer than the journal read so far, so they show.
    expect(entries.map((e) => e.seq).slice(-3)).toEqual([300, 301, 302]);
    expect(entries.at(-1)?.detail).toBe('live');
    expect(entries[0]?.detail).toBe('journal');
  });

  it('counts an event once whether the journal or a live update brings it (sequence is the key)', async () => {
    const { fetchPage } = serve(10, 200);
    const store = new JournalStore(fetchPage);
    await store.loadMore();
    store.pushLive([liveEvent(9), liveEvent(10), liveEvent(11), liveEvent(11), liveEvent(12)]);
    expect(store.getSnapshot().entries.map((e) => e.seq)).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    // The journal's copy wins over the live copy of an event it already holds.
    expect(store.getSnapshot().entries[8]?.detail).toBe('journal');
  });

  it('does not tell anybody about a live event it already has', async () => {
    const { fetchPage } = serve(10, 200);
    const store = new JournalStore(fetchPage);
    await store.loadMore();
    store.pushLive([liveEvent(11), liveEvent(12)]);
    const listener = vi.fn();
    store.subscribe(listener);
    store.pushLive([liveEvent(11), liveEvent(12), liveEvent(5)]);
    expect(listener).not.toHaveBeenCalled();
    store.pushLive([liveEvent(13)]);
    expect(listener).toHaveBeenCalledOnce();
  });

  it('drops the live copy of an event once a page covers it', async () => {
    const { fetchPage } = serve(250, 200);
    const store = new JournalStore(fetchPage);
    await store.loadMore();
    store.pushLive([liveEvent(205), liveEvent(240)]);
    await store.loadMore();
    const entries = store.getSnapshot().entries;
    expect(entries).toHaveLength(250);
    expect(entries.filter((e) => e.detail === 'live')).toHaveLength(0);
  });

  it('bounds the live events it keeps, letting the oldest go and saying so', async () => {
    const { fetchPage } = serve(1, 200);
    const store = new JournalStore(fetchPage, { liveCap: 100 });
    await store.loadMore();
    for (let i = 0; i < 5; i++) store.pushLive(Array.from({ length: 50 }, (_, j) => liveEvent(2 + i * 50 + j)));
    const { entries, droppedLive } = store.getSnapshot();
    expect(droppedLive).toBe(true);
    expect(entries).toHaveLength(1 + 100);
    expect(entries.at(-1)?.seq).toBe(251);
    expect(entries[1]?.seq).toBe(152);
  });

  it('reads again from the start on reload, and ignores a page of the read it abandoned', async () => {
    let release: ((page: Page<JournalEvent>) => void) | undefined;
    const slow = new Promise<Page<JournalEvent>>((resolve) => (release = resolve));
    const fetchPage = vi.fn().mockReturnValueOnce(slow).mockResolvedValue({ items: [journalEvent(1), journalEvent(2)], nextCursor: null } satisfies Page<JournalEvent>);
    const store = new JournalStore(fetchPage);
    store.start();
    store.reload();
    await vi.waitFor(() => expect(store.getSnapshot().entries).toHaveLength(2));
    release?.({ items: [journalEvent(1), journalEvent(2), journalEvent(3), journalEvent(4)], nextCursor: null });
    await Promise.resolve();
    await Promise.resolve();
    expect(store.getSnapshot().entries).toHaveLength(2);
    expect(fetchPage).toHaveBeenCalledTimes(2);
  });

  it('does not start reading on reload if it had not started', () => {
    const { fetchPage } = serve(5, 5);
    const store = new JournalStore(fetchPage);
    store.reload();
    expect(fetchPage).not.toHaveBeenCalled();
  });

  it('keeps what it has when a page fails, shows the error, and tries the same page again', async () => {
    const { fetchPage, all } = serve(10, 5);
    const failure = new ApiError({ code: 'UNAVAILABLE', status: 503, message: 'down', retryable: true });
    fetchPage.mockRejectedValueOnce(failure);
    const store = new JournalStore(fetchPage);
    await store.loadMore();
    expect(store.getSnapshot()).toMatchObject({ loading: false, error: failure, entries: [] });
    await store.loadMore();
    expect(store.getSnapshot().error).toBeUndefined();
    expect(store.getSnapshot().entries).toHaveLength(5);
    expect(all).toHaveLength(10);
  });

  it('reads everything up to a limit of pages and says when it stopped there', async () => {
    const { fetchPage } = serve(1000, 10);
    const store = new JournalStore(fetchPage, { maxPages: 5 });
    await store.loadAll();
    expect(store.getSnapshot()).toMatchObject({ capped: true, complete: false });
    expect(store.getSnapshot().entries).toHaveLength(50);
    const whole = new JournalStore(serve(95, 10).fetchPage, { maxPages: 50 });
    await whole.loadAll();
    expect(whole.getSnapshot()).toMatchObject({ capped: false, complete: true });
    expect(whole.getSnapshot().entries).toHaveLength(95);
  });

  it('can be started again after it was stopped (a page mounted twice does that)', async () => {
    const { fetchPage } = serve(5, 5);
    const store = new JournalStore(fetchPage);
    store.start();
    store.dispose();
    store.start();
    await vi.waitFor(() => expect(store.getSnapshot().entries).toHaveLength(5));
  });
});

describe('entries', () => {
  it('says what only the journal knows: who, the trace, the payload digest; a live event carries none', () => {
    const full = fromJournal(journalEvent(7, 'step.completed', { payloadRef: 'f'.repeat(64) }));
    expect(full).toMatchObject({ detail: 'journal', actor: 'worker:w1', traceId: 'a'.repeat(32), payloadRef: 'f'.repeat(64) });
    expect(fromLive(liveEvent(7))).toMatchObject({ detail: 'live', actor: undefined, traceId: null, payloadRef: null });
  });
});

describe('filterEntries', () => {
  const entries: Entry[] = [
    fromJournal(journalEvent(1, 'step.started', { stepId: 'a' })),
    fromJournal(journalEvent(2, 'step.failed', { stepId: 'a' })),
    fromJournal(journalEvent(3, 'step.retry_requested', { stepId: 'a', actor: 'user:alice' })),
    fromJournal(journalEvent(4, 'step.started', { stepId: 'b' })),
    fromJournal(journalEvent(5, 'step.retry_scheduled', { stepId: 'b' })),
    fromJournal(journalEvent(6, 'run.paused', { stepId: null })),
  ];
  const seqs = (f: Partial<typeof NO_FILTER>) => filterEntries(entries, { ...NO_FILTER, ...f }).map((e) => e.seq);

  it('shows everything without filters, as the same list', () => {
    expect(filterEntries(entries, NO_FILTER)).toBe(entries);
  });
  it('only problems', () => expect(seqs({ problems: true })).toEqual([2, 5]));
  it('only decisions', () => expect(seqs({ decisions: true })).toEqual([3, 6]));
  it('problems and decisions together show both', () => expect(seqs({ problems: true, decisions: true })).toEqual([2, 3, 5, 6]));
  it('one node', () => expect(seqs({ stepId: 'b' })).toEqual([4, 5]));
  it('one node and problems', () => expect(seqs({ stepId: 'a', problems: true })).toEqual([2]));
  it('a node id with spaces around it is the node', () => expect(seqs({ stepId: '  b ' })).toEqual([4, 5]));
  it('a node that does not exist shows nothing', () => expect(seqs({ stepId: 'nope' })).toEqual([]));
});
