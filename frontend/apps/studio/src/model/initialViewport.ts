import type { Point } from './layout';

const INITIAL_MARGIN = 16;

export interface InitialViewport {
  x: number;
  y: number;
  zoom: number;
}

/**
 * Open a graph with its first card at a readable 1:1 scale. The rest of the graph remains
 * reachable by panning, while the explicit fit control can still show the complete overview.
 */
export function readableInitialViewport(first: Point | undefined): InitialViewport {
  if (first === undefined || !Number.isFinite(first.x) || !Number.isFinite(first.y)) return { x: 0, y: 0, zoom: 1 };
  return { x: INITIAL_MARGIN - first.x, y: INITIAL_MARGIN - first.y, zoom: 1 };
}
