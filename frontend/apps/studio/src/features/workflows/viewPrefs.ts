import type { Direction, Point } from '../../model/layout';

/**
 * What the author prefers to see, not part of the document: which view, which direction, and where
 * they dragged the cards. It lives in the browser's storage, which may be blocked, full or cleared,
 * so every access is guarded and the page works without it. Nothing secret is ever stored here.
 */
const VIEW_KEY = 'studio.workflows.view';
const DIRECTION_KEY = 'studio.workflows.direction';
const layoutKey = (workflowId: string): string => `studio.workflow.layout.${workflowId.slice(0, 64)}`;

const MAX_POSITIONS = 20_000;
const MAX_COORDINATE = 1_000_000;

function get(key: string): string | null {
  try {
    return globalThis.localStorage?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

function set(key: string, value: string): void {
  try {
    globalThis.localStorage?.setItem(key, value);
  } catch {
    // Not remembering is acceptable.
  }
}

function remove(key: string): void {
  try {
    globalThis.localStorage?.removeItem(key);
  } catch {
    // Same.
  }
}

export type ViewMode = 'canvas' | 'list';

export function readView(): ViewMode {
  return get(VIEW_KEY) === 'list' ? 'list' : 'canvas';
}

export function writeView(view: ViewMode): void {
  set(VIEW_KEY, view);
}

export function readDirection(): Direction {
  return get(DIRECTION_KEY) === 'TB' ? 'TB' : 'LR';
}

export function writeDirection(direction: Direction): void {
  set(DIRECTION_KEY, direction);
}

/** Keyed by card id. A Map, because an id such as `constructor` or `__proto__` is a valid node id. */
export type Positions = Map<string, Point>;

/** The positions saved for a workflow; anything in storage that is not a position is left out. */
export function readPositions(workflowId: string): Positions {
  const raw = get(layoutKey(workflowId));
  if (raw === null || raw.length > 4_000_000) return new Map();
  try {
    const parsed = JSON.parse(raw) as { v?: unknown; positions?: unknown };
    if (parsed.v !== 1 || typeof parsed.positions !== 'object' || parsed.positions === null || Array.isArray(parsed.positions)) return new Map();
    const out: Positions = new Map();
    let count = 0;
    for (const [id, value] of Object.entries(parsed.positions as Record<string, unknown>)) {
      if (count >= MAX_POSITIONS) break;
      if (id.length > 80 || typeof value !== 'object' || value === null) continue;
      const { x, y } = value as { x?: unknown; y?: unknown };
      if (typeof x !== 'number' || typeof y !== 'number' || !Number.isFinite(x) || !Number.isFinite(y)) continue;
      if (Math.abs(x) > MAX_COORDINATE || Math.abs(y) > MAX_COORDINATE) continue;
      out.set(id, { x: Math.round(x), y: Math.round(y) });
      count++;
    }
    return out;
  } catch {
    return new Map();
  }
}

export function writePositions(workflowId: string, positions: Positions): void {
  if (positions.size === 0) remove(layoutKey(workflowId));
  else set(layoutKey(workflowId), JSON.stringify({ v: 1, positions: Object.fromEntries([...positions].map(([id, at]) => [id, { x: Math.round(at.x), y: Math.round(at.y) }])) }));
}
