import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '../../api/errors';
import type { Page, Run } from '../../api/types';
import { runOf } from '../../../test/runs';
import { RunListStore, mergeNewest, newerFirst } from './runList';
import type { RunListDeps } from './runList';

/** Run number `n` (1 is the oldest): ids and creation times grow with n, as the platform's do. */
const run = (n: number, over: Partial<Run> = {}): Run => runOf({ runId: `00000000-0000-7000-8000-${String(n).padStart(12, '0')}`, createdAt: new Date(Date.UTC(2026, 9, 10, 10, 0, n)).toISOString(), version: n, ...over });
const ids = (runs: readonly Run[]) => runs.map((r) => Number(r.runId.slice(-12)));

/** A platform that holds runs 1..total and serves them newest first, `size` at a time, cursor "from:<offset>". */
function platform(initialTotal: number, size = 50) {
  const state = { total: initialTotal, status: new Map<number, Run['status']>(), hidden: new Set<number>() };
  const calls: (string | undefined)[] = [];
  const all = () => {
    const out: Run[] = [];
    for (let n = state.total; n >= 1; n--) if (!state.hidden.has(n)) out.push(run(n, { status: state.status.get(n) ?? 'RUNNING' }));
    return out;
  };
  const fetchPage = vi.fn((cursor: string | undefined): Promise<Page<Run>> => {
    calls.push(cursor);
    const list = all();
    const from = cursor === undefined ? 0 : Number(cursor.replace('from:', ''));
    const items = list.slice(from, from + size);
    return Promise.resolve({ items, nextCursor: from + size < list.length ? `from:${from + size}` : null });
  });
  return { state, calls, fetchPage };
}

function store(fetchPage: RunListDeps['fetchPage'], over: Partial<RunListDeps> = {}) {
  const loaded = vi.fn();
  const s = new RunListStore({ fetchPage, onLoaded: loaded, ...over });
  s.start();
  return { s, loaded };
}

const settle = () => vi.advanceTimersByTimeAsync(0);

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('mergeNewest', () => {
  const held = new Map([5, 4, 3, 2, 1].map((n) => [run(n).runId, run(n)]));
  it('replaces what the page covers and keeps the older runs', () => {
    const merged = mergeNewest(held, { items: [run(7), run(6), run(5, { status: 'FAILED' })], nextCursor: 'x' });
    expect(ids([...merged.values()].sort(newerFirst))).toEqual([7, 6, 5, 4, 3, 2, 1]);
    expect(merged.get(run(5).runId)?.status).toBe('FAILED');
  });
  it('drops a run in the time the page covers that the page no longer has (it left a filter)', () => {
    const merged = mergeNewest(held, { items: [run(5), run(3)], nextCursor: 'x' });
    expect(ids([...merged.values()].sort(newerFirst))).toEqual([5, 3, 2, 1]);
  });
  it('is the whole list when the page has no next cursor', () => {
    const merged = mergeNewest(held, { items: [run(5)], nextCursor: null });
    expect(ids([...merged.values()])).toEqual([5]);
  });
});

describe('RunListStore', () => {
  it('reads the newest page, newest first, and says there is more when the platform does', async () => {
    const p = platform(120);
    const { s, loaded } = store(p.fetchPage);
    expect(s.getSnapshot()).toMatchObject({ phase: 'loading', items: [] });
    await settle();
    const snap = s.getSnapshot();
    expect(snap.phase).toBe('ready');
    expect(ids(snap.items)).toEqual(Array.from({ length: 50 }, (_, i) => 120 - i));
    expect(snap.hasMore).toBe(true);
    expect(loaded).toHaveBeenCalledWith({ kind: 'first', count: 50, total: 50, hasMore: true });
    s.dispose();
  });

  it('loads older pages with the cursor, until there are none', async () => {
    const p = platform(120);
    const { s, loaded } = store(p.fetchPage);
    await settle();
    await s.loadMore();
    await s.loadMore();
    expect(s.getSnapshot().items).toHaveLength(120);
    expect(s.getSnapshot().hasMore).toBe(false);
    expect(p.calls).toEqual([undefined, 'from:50', 'from:100']);
    expect(loaded).toHaveBeenLastCalledWith({ kind: 'more', count: 20, total: 120, hasMore: false });
    await s.loadMore();
    expect(p.calls).toHaveLength(3);
    s.dispose();
  });

  it('keeps what it has and says so when an older page fails, and tries that page again', async () => {
    const p = platform(120);
    const { s } = store(p.fetchPage);
    await settle();
    const failure = new ApiError({ code: 'UNAVAILABLE', status: 503, message: 'down', retryable: true });
    p.fetchPage.mockRejectedValueOnce(failure);
    await s.loadMore();
    expect(s.getSnapshot()).toMatchObject({ moreError: failure, loadingMore: false });
    expect(s.getSnapshot().items).toHaveLength(50);
    await s.loadMore();
    expect(s.getSnapshot().items).toHaveLength(100);
    expect(s.getSnapshot().moreError).toBeUndefined();
    s.dispose();
  });

  it('reads only the newest page every five seconds, updating runs in place and adding new ones on top', async () => {
    const p = platform(120);
    const { s, loaded } = store(p.fetchPage);
    await settle();
    await s.loadMore();
    p.calls.length = 0;
    p.state.status.set(119, 'FAILED');
    p.state.total = 123;
    await vi.advanceTimersByTimeAsync(5000);
    expect(p.calls).toEqual([undefined]);
    const items = s.getSnapshot().items;
    expect(ids(items).slice(0, 5)).toEqual([123, 122, 121, 120, 119]);
    expect(items.find((r) => r.runId === run(119).runId)?.status).toBe('FAILED');
    // The older pages stay (runs 21 to 120 were loaded), and nothing between them and the new newest page was lost.
    expect(ids(items)).toEqual(Array.from({ length: 103 }, (_, i) => 123 - i));
    // An automatic refresh is not announced.
    expect(loaded).toHaveBeenCalledTimes(2);
    s.dispose();
  });

  it('goes on from where the older pages stopped after the newest page moved on', async () => {
    const p = platform(120);
    const { s } = store(p.fetchPage);
    await settle();
    await s.loadMore();
    p.state.total = 130;
    await vi.advanceTimersByTimeAsync(5000);
    await s.loadMore();
    expect(ids(s.getSnapshot().items)).toEqual(Array.from({ length: 130 }, (_, i) => 130 - i));
    s.dispose();
  });

  it('adopts the cursor of the newest page when nothing older is loaded', async () => {
    const p = platform(60);
    const { s } = store(p.fetchPage);
    await settle();
    p.state.total = 70;
    await vi.advanceTimersByTimeAsync(5000);
    await s.loadMore();
    expect(ids(s.getSnapshot().items)).toEqual(Array.from({ length: 70 }, (_, i) => 70 - i));
    s.dispose();
  });

  it('drops a run that stopped matching the filter', async () => {
    const p = platform(10);
    const { s } = store(p.fetchPage);
    await settle();
    expect(s.getSnapshot().items).toHaveLength(10);
    p.state.hidden.add(7);
    await vi.advanceTimersByTimeAsync(5000);
    expect(ids(s.getSnapshot().items)).not.toContain(7);
    expect(s.getSnapshot().items).toHaveLength(9);
    s.dispose();
  });

  it('asks for nothing while the tab is hidden, and at once when it comes back', async () => {
    const p = platform(5);
    let visible = true;
    const { s } = store(p.fetchPage, { visible: () => visible });
    await settle();
    expect(p.calls).toHaveLength(1);
    visible = false;
    await vi.advanceTimersByTimeAsync(60_000);
    expect(p.calls).toHaveLength(1);
    visible = true;
    document.dispatchEvent(new Event('visibilitychange'));
    await settle();
    expect(p.calls).toHaveLength(2);
    // And it goes on every five seconds.
    await vi.advanceTimersByTimeAsync(5000);
    expect(p.calls).toHaveLength(3);
    s.dispose();
  });

  it('does not ask again just for becoming visible if it has asked a moment ago', async () => {
    const p = platform(5);
    const { s } = store(p.fetchPage);
    await settle();
    document.dispatchEvent(new Event('visibilitychange'));
    await settle();
    expect(p.calls).toHaveLength(1);
    s.dispose();
  });

  it('starts over when the person asks: older pages are forgotten, and the list stays on show until the answer', async () => {
    const p = platform(120);
    const { s, loaded } = store(p.fetchPage);
    await settle();
    await s.loadMore();
    expect(s.getSnapshot().items).toHaveLength(100);
    let release: ((page: Page<Run>) => void) | undefined;
    p.fetchPage.mockReturnValueOnce(new Promise((resolve) => (release = resolve)));
    const refreshing = s.refresh();
    expect(s.getSnapshot()).toMatchObject({ refreshing: true });
    expect(s.getSnapshot().items).toHaveLength(100);
    release?.({ items: [run(120), run(119)], nextCursor: 'from:2' });
    await refreshing;
    expect(ids(s.getSnapshot().items)).toEqual([120, 119]);
    expect(s.getSnapshot()).toMatchObject({ refreshing: false, hasMore: true });
    expect(loaded).toHaveBeenLastCalledWith({ kind: 'manual', count: 2, total: 2, hasMore: true });
    s.dispose();
  });

  it('shows the failure of the first read, and recovers when the person tries again', async () => {
    const p = platform(3);
    const failure = new ApiError({ code: 'FORBIDDEN', status: 403, message: 'no' });
    p.fetchPage.mockRejectedValueOnce(failure);
    const { s } = store(p.fetchPage);
    await settle();
    expect(s.getSnapshot()).toMatchObject({ phase: 'error', error: failure });
    await s.refresh();
    expect(s.getSnapshot()).toMatchObject({ phase: 'ready', error: undefined });
    expect(s.getSnapshot().items).toHaveLength(3);
    s.dispose();
  });

  it('keeps the list when an automatic refresh fails, says so, and tries again at the next tick', async () => {
    const p = platform(3);
    const { s } = store(p.fetchPage);
    await settle();
    const failure = new ApiError({ code: 'UNAVAILABLE', status: 503, message: 'down', retryable: true });
    p.fetchPage.mockRejectedValueOnce(failure);
    await vi.advanceTimersByTimeAsync(5000);
    expect(s.getSnapshot()).toMatchObject({ phase: 'ready', refreshError: failure });
    expect(s.getSnapshot().items).toHaveLength(3);
    await vi.advanceTimersByTimeAsync(5000);
    expect(s.getSnapshot().refreshError).toBeUndefined();
    s.dispose();
  });

  it('never has two reads in flight, and does a manual refresh asked for meanwhile right after', async () => {
    const p = platform(3);
    let release: ((page: Page<Run>) => void) | undefined;
    p.fetchPage.mockReturnValueOnce(new Promise((resolve) => (release = resolve)));
    const { s } = store(p.fetchPage);
    void s.refresh();
    void s.refresh();
    expect(p.fetchPage).toHaveBeenCalledTimes(1);
    release?.({ items: [run(3)], nextCursor: null });
    await settle();
    expect(p.fetchPage).toHaveBeenCalledTimes(2);
    s.dispose();
  });

  it('holds at most the limit it was given, and stops offering more', async () => {
    const p = platform(300, 100);
    const { s } = store(p.fetchPage, { maxItems: 150 });
    await settle();
    await s.loadMore();
    expect(s.getSnapshot().items).toHaveLength(150);
    expect(s.getSnapshot()).toMatchObject({ capped: true, hasMore: false });
    expect(ids(s.getSnapshot().items)[0]).toBe(300);
    s.dispose();
  });

  it('stops everything when disposed', async () => {
    const p = platform(3);
    const { s } = store(p.fetchPage);
    await settle();
    s.dispose();
    await vi.advanceTimersByTimeAsync(60_000);
    document.dispatchEvent(new Event('visibilitychange'));
    await settle();
    expect(p.calls).toHaveLength(1);
  });
});
