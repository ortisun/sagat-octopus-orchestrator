import '@xyflow/react/dist/style.css';
import { Background, Controls, Handle, MiniMap, Position, ReactFlow, ReactFlowProvider, useConnection, useReactFlow } from '@xyflow/react';
import type { Connection, NodeChange, NodeProps } from '@xyflow/react';
import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DragEvent, FocusEvent, KeyboardEvent } from 'react';
import { readableInitialViewport } from '../../model/initialViewport';
import type { IrDocument } from '../../model/ir';
import type { Direction, Point } from '../../model/layout';
import { kindOf } from '../../model/registry';
import { CARD_SIZE, CanvasBuilder, PALETTE_MIME, isTriggerCard, neighbour } from './canvasModel';
import type { Arrow, CanvasEdge, CardData, CardNode } from './canvasModel';

export interface CanvasApi {
  /** Where the middle of the visible canvas is, in canvas coordinates (the top-left of a card that would sit there). */
  viewportCenter(): Point | undefined;
  fit(): void;
}

export interface WorkflowCanvasProps {
  doc: IrDocument;
  mode: 'full' | 'simplified';
  direction: Direction;
  manual: ReadonlyMap<string, Point>;
  selectedId: string | undefined;
  /** The card in the tab order. */
  focusId: string | undefined;
  problems: ReadonlyMap<string, number>;
  advice: ReadonlyMap<string, number>;
  editable: boolean;
  /** A new value asks for the whole graph to be brought into view. */
  fitSignal: number;
  onReady: (api: CanvasApi | null) => void;
  onSelect: (id: string | undefined) => void;
  /** Enter on a card: open the inspector. */
  onOpen: (id: string) => void;
  onFocusCard: (id: string) => void;
  onMoved: (id: string, at: Point) => void;
  onConnect: (from: string, field: string, to: string) => void;
  onStartAt: (to: string) => void;
  onDelete: (id: string) => void;
  onDropKind: (type: string, at: Point | undefined) => void;
  onOpenTriggers: () => void;
}

const ARROWS: readonly string[] = ['ArrowRight', 'ArrowLeft', 'ArrowUp', 'ArrowDown'];

function handleOffset(index: number, count: number): string {
  return `${((index + 0.5) / count) * 100}%`;
}

/** While a link is being drawn from another card, the whole of this card is where it can be dropped. */
const WHOLE_CARD = { top: 0, left: 0, right: 'auto', bottom: 'auto', width: '100%', height: '100%', transform: 'none', borderRadius: 0, border: 0, opacity: 0 } as const;

const CardView = memo(function CardView({ id, data }: NodeProps<CardNode>) {
  const horizontal = data.direction === 'LR';
  const receiving = useConnection((connection) => connection.inProgress && connection.fromNode.id !== id);
  const count = data.handles.length;
  return (
    <div
      className="wf-card"
      data-category={data.category}
      data-role={data.role}
      data-direction={data.direction}
      data-simplified={data.simplified ? 'true' : undefined}
      data-problems={data.problems > 0 ? 'true' : undefined}
    >
      {data.role === 'node' ? <Handle type="target" id="in" position={horizontal ? Position.Left : Position.Top} {...(receiving ? { style: WHOLE_CARD } : {})} /> : null}
      <div className="wf-card-head">
        <span className="wf-shape" aria-hidden="true">
          {data.shape}
        </span>
        {data.simplified || data.role === 'trigger' ? <span className="wf-type">{data.typeLabel}</span> : null}
        <span className="wf-id">{data.title}</span>
        {data.simplified && data.problems > 0 ? (
          <span className="wf-badge" data-tone="bad">
            <span aria-hidden="true">✕</span>
            {data.problems}
          </span>
        ) : null}
      </div>
      {!data.simplified ? (
        <>
          {data.role === 'node' ? <div className="wf-kind-line">{data.typeLabel}</div> : null}
          {data.detail !== '' ? <div className="wf-detail">{data.detail}</div> : null}
          <div className="wf-badges">
            {data.first ? (
              <span className="wf-badge" data-tone="info">
                <span aria-hidden="true">▶</span> First step
              </span>
            ) : null}
            {data.problems > 0 ? (
              <span className="wf-badge" data-tone="bad">
                <span aria-hidden="true">✕</span> {data.problems} {data.problems === 1 ? 'problem' : 'problems'}
              </span>
            ) : null}
            {data.advice > 0 ? (
              <span className="wf-badge" data-tone="warn">
                <span aria-hidden="true">!</span> {data.advice} advice
              </span>
            ) : null}
            {data.unsupported ? (
              <span className="wf-badge" data-tone="warn" title="The platform cannot run this node yet.">
                <span aria-hidden="true">⊘</span> Not run yet
              </span>
            ) : null}
            {data.unknown ? (
              <span className="wf-badge" data-tone="warn">
                <span aria-hidden="true">?</span> Unknown kind
              </span>
            ) : null}
          </div>
          <div className="wf-ports" aria-hidden="true">
            {data.handles.map((h) => (
              <span key={h.id}>{h.label}</span>
            ))}
          </div>
        </>
      ) : null}
      {data.handles.map((h, i) => (
        <Handle
          key={h.id}
          type="source"
          id={h.id}
          position={horizontal ? Position.Right : Position.Bottom}
          style={horizontal ? { top: handleOffset(i, count) } : { left: handleOffset(i, count) }}
        />
      ))}
    </div>
  );
});

const NODE_TYPES = { card: CardView };

function CanvasInner(props: WorkflowCanvasProps) {
  const {
    doc,
    mode,
    direction,
    selectedId,
    focusId,
    editable,
    fitSignal,
    onReady,
    onSelect,
    onOpen,
    onFocusCard,
    onMoved,
    onConnect,
    onStartAt,
    onDelete,
    onDropKind,
    onOpenTriggers,
  } = props;
  const rf = useReactFlow<CardNode, CanvasEdge>();
  const container = useRef<HTMLDivElement>(null);
  const [builder] = useState(() => new CanvasBuilder());
  const [dragging, setDragging] = useState<ReadonlyMap<string, Point>>(new Map());
  const wanted = useRef<string | undefined>(undefined);

  const model = useMemo(
    () => builder.build(doc, { mode, direction, manual: props.manual, selectedId, focusId, problems: props.problems, advice: props.advice }),
    [builder, doc, mode, direction, props.manual, selectedId, focusId, props.problems, props.advice],
  );
  const initialCard = useMemo(
    () => model.nodes.find((node) => isTriggerCard(node.id)) ?? model.nodes.find((node) => node.id === doc.entrypoint) ?? model.nodes[0],
    [doc.entrypoint, model.nodes],
  );
  const initialViewport = useMemo(() => readableInitialViewport(initialCard?.position), [initialCard]);
  const nodes = useMemo(
    () => (dragging.size === 0 ? model.nodes : model.nodes.map((n) => (dragging.has(n.id) ? { ...n, position: dragging.get(n.id) ?? n.position } : n))),
    [model.nodes, dragging],
  );

  useEffect(() => {
    onReady({
      viewportCenter: () => {
        const box = container.current?.getBoundingClientRect();
        if (!box) return undefined;
        const centre = rf.screenToFlowPosition({ x: box.left + box.width / 2, y: box.top + box.height / 2 });
        const size = CARD_SIZE[mode];
        return { x: Math.round(centre.x - size.width / 2), y: Math.round(centre.y - size.height / 2) };
      },
      fit: () => void rf.fitView({ duration: 0 }),
    });
    return () => onReady(null);
  }, [onReady, rf, mode]);

  useEffect(() => {
    if (fitSignal === 0) return;
    const frame = requestAnimationFrame(() => void rf.fitView({ duration: 0 }));
    return () => cancelAnimationFrame(frame);
  }, [fitSignal, rf]);

  // Focus follows the roving tab stop once the card exists in the page (it may have to be panned into view first).
  useEffect(() => {
    if (wanted.current === undefined || wanted.current !== focusId) return;
    let tries = 0;
    let frame = 0;
    const attempt = () => {
      const card = [...(container.current?.querySelectorAll<HTMLElement>('.react-flow__node') ?? [])].find((el) => el.dataset['id'] === wanted.current);
      if (card) {
        card.focus({ preventScroll: true });
        wanted.current = undefined;
      } else if (tries++ < 8) {
        frame = requestAnimationFrame(attempt);
      }
    };
    attempt();
    return () => cancelAnimationFrame(frame);
  }, [focusId]);

  const moveFocusTo = useCallback(
    (id: string) => {
      wanted.current = id;
      onFocusCard(id);
      const card = model.nodes.find((n) => n.id === id);
      if (card) void rf.setCenter(card.position.x + (card.width ?? 0) / 2, card.position.y + (card.height ?? 0) / 2, { zoom: rf.getZoom(), duration: 0 });
    },
    [model.nodes, onFocusCard, rf],
  );

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const target = event.target;
    if (!(target instanceof HTMLElement) || !target.classList.contains('react-flow__node')) return;
    const id = target.dataset['id'];
    if (id === undefined) return;
    if (ARROWS.includes(event.key)) {
      event.preventDefault();
      const to = neighbour(doc, id, event.key as Arrow, direction);
      if (to !== undefined) moveFocusTo(to);
    } else if (event.key === 'Enter') {
      event.preventDefault();
      if (isTriggerCard(id)) onOpenTriggers();
      else onOpen(id);
    } else if (event.key === ' ') {
      event.preventDefault();
      if (isTriggerCard(id)) onOpenTriggers();
      else onSelect(id);
    } else if ((event.key === 'Delete' || event.key === 'Backspace') && editable && !isTriggerCard(id)) {
      event.preventDefault();
      // Focus stays on the canvas: on a neighbour of the card that is about to go.
      const heir =
        neighbour(doc, id, direction === 'LR' ? 'ArrowRight' : 'ArrowDown', direction) ??
        neighbour(doc, id, direction === 'LR' ? 'ArrowLeft' : 'ArrowUp', direction) ??
        doc.nodes.find((n) => n.id !== id)?.id;
      if (heir !== undefined) moveFocusTo(heir);
      onDelete(id);
    }
  };

  // Tab brings focus to a card: that card becomes the one in the tab order.
  const onFocus = (event: FocusEvent<HTMLDivElement>) => {
    const target = event.target;
    if (target instanceof HTMLElement && target.classList.contains('react-flow__node')) {
      const id = target.dataset['id'];
      if (id !== undefined && id !== focusId) onFocusCard(id);
    }
  };

  const handleNodesChange = useCallback((changes: NodeChange<CardNode>[]) => {
    for (const change of changes) {
      if (change.type === 'position' && change.dragging === true && change.position) {
        const { id, position } = change;
        setDragging((previous) => new Map(previous).set(id, position));
      }
    }
  }, []);

  const handleConnect = (connection: Connection) => {
    if (!editable || connection.target === null) return;
    if (isTriggerCard(connection.source)) onStartAt(connection.target);
    else if (connection.sourceHandle !== null) onConnect(connection.source, connection.sourceHandle, connection.target);
  };

  const onDragOver = (event: DragEvent<HTMLDivElement>) => {
    if (editable && event.dataTransfer.types.includes(PALETTE_MIME)) {
      event.preventDefault();
      event.dataTransfer.dropEffect = 'copy';
    }
  };

  const onDrop = (event: DragEvent<HTMLDivElement>) => {
    const type = event.dataTransfer.getData(PALETTE_MIME);
    if (!editable || !kindOf(type)) return;
    event.preventDefault();
    const at = rf.screenToFlowPosition({ x: event.clientX, y: event.clientY });
    const size = CARD_SIZE[mode];
    onDropKind(type, Number.isFinite(at.x) && Number.isFinite(at.y) ? { x: Math.round(at.x - size.width / 2), y: Math.round(at.y - size.height / 2) } : undefined);
  };

  return (
    // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions -- the cards inside are focusable and their key and focus events bubble up to this region
    <div
      ref={container}
      className="wf-canvas"
      role="region"
      aria-label="Workflow canvas"
      aria-describedby="wf-canvas-help"
      onKeyDown={onKeyDown}
      onFocus={onFocus}
      onDragOver={onDragOver}
      onDrop={onDrop}
      data-testid="canvas"
      data-mode={mode}
    >
      <p id="wf-canvas-help" className="visually-hidden">
        Each card is a step. Tab reaches the cards; the arrow keys move between connected cards, Enter opens the inspector, Delete removes the card. The list view does everything
        the canvas does.
      </p>
      <ReactFlow<CardNode, CanvasEdge>
        nodes={nodes}
        edges={model.edges}
        nodeTypes={NODE_TYPES}
        onNodesChange={handleNodesChange}
        onNodeClick={(_, node) => (isTriggerCard(node.id) ? onOpenTriggers() : onSelect(node.id))}
        onPaneClick={() => onSelect(undefined)}
        onNodeDragStop={(_, node) => {
          setDragging(new Map());
          onMoved(node.id, node.position);
        }}
        onConnect={handleConnect}
        isValidConnection={(connection) => connection.target !== connection.source && !isTriggerCard(connection.target)}
        nodesDraggable
        nodesConnectable={editable}
        nodesFocusable
        edgesFocusable={false}
        edgesReconnectable={false}
        elementsSelectable
        disableKeyboardA11y
        deleteKeyCode={null}
        selectionKeyCode={null}
        multiSelectionKeyCode={null}
        zoomOnDoubleClick={false}
        onlyRenderVisibleElements={mode === 'simplified'}
        defaultViewport={initialViewport}
        minZoom={0.05}
        maxZoom={2}
        colorMode="system"
      >
        <Background gap={24} />
        <Controls showInteractive={false} aria-label="Zoom and fit" />
        {mode === 'full' ? <MiniMap pannable zoomable ariaLabel="Overview of the workflow" /> : null}
      </ReactFlow>
    </div>
  );
}

export function WorkflowCanvas(props: WorkflowCanvasProps) {
  return (
    <ReactFlowProvider>
      <CanvasInner {...props} />
    </ReactFlowProvider>
  );
}

export type { CardData };
