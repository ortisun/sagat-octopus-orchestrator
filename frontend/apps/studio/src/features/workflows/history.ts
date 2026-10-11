/** How many earlier documents undo can reach back to. */
export const HISTORY_LIMIT = 100;

/** Typing in one field is one step to undo, not one per key: edits with the same key this close are one. */
export const COALESCE_MS = 1_000;

export interface History<T> {
  past: readonly T[];
  present: T;
  future: readonly T[];
  /** What the last edit was about, to fold the next one into it. */
  lastKey: string | undefined;
  lastAt: number;
}

export function startHistory<T>(present: T): History<T> {
  return { past: [], present, future: [], lastKey: undefined, lastAt: 0 };
}

export function commit<T>(history: History<T>, next: T, options: { key?: string | undefined; now?: number } = {}): History<T> {
  if (Object.is(next, history.present)) return history;
  const now = options.now ?? Date.now();
  const key = options.key;
  if (key !== undefined && history.lastKey === key && now - history.lastAt < COALESCE_MS && history.past.length > 0) {
    return { ...history, present: next, future: [], lastAt: now };
  }
  const past = [...history.past, history.present];
  return { past: past.length > HISTORY_LIMIT ? past.slice(past.length - HISTORY_LIMIT) : past, present: next, future: [], lastKey: key, lastAt: now };
}

export function undo<T>(history: History<T>): History<T> {
  const previous = history.past[history.past.length - 1];
  if (previous === undefined) return history;
  return { past: history.past.slice(0, -1), present: previous, future: [history.present, ...history.future], lastKey: undefined, lastAt: 0 };
}

export function redo<T>(history: History<T>): History<T> {
  const [next, ...rest] = history.future;
  if (next === undefined) return history;
  return { past: [...history.past, history.present], present: next, future: rest, lastKey: undefined, lastAt: 0 };
}

export const canUndo = (history: History<unknown>): boolean => history.past.length > 0;
export const canRedo = (history: History<unknown>): boolean => history.future.length > 0;
