import { MarkerType } from '@xyflow/react';
import type { Edge, Node } from '@xyflow/react';
import type { RuntimeEdge, RuntimeNode } from '../../api/types';
import type { LayoutNode, Point } from '../../model/layout';
import { edgeState } from '../../model/runGraph';
import type { EdgeState } from '../../model/runGraph';
import { edgeKindWords, nodeSummary } from './display';

/** D147: up to this many nodes the canvas is interactive with full cards. */
export const FULL_CARDS_UP_TO = 1000;
/** D147: above this many nodes the page opens in the table view and offers the canvas on demand. */
export const TABLE_FIRST_ABOVE = 10_000;

export interface ViewPolicy {
  /** Cards with everything, or small chips. */
  detail: 'full' | 'simple';
  /** The view the page opens in. */
  start: 'graph' | 'table';
  /** A minimap draws every node again; it is left out of big graphs. */
  minimap: boolean;
  /** Labels on the edges are text in the SVG: they are left out of big graphs. */
  edgeLabels: boolean;
}

export function viewPolicy(nodeCount: number): ViewPolicy {
  if (nodeCount <= FULL_CARDS_UP_TO) return { detail: 'full', start: 'graph', minimap: true, edgeLabels: true };
  return { detail: 'simple', start: nodeCount > TABLE_FIRST_ABOVE ? 'table' : 'graph', minimap: false, edgeLabels: false };
}

export const CARD_SIZE = { width: 264, height: 132 } as const;
export const CHIP_SIZE = { width: 176, height: 44 } as const;

export function sizeFor(detail: ViewPolicy['detail']): { width: number; height: number } {
  return detail === 'full' ? CARD_SIZE : CHIP_SIZE;
}

/** What a card on the canvas is drawn from. */
export type CardData = { node: RuntimeNode; flash: number };
export type CardNode = Node<CardData, 'run-card' | 'run-chip'>;

export function layoutNodes(order: readonly string[], detail: ViewPolicy['detail']): LayoutNode[] {
  const { width, height } = sizeFor(detail);
  return order.map((id) => ({ id, width, height }));
}

interface CachedNode {
  x: number;
  y: number;
  detail: ViewPolicy['detail'];
  selected: boolean;
  flash: number;
  value: CardNode;
}

/** A card keeps its identity while nothing it shows changed, so the canvas redraws only what changed. */
const NODE_CACHE = new WeakMap<RuntimeNode, CachedNode>();

export interface FlowInput {
  nodes: ReadonlyMap<string, RuntimeNode>;
  order: readonly string[];
  positions: ReadonlyMap<string, Point>;
  detail: ViewPolicy['detail'];
  selectedId: string | undefined;
  /** The nodes the last update touched, and the update's number (it restarts the highlight). */
  flash: { gen: number; ids: ReadonlySet<string> };
}

export function flowNodes(input: FlowInput): CardNode[] {
  const { width, height } = sizeFor(input.detail);
  const out: CardNode[] = [];
  for (const id of input.order) {
    const node = input.nodes.get(id);
    const at = input.positions.get(id);
    if (node === undefined || at === undefined) continue;
    const selected = id === input.selectedId;
    const flash = input.flash.ids.has(id) ? input.flash.gen : 0;
    const cached = NODE_CACHE.get(node);
    if (cached && cached.x === at.x && cached.y === at.y && cached.detail === input.detail && cached.selected === selected && cached.flash === flash) {
      out.push(cached.value);
      continue;
    }
    const value: CardNode = {
      id,
      type: input.detail === 'full' ? 'run-card' : 'run-chip',
      position: { x: at.x, y: at.y },
      width,
      height,
      selected,
      draggable: false,
      connectable: false,
      deletable: false,
      data: { node, flash },
      ariaLabel: nodeSummary(node),
    };
    NODE_CACHE.set(node, { x: at.x, y: at.y, detail: input.detail, selected, flash, value });
    out.push(value);
  }
  return out;
}

export function stateWords(state: EdgeState): string {
  switch (state) {
    case 'taken':
      return 'taken';
    case 'pending':
      return 'not decided yet';
    case 'not-taken':
      return 'omitted';
  }
}

interface CachedEdge {
  state: EdgeState;
  labels: boolean;
  value: Edge;
}

const EDGE_CACHE = new WeakMap<RuntimeEdge, CachedEdge>();

/**
 * The links of the definition as the run went through them: taken (solid), not decided yet
 * (dashed), omitted (dotted and faint), each named by its kind in text. A style is never the only
 * signal (the label and the accessible name say the same).
 */
export function flowEdges(edges: readonly RuntimeEdge[], nodes: ReadonlyMap<string, RuntimeNode>, labels: boolean): Edge[] {
  const seen = new Map<string, number>();
  return edges.map((edge) => {
    const state = edgeState(edge, nodes);
    const cached = EDGE_CACHE.get(edge);
    const base = `${edge.from}>${edge.to}#${edge.kind}`;
    const n = seen.get(base) ?? 0;
    seen.set(base, n + 1);
    if (cached && cached.state === state && cached.labels === labels && cached.value.id === (n === 0 ? base : `${base}#${n}`)) return cached.value;
    const kind = edgeKindWords(edge.kind);
    const value: Edge = {
      id: n === 0 ? base : `${base}#${n}`,
      source: edge.from,
      target: edge.to,
      type: labels ? 'smoothstep' : 'straight',
      className: `edge-${state} edge-kind-${edge.kind}`,
      selectable: false,
      focusable: false,
      deletable: false,
      markerEnd: { type: MarkerType.ArrowClosed, width: 16, height: 16 },
      ariaLabel: `${edge.from} to ${edge.to}: ${kind}, ${stateWords(state)}`,
      ...(labels ? { label: state === 'not-taken' ? `${kind} (omitted)` : kind } : {}),
    };
    EDGE_CACHE.set(edge, { state, labels, value });
    return value;
  });
}

/** What leads into and out of a node, for the panel: the structure of the graph in words, for those who cannot see the canvas. */
export interface Neighbour {
  id: string;
  kind: string;
  state: EdgeState;
}

export function neighbours(edges: readonly RuntimeEdge[], nodes: ReadonlyMap<string, RuntimeNode>, id: string): { from: Neighbour[]; to: Neighbour[] } {
  const from: Neighbour[] = [];
  const to: Neighbour[] = [];
  for (const edge of edges) {
    if (edge.to === id) from.push({ id: edge.from, kind: edgeKindWords(edge.kind), state: edgeState(edge, nodes) });
    if (edge.from === id) to.push({ id: edge.to, kind: edgeKindWords(edge.kind), state: edgeState(edge, nodes) });
  }
  return { from, to };
}
