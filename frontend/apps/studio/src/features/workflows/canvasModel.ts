import { MarkerType, Position } from '@xyflow/react';
import type { Edge, Node, NodeHandle } from '@xyflow/react';
import type { IrDocument, IrNode, IrTrigger } from '../../model/ir';
import { layoutGraph } from '../../model/layout';
import type { Direction, LayoutEdge, LayoutNode, Point } from '../../model/layout';
import { kindOf, linksOf } from '../../model/registry';
import type { Category, EdgeKind } from '../../model/registry';
import { formatDuration } from './duration';

/** What a palette item carries when it is dragged onto the canvas. */
export const PALETTE_MIME = 'application/x-sagat-node-kind';

/** Up to this many nodes the canvas draws full cards (D147). */
export const FULL_UP_TO = 1_000;
/** Above this many nodes the Studio opens the list view and offers the canvas on demand (D147). */
export const LIST_ABOVE = 10_000;

export type CanvasMode = 'full' | 'simplified' | 'list';

export function canvasMode(nodeCount: number): CanvasMode {
  if (nodeCount > LIST_ABOVE) return 'list';
  return nodeCount > FULL_UP_TO ? 'simplified' : 'full';
}

export const CARD_SIZE = {
  full: { width: 320, height: 192 },
  simplified: { width: 248, height: 104 },
  trigger: { width: 320, height: 112 },
} as const;

/** Trigger cards share the canvas with the nodes; `@` and `:` cannot be part of a node id. */
export const TRIGGER_PREFIX = '@trigger:';

export function triggerCardId(index: number): string {
  return `${TRIGGER_PREFIX}${index}`;
}

export function isTriggerCard(id: string): boolean {
  return id.startsWith(TRIGGER_PREFIX);
}

export interface CardHandle {
  id: string;
  label: string;
}

export type CardData = {
  role: 'node' | 'trigger';
  /** The IR id of a node; the label of a trigger. */
  title: string;
  /** The type in words: Step, Condition, Start. */
  typeLabel: string;
  type: string;
  category: Category | 'start';
  /** A shape in text, repeated by the border of the card: the kind is never colour alone. */
  shape: string;
  /** The key field: the activity name, the expression, the delay. */
  detail: string;
  handles: readonly CardHandle[];
  first: boolean;
  problems: number;
  advice: number;
  unsupported: boolean;
  unknown: boolean;
  simplified: boolean;
  direction: Direction;
};

export type CardNode = Node<CardData, 'card'>;

export type CanvasEdgeKind = EdgeKind | 'start';

export interface EdgeInfo extends Record<string, unknown> {
  kind: CanvasEdgeKind;
  from: string;
  to: string;
  field: string;
  index?: number;
}

export type CanvasEdge = Edge<EdgeInfo>;

const SHAPE: Record<Category | 'start', string> = { work: '▭', flow: '◇', wait: '⬭', saga: '▣', end: '●', start: '▶' };

const EDGE_WORDS: Record<CanvasEdgeKind, string | undefined> = {
  next: undefined,
  error: 'On error',
  true: 'If true',
  false: 'If false',
  branch: 'Branch',
  timeout: 'On timeout',
  begin: 'First step',
  body: 'Body',
  start: 'First step',
};

/** Dash patterns tell the kinds of link apart where colour cannot. */
const EDGE_DASH: Record<CanvasEdgeKind, string | undefined> = {
  next: undefined,
  error: '7 4',
  true: undefined,
  false: '2 4',
  branch: '12 3 2 3',
  timeout: '1 6',
  begin: undefined,
  body: '8 2',
  start: '3 3',
};

export function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

function text(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

/** The one field that says most about a node, as short text. */
export function keyField(node: IrNode): string {
  switch (node.type) {
    case 'activity':
      return clip(text(node['activity']), 60);
    case 'database': {
      const ref = node['connectionRef'];
      return ref !== null && typeof ref === 'object' && 'name' in ref ? clip(`${text(node['operation'])} · ${String(ref.name)}`, 60) : '';
    }
    case 'script': {
      const digest = text(node['artifactDigest']);
      return clip(digest, 24);
    }
    case 'condition':
      return clip(text(node['expression']), 60);
    case 'parallel': {
      const branches = Array.isArray(node['branches']) ? node['branches'].length : 0;
      return `join ${text(node['join']) || 'all'} · ${branches} ${branches === 1 ? 'branch' : 'branches'}`;
    }
    case 'timer':
      return typeof node['durationMs'] === 'number' ? formatDuration(node['durationMs']) : '';
    case 'waitEvent':
      return clip(text(node['eventType']), 60);
    case 'child': {
      const version = typeof node['version'] === 'number' ? ` v${node['version']}` : '';
      return clip(`${text(node['workflowId'])}${text(node['workflowId']) === '' ? '' : version}`, 60);
    }
    case 'saga':
      return text(node['begin']) === '' ? '' : `begins at ${clip(text(node['begin']), 40)}`;
    case 'end':
      return clip(text(node['outputExpr']), 60);
    case 'foreach':
      return clip(text(node['collectionExpr']), 60);
    case 'humanApproval':
      return clip(text(node['approverPolicy']), 60);
    default:
      return '';
  }
}

export function triggerLabel(trigger: IrTrigger): { label: string; detail: string } {
  switch (trigger.kind) {
    case 'manual':
      return { label: 'Manual start', detail: 'Started by a person or the API' };
    case 'event':
      return { label: 'Event', detail: clip(text(trigger['eventType']), 50) };
    case 'webhook':
      return { label: 'Webhook', detail: clip(text(trigger['endpoint']), 50) };
    case 'message':
      return { label: 'Message', detail: clip(text(trigger['connector']), 50) };
    case 'cron':
      return { label: 'Schedule', detail: clip(text(trigger['cron']), 50) };
    default:
      return { label: clip(String(trigger.kind), 20), detail: '' };
  }
}

/** What the layout needs: every card with its size, and the links between them. */
export function layoutInputs(doc: IrDocument, mode: Exclude<CanvasMode, 'list'>): { nodes: LayoutNode[]; edges: LayoutEdge[]; key: string } {
  const size = CARD_SIZE[mode];
  const nodes: LayoutNode[] = [];
  const edges: LayoutEdge[] = [];
  (doc.triggers ?? []).forEach((_, index) => {
    nodes.push({ id: triggerCardId(index), ...CARD_SIZE.trigger });
    if (doc.entrypoint !== '') edges.push({ from: triggerCardId(index), to: doc.entrypoint });
  });
  for (const node of doc.nodes) {
    nodes.push({ id: node.id, ...size });
    for (const link of linksOf(node)) edges.push({ from: link.from, to: link.to });
  }
  const key = `${mode}\u0001${nodes.map((n) => n.id).join('\u0002')}\u0001${edges.map((e) => `${e.from}>${e.to}`).join('\u0002')}`;
  return { nodes, edges, key };
}

export interface CanvasOptions {
  direction: Direction;
  mode: Exclude<CanvasMode, 'list'>;
  /** The computed layout. */
  auto: ReadonlyMap<string, Point>;
  /** Positions the user chose by dragging; they win over the layout. */
  manual: ReadonlyMap<string, Point>;
  selectedId: string | undefined;
  /** The card that is in the tab order (roving tabindex). */
  focusId: string | undefined;
  /** Problems the platform reported, by node id. */
  problems: ReadonlyMap<string, number>;
  advice: ReadonlyMap<string, number>;
}

export interface CanvasModel {
  nodes: CardNode[];
  edges: CanvasEdge[];
  /** Links to a node that does not exist (a draft written by hand can have them); they are not drawn. */
  dangling: { from: string; field: string; to: string }[];
}

function describe(data: CardData): string {
  const parts = [`${data.typeLabel} ${data.title}`];
  if (data.detail !== '') parts.push(data.detail);
  if (data.first) parts.push('first step');
  if (data.problems > 0) parts.push(`${data.problems} ${data.problems === 1 ? 'problem' : 'problems'}`);
  if (data.advice > 0) parts.push(`${data.advice} ${data.advice === 1 ? 'piece of advice' : 'pieces of advice'}`);
  if (data.unsupported) parts.push('the platform cannot run this node yet');
  if (data.unknown) parts.push('a kind of node this Studio does not know');
  return parts.join(', ');
}

/** The size of a handle on the card (`.wf-canvas .react-flow__handle` in workflows.css). */
const HANDLE = 10;

/**
 * Where the handles of a card are, worked out from its fixed size and the way the card lays them out.
 * React Flow draws a node it has no handle positions for until it has drawn it and measured it, so
 * telling it up front is what lets it draw only the cards in view of a graph of ten thousand.
 */
function handleGeometry(handles: readonly CardHandle[], hasInput: boolean, direction: Direction, width: number, height: number): NodeHandle[] {
  const horizontal = direction === 'LR';
  const out: NodeHandle[] = [];
  if (hasInput) {
    out.push(
      horizontal
        ? { id: 'in', type: 'target', position: Position.Left, x: -HANDLE / 2, y: height / 2 - HANDLE / 2, width: HANDLE, height: HANDLE }
        : { id: 'in', type: 'target', position: Position.Top, x: width / 2 - HANDLE / 2, y: -HANDLE / 2, width: HANDLE, height: HANDLE },
    );
  }
  handles.forEach((handle, index) => {
    const along = (index + 0.5) / handles.length;
    out.push(
      horizontal
        ? { id: handle.id, type: 'source', position: Position.Right, x: width - HANDLE / 2, y: along * height - HANDLE / 2, width: HANDLE, height: HANDLE }
        : { id: handle.id, type: 'source', position: Position.Bottom, x: along * width - HANDLE / 2, y: height - HANDLE / 2, width: HANDLE, height: HANDLE },
    );
  });
  return out;
}

const START_HANDLES: readonly CardHandle[] = [{ id: 'start', label: 'First step' }];
const DEFAULT_HANDLES: readonly CardHandle[] = [
  { id: 'next', label: 'Next' },
  { id: 'onError', label: 'On error' },
];
const HANDLES_OF = new Map<string, readonly CardHandle[]>();

/** The handles of a kind as the card shows them; one list per kind, so an unchanged card compares equal. */
function cardHandles(type: string): readonly CardHandle[] {
  const spec = kindOf(type);
  if (!spec) return DEFAULT_HANDLES;
  let handles = HANDLES_OF.get(type);
  if (!handles) {
    handles = spec.handles.map((h) => ({ id: h.field, label: h.label }));
    HANDLES_OF.set(type, handles);
  }
  return handles;
}

export function buildCanvas(doc: IrDocument, options: CanvasOptions): CanvasModel {
  const size = CARD_SIZE[options.mode];
  const simplified = options.mode === 'simplified';
  const at = (id: string): Point => options.manual.get(id) ?? options.auto.get(id) ?? { x: 0, y: 0 };
  const nodes: CardNode[] = [];
  const known = new Set(doc.nodes.map((n) => n.id));

  (doc.triggers ?? []).forEach((trigger, index) => {
    const id = triggerCardId(index);
    const { label, detail } = triggerLabel(trigger);
    const data: CardData = {
      role: 'trigger',
      title: label,
      typeLabel: 'Start',
      type: `trigger:${String(trigger.kind)}`,
      category: 'start',
      shape: SHAPE.start,
      detail,
      handles: START_HANDLES,
      first: false,
      problems: 0,
      advice: 0,
      unsupported: false,
      unknown: false,
      simplified,
      direction: options.direction,
    };
    nodes.push({
      id,
      type: 'card',
      position: at(id),
      data,
      width: CARD_SIZE.trigger.width,
      height: CARD_SIZE.trigger.height,
      measured: { width: CARD_SIZE.trigger.width, height: CARD_SIZE.trigger.height },
      handles: handleGeometry(START_HANDLES, false, options.direction, CARD_SIZE.trigger.width, CARD_SIZE.trigger.height),
      draggable: true,
      ariaLabel: `Start: ${label}${detail === '' ? '' : `, ${detail}`}. Opens the workflow settings.`,
      domAttributes: { tabIndex: options.focusId === id ? 0 : -1 },
    });
  });

  for (const node of doc.nodes) {
    const spec = kindOf(node.type);
    const data: CardData = {
      role: 'node',
      title: clip(node.id, 64),
      typeLabel: spec?.label ?? clip(node.type, 24),
      type: node.type,
      category: spec?.category ?? 'work',
      shape: SHAPE[spec?.category ?? 'work'],
      detail: keyField(node),
      handles: cardHandles(node.type),
      first: doc.entrypoint === node.id,
      problems: options.problems.get(node.id) ?? 0,
      advice: options.advice.get(node.id) ?? 0,
      unsupported: spec !== undefined && !spec.runs,
      unknown: spec === undefined,
      simplified,
      direction: options.direction,
    };
    nodes.push({
      id: node.id,
      type: 'card',
      position: at(node.id),
      data,
      selected: options.selectedId === node.id,
      width: size.width,
      height: size.height,
      // The cards have a fixed size, so they are known to React Flow from the start: it draws only those in view
      // at once, instead of drawing all of them until each has been measured.
      measured: { width: size.width, height: size.height },
      handles: handleGeometry(data.handles, true, options.direction, size.width, size.height),
      ariaLabel: `${describe(data)}. Press Enter to open the inspector.`,
      domAttributes: { tabIndex: options.focusId === node.id ? 0 : -1 },
    });
  }

  const edges: CanvasEdge[] = [];
  const dangling: CanvasModel['dangling'] = [];
  const edge = (info: EdgeInfo, id: string, source: string, sourceHandle: string, target: string, label: string | undefined): void => {
    const dash = EDGE_DASH[info.kind];
    edges.push({
      id,
      source,
      sourceHandle,
      target,
      targetHandle: 'in',
      type: simplified ? 'straight' : 'default',
      data: info,
      focusable: false,
      selectable: false,
      markerEnd: { type: MarkerType.ArrowClosed, width: 16, height: 16 },
      style: { strokeWidth: info.kind === 'begin' || info.kind === 'start' ? 2.5 : 1.5, ...(dash ? { strokeDasharray: dash } : {}) },
      ariaLabel: `${label ?? 'Next'} link from ${clip(source, 40)} to ${clip(target, 40)}`,
      ...(label && !simplified ? { label, labelShowBg: true, labelStyle: { fontSize: 11 }, labelBgPadding: [4, 2] as [number, number], labelBgBorderRadius: 3 } : {}),
    });
  };

  if (doc.entrypoint !== '' && known.has(doc.entrypoint)) {
    (doc.triggers ?? []).forEach((_, index) => {
      const from = triggerCardId(index);
      edge({ kind: 'start', from, to: doc.entrypoint, field: 'start' }, `${from}|start|${doc.entrypoint}`, from, 'start', doc.entrypoint, EDGE_WORDS.start);
    });
  }
  for (const node of doc.nodes) {
    for (const link of linksOf(node)) {
      if (!known.has(link.to)) {
        dangling.push({ from: link.from, field: link.field, to: link.to });
        continue;
      }
      const base = EDGE_WORDS[link.kind];
      const label = link.kind === 'branch' && link.index !== undefined ? `Branch ${link.index + 1}` : base;
      edge(
        { kind: link.kind, from: link.from, to: link.to, field: link.field, ...(link.index === undefined ? {} : { index: link.index }) },
        `${link.from}|${link.field}${link.index ?? ''}|${link.to}`,
        link.from,
        link.field,
        link.to,
        label,
      );
    }
  }
  return { nodes, edges, dangling };
}

function sameCard(a: CardNode, b: CardNode): boolean {
  if (a.position.x !== b.position.x || a.position.y !== b.position.y) return false;
  if (a.selected !== b.selected || a.ariaLabel !== b.ariaLabel) return false;
  if (a.domAttributes?.tabIndex !== b.domAttributes?.tabIndex) return false;
  const keys = Object.keys(b.data) as (keyof CardData)[];
  return keys.every((key) => a.data[key] === b.data[key]);
}

/**
 * The nodes of a new model, keeping the very object of every card that did not change, so React
 * Flow and the cards skip the work for them: typing in one field redraws one card, not a thousand.
 */
export function reuseCards(previous: readonly CardNode[], next: CardNode[]): CardNode[] {
  const before = new Map(previous.map((n) => [n.id, n]));
  return next.map((card) => {
    const old = before.get(card.id);
    return old && sameCard(old, card) ? old : card;
  });
}

function reuseEdges(previous: readonly CanvasEdge[], next: CanvasEdge[]): CanvasEdge[] {
  // An edge's id holds its source, handle and target; what else can differ is how it is drawn.
  const before = new Map(previous.map((e) => [e.id, e]));
  return next.map((edge) => {
    const old = before.get(edge.id);
    return old && old.type === edge.type && old.label === edge.label ? old : edge;
  });
}

export type Arrow = 'ArrowRight' | 'ArrowLeft' | 'ArrowUp' | 'ArrowDown';

function outgoing(doc: IrDocument, id: string): string[] {
  if (isTriggerCard(id)) return doc.entrypoint !== '' && doc.nodes.some((n) => n.id === doc.entrypoint) ? [doc.entrypoint] : [];
  const node = doc.nodes.find((n) => n.id === id);
  if (!node) return [];
  const known = new Set(doc.nodes.map((n) => n.id));
  const out: string[] = [];
  for (const link of linksOf(node)) if (known.has(link.to) && !out.includes(link.to)) out.push(link.to);
  return out;
}

function incoming(doc: IrDocument, id: string): string | undefined {
  if (isTriggerCard(id)) return undefined;
  if (doc.entrypoint === id && (doc.triggers ?? []).length > 0) return triggerCardId(0);
  for (const node of doc.nodes) if (linksOf(node).some((l) => l.to === id)) return node.id;
  return undefined;
}

/**
 * Where an arrow key moves focus on the canvas, by the shape of the graph rather than by pixels:
 * along the flow follows the first link out, against it goes back to the node that leads here, and
 * across it steps between the siblings that one node leads to.
 */
export function neighbour(doc: IrDocument, id: string, key: Arrow, direction: Direction): string | undefined {
  const along = direction === 'LR' ? 'ArrowRight' : 'ArrowDown';
  const against = direction === 'LR' ? 'ArrowLeft' : 'ArrowUp';
  if (key === along) return outgoing(doc, id)[0];
  if (key === against) return incoming(doc, id);
  const parent = incoming(doc, id);
  if (parent === undefined) return undefined;
  const siblings = outgoing(doc, parent);
  const at = siblings.indexOf(id);
  if (at === -1) return undefined;
  const forward = key === (direction === 'LR' ? 'ArrowDown' : 'ArrowRight');
  return siblings[at + (forward ? 1 : -1)];
}

/**
 * Builds the canvas again and again as the document changes without redoing what did not change:
 * the layout (the expensive part) is computed only when the nodes or their links change, and the
 * cards that look the same keep their objects.
 */
export class CanvasBuilder {
  #layoutKey: string | undefined;
  #auto: Map<string, Point> = new Map();
  #cards: CardNode[] = [];
  #edges: CanvasEdge[] = [];

  build(doc: IrDocument, options: Omit<CanvasOptions, 'auto'>): CanvasModel {
    const inputs = layoutInputs(doc, options.mode);
    const key = `${inputs.key}\u0001${options.direction}`;
    if (key !== this.#layoutKey) {
      this.#auto = layoutGraph(inputs.nodes, inputs.edges, options.direction);
      this.#layoutKey = key;
    }
    const model = buildCanvas(doc, { ...options, auto: this.#auto });
    this.#cards = reuseCards(this.#cards, model.nodes);
    this.#edges = reuseEdges(this.#edges, model.edges);
    return { ...model, nodes: this.#cards, edges: this.#edges };
  }
}
