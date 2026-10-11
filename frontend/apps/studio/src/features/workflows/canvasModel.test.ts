import { describe, expect, it } from 'vitest';
import { addNode, connect } from '../../model/edit';
import { emptyDocument } from '../../model/ir';
import type { IrDocument } from '../../model/ir';
import type { Point } from '../../model/layout';
import { NODE_KINDS } from '../../model/registry';
import {
  CARD_SIZE,
  CanvasBuilder,
  FULL_UP_TO,
  LIST_ABOVE,
  buildCanvas,
  canvasMode,
  isTriggerCard,
  keyField,
  layoutInputs,
  neighbour,
  reuseCards,
  triggerCardId,
} from './canvasModel';
import type { CanvasOptions } from './canvasModel';

const none = new Map<string, number>();

function options(over: Partial<CanvasOptions> = {}): CanvasOptions {
  return { direction: 'LR', mode: 'full', auto: new Map(), manual: new Map(), selectedId: undefined, focusId: undefined, problems: none, advice: none, ...over };
}

function flow(): IrDocument {
  let doc = emptyDocument('order-flow');
  doc = addNode(doc, 'condition', 'check');
  doc = addNode(doc, 'activity', 'pay');
  doc = addNode(doc, 'parallel', 'both');
  doc = addNode(doc, 'activity', 'a');
  doc = addNode(doc, 'activity', 'b');
  doc = addNode(doc, 'end', 'done');
  doc = connect(doc, 'check', 'ifTrue', 'pay');
  doc = connect(doc, 'check', 'ifFalse', 'done');
  doc = connect(doc, 'pay', 'next', 'both');
  doc = connect(doc, 'pay', 'onError', 'done');
  doc = connect(doc, 'both', 'branches', 'a');
  doc = connect(doc, 'both', 'branches', 'b');
  doc = connect(doc, 'both', 'next', 'done');
  return doc;
}

describe('canvasMode (D147)', () => {
  it('draws full cards up to a thousand nodes, simple cards above, and opens the list above ten thousand', () => {
    expect(canvasMode(0)).toBe('full');
    expect(canvasMode(FULL_UP_TO)).toBe('full');
    expect(canvasMode(FULL_UP_TO + 1)).toBe('simplified');
    expect(canvasMode(LIST_ABOVE)).toBe('simplified');
    expect(canvasMode(LIST_ABOVE + 1)).toBe('list');
  });
});

describe('buildCanvas', () => {
  it('draws one card per node, with a handle for every way out the kind has, labelled', () => {
    for (const kind of NODE_KINDS) {
      const doc: IrDocument = { ...emptyDocument('x-flow'), entrypoint: 'n', nodes: [kind.make('n')], triggers: [] };
      const [card] = buildCanvas(doc, options()).nodes;
      expect(card?.id).toBe('n');
      expect(card?.data.typeLabel).toBe(kind.label);
      expect(card?.data.handles).toEqual(kind.handles.map((h) => ({ id: h.field, label: h.label })));
      expect(card?.data.unsupported).toBe(!kind.runs);
      expect(card?.width).toBe(CARD_SIZE.full.width);
    }
  });

  it('gives cards of different categories different shapes, so a kind is never told by colour alone', () => {
    const doc = flow();
    const shapes = new Map(buildCanvas(doc, options()).nodes.map((n) => [n.data.category, n.data.shape]));
    expect(new Set(shapes.values()).size).toBe(shapes.size);
    expect(shapes.size).toBeGreaterThanOrEqual(4);
  });

  it('draws a start card for each trigger and a link from it to the first step', () => {
    const doc: IrDocument = {
      ...flow(),
      triggers: [
        { kind: 'manual' },
        { kind: 'event', eventType: 'order.created' },
        { kind: 'cron', cron: '0 9 * * *' },
        { kind: 'webhook', endpoint: 'shop' },
        { kind: 'message', connector: 'orders' },
      ],
    };
    const model = buildCanvas(doc, options());
    const starts = model.nodes.filter((n) => n.data.role === 'trigger');
    expect(starts.map((n) => n.id)).toEqual([0, 1, 2, 3, 4].map(triggerCardId));
    expect(starts.map((n) => n.data.title)).toEqual(['Manual start', 'Event', 'Schedule', 'Webhook', 'Message']);
    expect(starts[1]?.data.detail).toBe('order.created');
    expect(starts.every((n) => n.data.typeLabel === 'Start')).toBe(true);
    const fromStarts = model.edges.filter((e) => isTriggerCard(e.source));
    expect(fromStarts).toHaveLength(5);
    expect(fromStarts.every((e) => e.target === 'check' && e.label === 'First step')).toBe(true);
  });

  it('draws a link for every link of the document, from the right handle, labelled when it is not a plain next', () => {
    const model = buildCanvas(flow(), options());
    const find = (source: string, handle: string, target: string) => model.edges.find((e) => e.source === source && e.sourceHandle === handle && e.target === target);
    expect(find('check', 'ifTrue', 'pay')?.label).toBe('If true');
    expect(find('check', 'ifFalse', 'done')?.label).toBe('If false');
    expect(find('pay', 'next', 'both')?.label).toBeUndefined();
    expect(find('pay', 'onError', 'done')?.label).toBe('On error');
    expect(find('both', 'branches', 'a')?.label).toBe('Branch 1');
    expect(find('both', 'branches', 'b')?.label).toBe('Branch 2');
    expect(find('both', 'next', 'done')?.label).toBeUndefined();
    expect(new Set(model.edges.map((e) => e.id)).size).toBe(model.edges.length);
    expect(model.edges.every((e) => e.targetHandle === 'in')).toBe(true);
  });

  it('tells kinds of link apart by dash pattern as well as by label', () => {
    const model = buildCanvas(flow(), options());
    const dash = (kind: string) => model.edges.find((e) => e.data?.kind === kind)?.style?.strokeDasharray;
    const patterns = ['error', 'false', 'branch'].map(dash);
    expect(patterns.every((p) => typeof p === 'string')).toBe(true);
    expect(new Set(patterns).size).toBe(3);
    expect(dash('next')).toBeUndefined();
    expect(dash('true')).toBeUndefined();
  });

  it('does not draw a link to a node that is not there, and reports it', () => {
    const doc: IrDocument = { ...flow(), nodes: flow().nodes.map((n) => (n.id === 'pay' ? { ...n, next: 'ghost' } : n)) };
    const model = buildCanvas(doc, options());
    expect(model.edges.some((e) => e.target === 'ghost')).toBe(false);
    expect(model.dangling).toEqual([{ from: 'pay', field: 'next', to: 'ghost' }]);
  });

  it('merges positions: the layout, with what the user dragged on top', () => {
    const auto = new Map<string, Point>([
      ['check', { x: 0, y: 0 }],
      ['pay', { x: 100, y: 0 }],
    ]);
    const manual = new Map<string, Point>([['pay', { x: 7, y: 9 }]]);
    const model = buildCanvas(flow(), options({ auto, manual }));
    const at = (id: string) => model.nodes.find((n) => n.id === id)?.position;
    expect(at('check')).toEqual({ x: 0, y: 0 });
    expect(at('pay')).toEqual({ x: 7, y: 9 });
    // A node nobody placed is at the origin, not missing.
    expect(at('done')).toEqual({ x: 0, y: 0 });
  });

  it('is not fooled by a node called like a property of every object', () => {
    let doc = addNode(emptyDocument('x-flow'), 'end', 'constructor');
    doc = addNode(doc, 'end', 'toString');
    doc = addNode(doc, 'end', 'hasOwnProperty');
    const model = buildCanvas(doc, options());
    for (const card of model.nodes.filter((n) => n.data.role === 'node')) expect(card.position).toEqual({ x: 0, y: 0 });
    const placed = buildCanvas(doc, options({ manual: new Map([['constructor', { x: 5, y: 6 }]]) }));
    expect(placed.nodes.find((n) => n.id === 'constructor')?.position).toEqual({ x: 5, y: 6 });
    expect(placed.nodes.find((n) => n.id === 'toString')?.position).toEqual({ x: 0, y: 0 });
  });

  it('shows badges for problems and advice on the nodes they are about, and flags the first step', () => {
    const model = buildCanvas(flow(), options({ problems: new Map([['pay', 2]]), advice: new Map([['pay', 1]]) }));
    const pay = model.nodes.find((n) => n.id === 'pay');
    expect(pay?.data.problems).toBe(2);
    expect(pay?.data.advice).toBe(1);
    expect(pay?.ariaLabel).toContain('2 problems');
    expect(pay?.ariaLabel).toContain('1 piece of advice');
    expect(model.nodes.find((n) => n.id === 'check')?.data.first).toBe(true);
    expect(model.nodes.find((n) => n.id === 'pay')?.data.first).toBe(false);
  });

  it('draws simplified cards for a big graph: smaller, straight links, no link labels', () => {
    const model = buildCanvas(flow(), options({ mode: 'simplified' }));
    expect(model.nodes.filter((n) => n.data.role === 'node').every((n) => n.data.simplified && n.width === CARD_SIZE.simplified.width)).toBe(true);
    expect(model.edges.every((e) => e.type === 'straight' && e.label === undefined)).toBe(true);
  });

  it('puts exactly one card in the tab order (roving tabindex) and marks the selected one', () => {
    const model = buildCanvas(flow(), options({ focusId: 'pay', selectedId: 'pay' }));
    expect(model.nodes.filter((n) => n.domAttributes?.tabIndex === 0).map((n) => n.id)).toEqual(['pay']);
    expect(model.nodes.filter((n) => n.selected).map((n) => n.id)).toEqual(['pay']);
  });

  it('survives a kind it does not know, with the ordinary ways out', () => {
    const doc: IrDocument = {
      ...emptyDocument('x-flow'),
      entrypoint: 'x',
      nodes: [
        { id: 'x', type: 'teleport', next: 'y' },
        { id: 'y', type: 'end' },
      ],
    };
    const model = buildCanvas(doc, options());
    expect(model.nodes[1]?.data.unknown).toBe(true);
    expect(model.nodes[1]?.data.handles.map((h) => h.id)).toEqual(['next', 'onError']);
    expect(model.edges.some((e) => e.source === 'x' && e.target === 'y')).toBe(true);
  });
});

describe('what React Flow is told about each card up front', () => {
  it('gives every card its measured size and the place of every handle, so only the cards in view are drawn at the start', () => {
    const lr = buildCanvas(flow(), options());
    for (const card of lr.nodes) {
      expect(card.measured).toEqual({ width: card.width, height: card.height });
      const sources = (card.handles ?? []).filter((h) => h.type === 'source');
      expect(sources.map((h) => h.id)).toEqual(card.data.handles.map((h) => h.id));
      expect((card.handles ?? []).some((h) => h.type === 'target')).toBe(card.data.role === 'node');
    }
    const check = lr.nodes.find((n) => n.id === 'check');
    const [inbound, ifTrue, ifFalse] = check?.handles ?? [];
    // Left edge in the middle for what comes in; right edge, a quarter and three quarters down, for the two ways out.
    expect([inbound?.position, inbound?.x]).toEqual(['left', -5]);
    expect([ifTrue?.position, ifTrue?.x, ifTrue?.y]).toEqual(['right', CARD_SIZE.full.width - 5, CARD_SIZE.full.height * 0.25 - 5]);
    expect(ifFalse?.y).toBe(CARD_SIZE.full.height * 0.75 - 5);
  });

  it('puts the handles on the top and bottom edges when the layout runs top to bottom', () => {
    const tb = buildCanvas(flow(), options({ direction: 'TB' }));
    const check = tb.nodes.find((n) => n.id === 'check');
    const [inbound, ifTrue] = check?.handles ?? [];
    expect([inbound?.position, inbound?.y]).toEqual(['top', -5]);
    expect([ifTrue?.position, ifTrue?.y]).toEqual(['bottom', CARD_SIZE.full.height - 5]);
  });
});

describe('the key field', () => {
  it('says the most useful thing about each kind, clipped', () => {
    const doc = flow();
    const field = (id: string) => keyField(doc.nodes.find((n) => n.id === id)!);
    expect(field('pay')).toBe('');
    expect(keyField({ id: 'x', type: 'activity', activity: 'payment.submit' })).toBe('payment.submit');
    expect(keyField({ id: 'x', type: 'timer', durationMs: 90_000 })).toBe('90 s');
    expect(keyField({ id: 'x', type: 'condition', expression: 'x'.repeat(200) }).length).toBeLessThanOrEqual(60);
    expect(field('both')).toBe('join all · 2 branches');
    expect(keyField({ id: 'x', type: 'waitEvent', eventType: 'payment.settled' })).toBe('payment.settled');
    expect(keyField({ id: 'x', type: 'child', workflowId: 'refund', version: 3 })).toBe('refund v3');
  });
});

describe('CanvasBuilder', () => {
  it('keeps the card objects that did not change, so editing one field redraws one card', () => {
    const builder = new CanvasBuilder();
    const base = flow();
    const first = builder.build(base, options());
    const edited: IrDocument = { ...base, nodes: base.nodes.map((n) => (n.id === 'pay' ? { ...n, activity: 'payment.capture' } : n)) };
    const second = builder.build(edited, options());
    for (const card of second.nodes) {
      const before = first.nodes.find((n) => n.id === card.id);
      if (card.id === 'pay') expect(card).not.toBe(before);
      else expect(card).toBe(before);
    }
    expect(second.edges).toEqual(first.edges);
    expect(second.edges[0]).toBe(first.edges[0]);
  });

  it('lays the graph out again when the nodes or their links change, not when a field does', () => {
    const builder = new CanvasBuilder();
    const base = flow();
    const first = builder.build(base, options());
    const edited: IrDocument = { ...base, nodes: base.nodes.map((n) => (n.id === 'pay' ? { ...n, activity: 'x' } : n)) };
    expect(builder.build(edited, options()).nodes.map((n) => n.position)).toEqual(first.nodes.map((n) => n.position));
    const grown = addNode(base, 'timer', 'wait');
    const third = builder.build(connect(grown, 'a', 'next', 'wait'), options());
    expect(third.nodes.some((n) => n.id === 'wait')).toBe(true);
    // Flipping the direction lays it out again.
    const down = builder.build(base, options({ direction: 'TB' }));
    expect(down.nodes.find((n) => n.id === 'pay')?.position).not.toEqual(first.nodes.find((n) => n.id === 'pay')?.position);
  });

  it('reuseCards gives back the old card when nothing visible changed', () => {
    const doc = flow();
    const a = buildCanvas(doc, options()).nodes;
    const b = buildCanvas(doc, options()).nodes;
    const reused = reuseCards(a, b);
    expect(reused.every((card, i) => card === a[i])).toBe(true);
  });

  it('lays out 10 000 nodes without taking long enough to freeze a tab', () => {
    let doc = emptyDocument('big-flow');
    const nodes = Array.from({ length: 10_000 }, (_, i) => ({ id: `n${i}`, type: 'activity', activity: 'a', ...(i < 9_999 ? { next: `n${i + 1}` } : {}) }));
    doc = { ...doc, entrypoint: 'n0', nodes };
    const started = performance.now();
    const model = new CanvasBuilder().build(doc, options({ mode: 'simplified' }));
    expect(model.nodes).toHaveLength(10_001);
    expect(performance.now() - started).toBeLessThan(5_000);
  });
});

describe('layoutInputs', () => {
  it('has a key that changes with the nodes and their links and not with their fields', () => {
    const base = flow();
    const key = layoutInputs(base, 'full').key;
    expect(layoutInputs({ ...base, nodes: base.nodes.map((n) => ({ ...n, note: 'x' })) }, 'full').key).toBe(key);
    expect(layoutInputs(addNode(base, 'timer', 't'), 'full').key).not.toBe(key);
    expect(layoutInputs(base, 'simplified').key).not.toBe(key);
  });
});

describe('keyboard navigation on the canvas', () => {
  const doc = flow();

  it('goes along the flow to the first link out, and back to the node that leads here', () => {
    expect(neighbour(doc, 'check', 'ArrowRight', 'LR')).toBe('pay');
    expect(neighbour(doc, 'pay', 'ArrowRight', 'LR')).toBe('both');
    expect(neighbour(doc, 'pay', 'ArrowLeft', 'LR')).toBe('check');
    expect(neighbour(doc, 'done', 'ArrowRight', 'LR')).toBeUndefined();
  });

  it('steps between the nodes one node leads to, across the flow', () => {
    expect(neighbour(doc, 'a', 'ArrowDown', 'LR')).toBe('b');
    expect(neighbour(doc, 'b', 'ArrowUp', 'LR')).toBe('a');
    expect(neighbour(doc, 'a', 'ArrowUp', 'LR')).toBeUndefined();
    // The last sibling of a fork is whatever else its parent leads to: the join.
    expect(neighbour(doc, 'b', 'ArrowDown', 'LR')).toBe('done');
    expect(neighbour(doc, 'done', 'ArrowDown', 'LR')).toBeUndefined();
  });

  it('turns with the direction of the layout', () => {
    expect(neighbour(doc, 'check', 'ArrowDown', 'TB')).toBe('pay');
    expect(neighbour(doc, 'pay', 'ArrowUp', 'TB')).toBe('check');
    expect(neighbour(doc, 'a', 'ArrowRight', 'TB')).toBe('b');
  });

  it('connects the first step to its start card', () => {
    expect(neighbour(doc, 'check', 'ArrowLeft', 'LR')).toBe(triggerCardId(0));
    expect(neighbour(doc, triggerCardId(0), 'ArrowRight', 'LR')).toBe('check');
  });
});
