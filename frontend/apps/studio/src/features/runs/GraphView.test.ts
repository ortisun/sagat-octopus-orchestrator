import { describe, expect, it } from 'vitest';
import { initialStepViewportOffset } from './GraphView';

describe('initialStepViewportOffset', () => {
  it('moves a clipped initial step to the top of the visible part of the canvas', () => {
    expect(
      initialStepViewportOffset({
        canvasTop: 560,
        canvasBottom: 1_112,
        viewportHeight: 812,
        stepTop: 790,
        stepBottom: 852,
      }),
    ).toBe(-214);
  });

  it('leaves an already complete initial step where fit view put it', () => {
    expect(
      initialStepViewportOffset({
        canvasTop: 420,
        canvasBottom: 972,
        viewportHeight: 812,
        stepTop: 650,
        stepBottom: 712,
      }),
    ).toBe(0);
  });

  it('does not move the camera when the visible slice cannot contain a complete step', () => {
    expect(
      initialStepViewportOffset({
        canvasTop: 770,
        canvasBottom: 1_322,
        viewportHeight: 812,
        stepTop: 790,
        stepBottom: 852,
      }),
    ).toBe(0);
  });
});
