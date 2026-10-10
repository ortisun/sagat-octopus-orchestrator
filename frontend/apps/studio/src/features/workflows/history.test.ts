import { describe, expect, it } from 'vitest';
import { COALESCE_MS, HISTORY_LIMIT, canRedo, canUndo, commit, redo, startHistory, undo } from './history';

describe('history', () => {
  it('goes back and forth through the documents', () => {
    let h = startHistory('a');
    h = commit(h, 'b');
    h = commit(h, 'c');
    expect(h.present).toBe('c');
    h = undo(h);
    expect(h.present).toBe('b');
    h = undo(h);
    expect(h.present).toBe('a');
    expect(canUndo(h)).toBe(false);
    expect(undo(h)).toBe(h);
    h = redo(h);
    h = redo(h);
    expect(h.present).toBe('c');
    expect(canRedo(h)).toBe(false);
    expect(redo(h)).toBe(h);
  });

  it('forgets what could be redone when something new is done', () => {
    let h = commit(commit(startHistory(1), 2), 3);
    h = undo(h);
    expect(canRedo(h)).toBe(true);
    h = commit(h, 9);
    expect(canRedo(h)).toBe(false);
    expect(h.past).toEqual([1, 2]);
  });

  it('ignores a change that changes nothing', () => {
    const h = startHistory({ x: 1 });
    expect(commit(h, h.present)).toBe(h);
  });

  it('keeps at most 100 earlier documents, dropping the oldest', () => {
    let h = startHistory(0);
    for (let i = 1; i <= HISTORY_LIMIT + 25; i++) h = commit(h, i, { now: i * 10_000 });
    expect(h.past).toHaveLength(HISTORY_LIMIT);
    expect(h.past[0]).toBe(25);
    expect(h.present).toBe(HISTORY_LIMIT + 25);
    // Undoing all the way stops at the oldest document it kept.
    while (canUndo(h)) h = undo(h);
    expect(h.present).toBe(25);
  });

  it('folds typing in one field into one step, and not typing in two fields', () => {
    let h = startHistory('');
    h = commit(h, 'a', { key: 'name', now: 1_000 });
    h = commit(h, 'ab', { key: 'name', now: 1_200 });
    h = commit(h, 'abc', { key: 'name', now: 1_400 });
    expect(h.past).toEqual(['']);
    h = commit(h, 'abcd', { key: 'other', now: 1_500 });
    expect(h.past).toEqual(['', 'abc']);
    expect(undo(h).present).toBe('abc');
    expect(undo(undo(h)).present).toBe('');
  });

  it('does not fold two bursts of typing that are a while apart', () => {
    let h = startHistory('');
    h = commit(h, 'a', { key: 'name', now: 1_000 });
    h = commit(h, 'ab', { key: 'name', now: 1_000 + COALESCE_MS + 1 });
    expect(h.past).toEqual(['', 'a']);
  });

  it('does not fold into an edit that was undone', () => {
    let h = startHistory('');
    h = commit(h, 'a', { key: 'name', now: 1_000 });
    h = undo(h);
    h = commit(h, 'b', { key: 'name', now: 1_100 });
    expect(h.past).toEqual(['']);
    expect(h.present).toBe('b');
  });
});
