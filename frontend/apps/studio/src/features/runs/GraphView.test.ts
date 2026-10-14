import { describe, expect, it } from 'vitest';
import { initialRunViewport } from './GraphView';

describe('initialRunViewport', () => {
  it('opens on the root run step at a readable scale even when ids sort differently', () => {
    const positions = new Map([
      ['validate', { x: 420, y: 230 }],
      ['charge', { x: 2_400, y: 800 }],
    ]);

    expect(initialRunViewport(['charge', 'validate'], positions, [{ from: 'validate', to: 'charge' }], new Map())).toEqual({ x: -404, y: -214, zoom: 1 });
  });

  it('skips missing projected steps without fitting the remaining graph', () => {
    expect(initialRunViewport(['missing', 'charge'], new Map([['charge', { x: 600, y: 16 }]]), [], new Map())).toEqual({ x: -584, y: 0, zoom: 1 });
  });

  it('prefers the forward root when a compensation root sorts first', () => {
    const positions = new Map([
      ['submit~c', { x: 16, y: 16 }],
      ['validate', { x: 420, y: 230 }],
    ]);
    const nodes = new Map([
      ['submit~c', { kind: 'COMPENSATION' as const }],
      ['validate', {}],
    ]);

    expect(initialRunViewport(['submit~c', 'validate'], positions, [], nodes)).toEqual({ x: -404, y: -214, zoom: 1 });
  });
});
