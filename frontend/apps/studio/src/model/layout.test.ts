import { describe, expect, it } from 'vitest';
import { LAYERED_ABOVE, layeredLayout, layoutGraph } from './layout';
import type { LayoutEdge, LayoutNode } from './layout';

const box = (id: string): LayoutNode => ({ id, width: 160, height: 56 });
const chain = (n: number): { nodes: LayoutNode[]; edges: LayoutEdge[] } => ({
  nodes: Array.from({ length: n }, (_, i) => box(`n${i}`)),
  edges: Array.from({ length: n - 1 }, (_, i) => ({ from: `n${i}`, to: `n${i + 1}` })),
});

describe('layoutGraph', () => {
  it('puts a chain in order, one rank after the other, without overlapping', () => {
    const { nodes, edges } = chain(5);
    const at = layoutGraph(nodes, edges, 'LR');
    const xs = nodes.map((n) => at.get(n.id)?.x ?? NaN);
    expect(xs).toEqual([...xs].sort((a, b) => a - b));
    expect(new Set(xs).size).toBe(5);
    for (let i = 1; i < xs.length; i++) expect(xs[i]! - xs[i - 1]!).toBeGreaterThanOrEqual(160);
    const down = layoutGraph(nodes, edges, 'TB');
    const ys = nodes.map((n) => down.get(n.id)?.y ?? NaN);
    expect(ys).toEqual([...ys].sort((a, b) => a - b));
  });

  it('separates the branches of a fork', () => {
    const nodes = ['a', 'b', 'c', 'd'].map(box);
    const edges = [
      { from: 'a', to: 'b' },
      { from: 'a', to: 'c' },
      { from: 'b', to: 'd' },
      { from: 'c', to: 'd' },
    ];
    const at = layoutGraph(nodes, edges, 'LR');
    expect(at.get('b')?.y).not.toBe(at.get('c')?.y);
    expect(at.get('b')?.x).toBe(at.get('c')?.x);
    expect(at.get('d')?.x).toBeGreaterThan(at.get('b')?.x ?? 0);
  });

  it('is the same every time for the same graph, so a picture does not jump on a refresh', () => {
    const { nodes, edges } = chain(8);
    expect([...layoutGraph(nodes, edges, 'LR')]).toEqual([...layoutGraph(nodes, edges, 'LR')]);
  });

  it('places nodes that nothing links, and ignores links to nodes that are not there', () => {
    const at = layoutGraph([box('a'), box('b')], [{ from: 'a', to: 'ghost' }], 'LR');
    expect(at.size).toBe(2);
    expect(layoutGraph([], [], 'LR').size).toBe(0);
  });
});

describe('layeredLayout (for graphs too big for the careful layout)', () => {
  it('ranks by the longest path and survives a cycle', () => {
    const at = layeredLayout(['a', 'b', 'c', 'd'].map(box), [
      { from: 'a', to: 'b' },
      { from: 'b', to: 'c' },
      { from: 'a', to: 'c' },
      { from: 'c', to: 'd' },
      { from: 'd', to: 'b' }, // a cycle that should not exist
    ], 'LR');
    expect(at.get('c')!.x).toBeGreaterThan(at.get('b')!.x);
    expect(at.size).toBe(4);
  });

  it('lays out ten thousand nodes quickly', () => {
    const nodes = Array.from({ length: 10_000 }, (_, i) => box(`n${i}`));
    const edges: LayoutEdge[] = nodes.slice(1).map((n, i) => ({ from: `n${Math.floor(i / 2)}`, to: n.id }));
    const started = performance.now();
    const at = layoutGraph(nodes, edges, 'LR');
    expect(performance.now() - started).toBeLessThan(2000);
    expect(at.size).toBe(10_000);
    expect(nodes.length).toBeGreaterThan(LAYERED_ABOVE);
  });
});
