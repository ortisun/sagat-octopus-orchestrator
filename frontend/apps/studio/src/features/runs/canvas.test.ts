import { describe, expect, it } from 'vitest';
import type { RuntimeEdge, RuntimeNode } from '../../api/types';
import { layoutGraph } from '../../model/layout';
import { applyChanges, modelFromGraph } from '../../model/runGraph';
import { CARD_SIZE, CHIP_SIZE, FULL_CARDS_UP_TO, TABLE_FIRST_ABOVE, flowEdges, flowNodes, layoutNodes, neighbours, viewPolicy } from './canvas';
import { idOrder } from './table';

const RUN = '019b5d6e-0000-7000-8000-000000000001';
const node = (id: string, status: RuntimeNode['status'], over: Partial<RuntimeNode> = {}): RuntimeNode => ({ id, type: 'activity', status, attempt: 1, ...over });
const NO_FLASH = { gen: 0, ids: new Set<string>() };

function setup(nodes: RuntimeNode[], edges: RuntimeEdge[]) {
  const model = modelFromGraph({ runId: RUN, projectionSeq: 10, nodes, edges });
  const order = idOrder(model.nodes);
  const positions = layoutGraph(layoutNodes(order, 'full'), edges, 'LR');
  return { model, order, positions };
}

describe('viewPolicy (D147)', () => {
  // The numbers are D147's and are written out here on purpose: changing a threshold must change this test.
  it('is interactive with full cards up to 1 000 nodes', () => {
    expect(viewPolicy(0)).toMatchObject({ detail: 'full', start: 'graph' });
    expect(viewPolicy(1)).toEqual({ detail: 'full', start: 'graph', minimap: true, edgeLabels: true });
    expect(viewPolicy(1000)).toMatchObject({ detail: 'full', start: 'graph', minimap: true });
    expect([FULL_CARDS_UP_TO, TABLE_FIRST_ABOVE]).toEqual([1000, 10_000]);
  });
  it('draws simplified chips above 1 000, still opening on the canvas, without minimap or edge labels', () => {
    expect(viewPolicy(1001)).toEqual({ detail: 'simple', start: 'graph', minimap: false, edgeLabels: false });
    expect(viewPolicy(10_000)).toMatchObject({ detail: 'simple', start: 'graph' });
  });
  it('opens in the table view above 10 000, 50 000 included', () => {
    expect(viewPolicy(10_001)).toMatchObject({ detail: 'simple', start: 'table' });
    expect(viewPolicy(50_000)).toMatchObject({ detail: 'simple', start: 'table' });
  });
});

describe('flowNodes', () => {
  const nodes = [node('a', 'SUCCEEDED'), node('b', 'FAILED', { failureCode: 'HTTP_500', attempt: 3 }), node('c', 'NOT_STARTED')];
  const edges: RuntimeEdge[] = [
    { from: 'a', to: 'b', kind: 'next' },
    { from: 'b', to: 'c', kind: 'next' },
  ];

  it('draws one card per node with the size the layout reserved, named for assistive technology', () => {
    expect(CARD_SIZE).toEqual({ width: 320, height: 176 });
    const { model, order, positions } = setup(nodes, edges);
    const out = flowNodes({ nodes: model.nodes, order, positions, detail: 'full', selectedId: undefined, flash: NO_FLASH });
    expect(out.map((n) => n.id)).toEqual(['a', 'b', 'c']);
    expect(out.every((n) => n.type === 'run-card' && n.width === CARD_SIZE.width && n.height === CARD_SIZE.height)).toBe(true);
    expect(out[1]?.ariaLabel).toBe('b, Step, Failed, attempt 3, failure HTTP_500');
    expect(out.every((n) => n.draggable === false && n.connectable === false)).toBe(true);
    const placed = new Set(out.map((n) => `${n.position.x},${n.position.y}`));
    expect(placed.size).toBe(3);
  });

  it('draws chips in the simplified mode', () => {
    expect(CHIP_SIZE).toEqual({ width: 248, height: 88 });
    const { model, order } = setup(nodes, edges);
    const positions = layoutGraph(layoutNodes(order, 'simple'), edges, 'LR');
    const out = flowNodes({ nodes: model.nodes, order, positions, detail: 'simple', selectedId: undefined, flash: NO_FLASH });
    expect(out.every((n) => n.type === 'run-chip' && n.width === CHIP_SIZE.width)).toBe(true);
  });

  it('keeps the cards that did not change as the very same objects, and replaces only the ones the update touched', () => {
    const { model, order, positions } = setup(nodes, edges);
    const before = flowNodes({ nodes: model.nodes, order, positions, detail: 'full', selectedId: undefined, flash: NO_FLASH });
    const applied = applyChanges(model, {
      runId: RUN,
      status: 'RUNNING',
      fromSeq: 10,
      projectionSeq: 12,
      reset: false,
      events: [],
      nodes: [node('c', 'RUNNING')],
    });
    if (applied.kind !== 'applied') throw new Error('expected the change to apply');
    const after = flowNodes({ nodes: applied.model.nodes, order, positions, detail: 'full', selectedId: undefined, flash: { gen: 1, ids: new Set(['c']) } });
    expect(after[0]).toBe(before[0]);
    expect(after[1]).toBe(before[1]);
    expect(after[2]).not.toBe(before[2]);
    expect(after[2]?.data.node.status).toBe('RUNNING');
    expect(after[2]?.data.flash).toBe(1);
    expect(before[2]?.data.flash).toBe(0);
    expect(after.map((value) => value.position)).toEqual(before.map((value) => value.position));
  });

  it('marks the selected card, and only that one is replaced when the selection moves', () => {
    const { model, order, positions } = setup(nodes, edges);
    const one = flowNodes({ nodes: model.nodes, order, positions, detail: 'full', selectedId: 'a', flash: NO_FLASH });
    expect(one.map((n) => n.selected)).toEqual([true, false, false]);
    const two = flowNodes({ nodes: model.nodes, order, positions, detail: 'full', selectedId: 'b', flash: NO_FLASH });
    expect(two.map((n) => n.selected)).toEqual([false, true, false]);
    expect(two[2]).toBe(one[2]);
    expect(two[0]).not.toBe(one[0]);
  });

  it('restarts the highlight of a node the next update touches again', () => {
    const { model, order, positions } = setup(nodes, edges);
    const first = flowNodes({ nodes: model.nodes, order, positions, detail: 'full', selectedId: undefined, flash: { gen: 4, ids: new Set(['a']) } });
    const second = flowNodes({ nodes: model.nodes, order, positions, detail: 'full', selectedId: undefined, flash: { gen: 5, ids: new Set(['a']) } });
    expect(first[0]?.data.flash).toBe(4);
    expect(second[0]?.data.flash).toBe(5);
    expect(second[0]).not.toBe(first[0]);
  });

  it('leaves out a node the layout does not know instead of drawing it at the origin', () => {
    const { model, order } = setup(nodes, edges);
    const out = flowNodes({ nodes: model.nodes, order, positions: new Map([['a', { x: 0, y: 0 }]]), detail: 'full', selectedId: undefined, flash: NO_FLASH });
    expect(out.map((n) => n.id)).toEqual(['a']);
  });

  it('describes compensations and lookups for what they are', () => {
    const undo = node('reserve~c', 'WAITING', { kind: 'COMPENSATION', forward: 'reserve', waitingOn: 'operator' });
    const { model, order, positions } = setup([undo], []);
    const [card] = flowNodes({ nodes: model.nodes, order, positions, detail: 'full', selectedId: undefined, flash: NO_FLASH });
    expect(card?.ariaLabel).toBe('reserve~c, Undo of reserve, Waiting, waiting for a person');
  });
});

describe('flowEdges', () => {
  const nodes = [node('check', 'SUCCEEDED', { type: 'condition' }), node('pay', 'RUNNING'), node('skip', 'NOT_STARTED'), node('later', 'NOT_STARTED')];
  const edges: RuntimeEdge[] = [
    { from: 'check', to: 'pay', kind: 'true' },
    { from: 'check', to: 'skip', kind: 'false' },
    { from: 'pay', to: 'later', kind: 'next' },
  ];
  const map = new Map(nodes.map((n) => [n.id, n]));

  it('styles each edge by what happened to it and says the same in words', () => {
    const out = flowEdges(edges, map, true);
    expect(out.map((e) => e.className)).toEqual(['edge-taken edge-kind-true', 'edge-not-taken edge-kind-false', 'edge-pending edge-kind-next']);
    expect(out.map((e) => e.label)).toEqual(['If true', 'If false (omitted)', 'Next']);
    expect(out.map((e) => e.ariaLabel)).toEqual(['check to pay: If true, taken', 'check to skip: If false, omitted', 'pay to later: Next, not decided yet']);
    expect(out.every((edge) => edge.type === 'default')).toBe(true);
  });

  it('has ids that are unique even for two links of the same kind between the same nodes', () => {
    const twice: RuntimeEdge[] = [
      { from: 'a', to: 'b', kind: 'branch' },
      { from: 'a', to: 'b', kind: 'branch' },
    ];
    const ids = flowEdges(twice, new Map(), true).map((e) => e.id);
    expect(new Set(ids).size).toBe(2);
  });

  it('keeps an edge as the same object while its state does not change', () => {
    const first = flowEdges(edges, map, true);
    const moved = new Map(map).set('later', node('later', 'NOT_STARTED'));
    const second = flowEdges(edges, moved, true);
    expect(second[0]).toBe(first[0]);
    const finished = new Map(map).set('pay', node('pay', 'SUCCEEDED'));
    const third = flowEdges(edges, finished, true);
    expect(third[2]).not.toBe(first[2]);
    expect(third[2]?.className).toBe('edge-not-taken edge-kind-next');
  });

  it('leaves the labels out of big graphs but not the accessible names', () => {
    const out = flowEdges(edges, map, false);
    expect(out.every((e) => e.label === undefined)).toBe(true);
    expect(out[0]?.ariaLabel).toBe('check to pay: If true, taken');
    expect(out[0]?.type).toBe('straight');
  });
});

describe('neighbours', () => {
  it('lists what leads into a node and out of it, with the kind and whether the run went that way', () => {
    const nodes = new Map([node('a', 'SUCCEEDED'), node('b', 'RUNNING'), node('c', 'NOT_STARTED')].map((n) => [n.id, n]));
    const edges: RuntimeEdge[] = [
      { from: 'a', to: 'b', kind: 'next' },
      { from: 'b', to: 'c', kind: 'error' },
    ];
    expect(neighbours(edges, nodes, 'b')).toEqual({
      from: [{ id: 'a', kind: 'Next', state: 'taken' }],
      to: [{ id: 'c', kind: 'On error', state: 'pending' }],
    });
  });
});

describe('a graph of 50 000 steps through the canvas mapper', () => {
  it('is not drawn unless asked: the policy says table, and the mapper stays linear when it is asked', () => {
    const count = 50_000;
    const nodes = Array.from({ length: count }, (_, i) => node(`s${i}`, i % 3 === 0 ? 'SUCCEEDED' : 'NOT_STARTED'));
    const edges: RuntimeEdge[] = Array.from({ length: count - 1 }, (_, i) => ({ from: `s${i}`, to: `s${i + 1}`, kind: 'next' }));
    expect(viewPolicy(count).start).toBe('table');
    const model = modelFromGraph({ runId: RUN, projectionSeq: 1, nodes, edges });
    const order = idOrder(model.nodes);
    const t0 = performance.now();
    const positions = layoutGraph(layoutNodes(order, 'simple'), edges, 'LR');
    const out = flowNodes({ nodes: model.nodes, order, positions, detail: 'simple', selectedId: undefined, flash: NO_FLASH });
    const drawn = flowEdges(edges, model.nodes, false);
    const elapsed = performance.now() - t0;
    expect(out).toHaveLength(count);
    expect(drawn).toHaveLength(count - 1);
    expect(elapsed).toBeLessThan(5000);
  });
});
