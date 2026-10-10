import { createContext } from 'react';

/** How long the editor waits before it checks and before it saves; a test shortens them, nothing else does. */
export interface EditorTimings {
  /** Quiet time before the document is validated (after the first check on load). */
  validateMs: number;
  /** Quiet time before the draft is saved. */
  autosaveMs: number;
  /** How long a line such as "Not changed. That link would make a cycle" stays on the page before it is cleared (default 10 s). */
  noticeMs?: number;
}

export const DEFAULT_TIMINGS: EditorTimings = { validateMs: 600, autosaveMs: 1_500, noticeMs: 10_000 };

export const TimingsContext = createContext<EditorTimings>(DEFAULT_TIMINGS);
