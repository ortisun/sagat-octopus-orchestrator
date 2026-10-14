import { describe, expect, it } from 'vitest';
import { readableInitialViewport } from './initialViewport';

describe('readableInitialViewport', () => {
  it('opens a wide graph at reading size and anchors its first card', () => {
    expect(readableInitialViewport({ x: 420, y: 230 })).toEqual({ x: -404, y: -214, zoom: 1 });
  });

  it('does not shrink a large graph to make its full bounds visible', () => {
    const viewport = readableInitialViewport({ x: 16, y: 16 });

    expect(viewport.zoom).toBe(1);
    expect(viewport.zoom).toBeGreaterThanOrEqual(0.85);
  });

  it('uses a safe readable viewport when there is no first card', () => {
    expect(readableInitialViewport(undefined)).toEqual({ x: 0, y: 0, zoom: 1 });
  });
});
