import { describe, expect, it } from 'vitest';
import { initialGraphViewportOffset } from './GraphView';

describe('initialGraphViewportOffset', () => {
  it('anchors the first row at the visible top of a canvas that extends below the page fold', () => {
    expect(
      initialGraphViewportOffset({
        canvasTop: 560,
        canvasBottom: 1_112,
        viewportHeight: 812,
        graphTop: 230,
        firstRowHeight: 62,
      }),
    ).toBe(-214);
  });

  it('leaves an initial graph alone when its first row is already anchored', () => {
    expect(
      initialGraphViewportOffset({
        canvasTop: 420,
        canvasBottom: 972,
        viewportHeight: 812,
        graphTop: 16,
        firstRowHeight: 62,
      }),
    ).toBe(0);
  });

  it('uses the same canvas-relative anchor after content above the graph grows', () => {
    const graph = { viewportHeight: 812, graphTop: 230, firstRowHeight: 62 };
    expect(initialGraphViewportOffset({ ...graph, canvasTop: 420, canvasBottom: 972 })).toBe(-214);
    expect(initialGraphViewportOffset({ ...graph, canvasTop: 560, canvasBottom: 1_112 })).toBe(-214);
  });

  it('anchors below the window top when the canvas starts above it', () => {
    expect(
      initialGraphViewportOffset({
        canvasTop: -100,
        canvasBottom: 452,
        viewportHeight: 812,
        graphTop: 276,
        firstRowHeight: 62,
      }),
    ).toBe(-160);
  });

  it('does not move the camera when the visible canvas slice cannot contain one row', () => {
    expect(
      initialGraphViewportOffset({ canvasTop: 770, canvasBottom: 1_322, viewportHeight: 812, graphTop: 20, firstRowHeight: 62 }),
    ).toBe(0);
  });
});
