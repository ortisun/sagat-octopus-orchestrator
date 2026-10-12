import { Controls, MiniMap, ReactFlow, useNodesInitialized, useReactFlow } from '@xyflow/react';
import type { NodeChange } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { useEffect, useMemo, useRef } from 'react';
import type { RefObject } from 'react';
import { layoutGraph } from '../../model/layout';
import type { RunModel } from '../../model/runGraph';
import { flowEdges, flowNodes, layoutNodes, sizeFor } from './canvas';
import type { CardNode, ViewPolicy } from './canvas';
import { NODE_TYPES } from './RunNodeCard';

const ARIA = {
  'node.a11yDescription.default': 'Press Enter or Space to open the details of this step. The table view lists the same steps.',
  'node.a11yDescription.keyboardDisabled': 'The table view lists the same steps.',
  'edge.a11yDescription.default': 'A link between two steps.',
  'controls.ariaLabel': 'Graph view controls',
  'minimap.ariaLabel': 'Overview of the graph',
};

const FIRST_STEP_MARGIN = 16;

interface InitialGraphGeometry {
  canvasTop: number;
  canvasBottom: number;
  viewportHeight: number;
  graphTop: number;
  firstRowHeight: number;
}

/**
 * Fit view centres a small graph vertically. Anchor its first row at the visible top of the canvas
 * so content that resolves above the canvas cannot later push every card below the page fold.
 */
export function initialGraphViewportOffset(geometry: InitialGraphGeometry): number {
  const visibleTop = Math.max(0, -geometry.canvasTop) + FIRST_STEP_MARGIN;
  const visibleBottom = Math.min(geometry.viewportHeight, geometry.canvasBottom) - geometry.canvasTop - FIRST_STEP_MARGIN;
  if (geometry.firstRowHeight <= 0 || visibleBottom - visibleTop < geometry.firstRowHeight) return 0;
  return visibleTop - geometry.graphTop;
}

/** Fits exactly once, after React Flow has measured every node. Later data updates and user camera
 * changes do not run this effect again. Logical graph bounds avoid depending on virtualized DOM. */
function InitialViewport({ canvas, detail }: { canvas: RefObject<HTMLDivElement | null>; detail: ViewPolicy['detail'] }) {
  const flow = useReactFlow<CardNode>();
  const nodesInitialized = useNodesInitialized();
  const started = useRef(false);

  useEffect(() => {
    if (!flow.viewportInitialized || !nodesInitialized || started.current) return;
    started.current = true;
    void (async () => {
      const fitted = await flow.fitView({ padding: 0.1, duration: 0 });
      if (!fitted) return;
      const element = canvas.current;
      if (element === null) return;
      const viewport = flow.getViewport();
      const graph = flow.getNodesBounds(flow.getNodes());
      const bounds = element.getBoundingClientRect();
      const offset = initialGraphViewportOffset({
        canvasTop: bounds.top,
        canvasBottom: bounds.bottom,
        viewportHeight: window.innerHeight,
        graphTop: viewport.y + graph.y * viewport.zoom,
        firstRowHeight: sizeFor(detail).height * viewport.zoom,
      });
      if (offset !== 0) await flow.setViewport({ ...viewport, y: viewport.y + offset }, { duration: 0 });
    })();
  }, [canvas, detail, flow, nodesInitialized]);

  return null;
}

/** The colour of a step in the overview: only a hint, the cards say it in words. */
function overviewColor(node: { data?: unknown }): string {
  const status = (node.data as { node?: { status?: string } } | undefined)?.node?.status;
  if (status === 'FAILED') return 'var(--status-bad)';
  if (status === 'SUCCEEDED') return 'var(--status-ok)';
  if (status === 'RUNNING' || status === 'LEASED') return 'var(--status-running)';
  return 'var(--border)';
}

/**
 * The definition's graph with the state of the run on it (read-only). The layout depends only on
 * the shape of the graph, so an update never moves anything, and the viewport is only fitted when
 * the canvas opens. Every step can be reached and chosen with the keyboard; the table lists the
 * same steps with the same actions for anyone who prefers a list.
 */
export function GraphView({
  model,
  order,
  flash,
  policy,
  selectedId,
  onSelect,
  onShowTable,
}: {
  model: RunModel;
  order: readonly string[];
  flash: { gen: number; ids: ReadonlySet<string> };
  policy: ViewPolicy;
  selectedId: string | undefined;
  onSelect: (id: string | undefined) => void;
  onShowTable: () => void;
}) {
  const after = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLDivElement>(null);
  const positions = useMemo(() => layoutGraph(layoutNodes(order, policy.detail), model.edges, 'LR'), [order, model.edges, policy.detail]);
  const nodes = useMemo<CardNode[]>(
    () => flowNodes({ nodes: model.nodes, order, positions, detail: policy.detail, selectedId, flash }),
    [model.nodes, order, positions, policy.detail, selectedId, flash],
  );
  const edges = useMemo(() => flowEdges(model.edges, model.nodes, policy.edgeLabels), [model.edges, model.nodes, policy.edgeLabels]);

  // The selection is the page's: a click or Enter on a card asks for it, nothing is selected inside the canvas.
  const onNodesChange = (changes: NodeChange<CardNode>[]) => {
    for (const change of changes) if (change.type === 'select' && change.selected) onSelect(change.id);
  };

  return (
    <div className="rn-graph">
      <div className="toolbar">
        <span>
          {model.nodes.size} steps, {model.edges.length} links.
          {policy.detail === 'simple' ? ' Large graph: simplified cards.' : ''}
        </span>
        <button type="button" className="btn" onClick={onShowTable}>
          Open the table view
        </button>
        <button type="button" className="btn" onClick={() => after.current?.focus()}>
          Skip the graph
        </button>
      </div>
      <ul className="rn-legend" aria-label="How links are drawn">
        <li>
          <span className="rn-swatch taken" aria-hidden="true" /> Solid: the run went this way
        </li>
        <li>
          <span className="rn-swatch pending" aria-hidden="true" /> Dashed: not decided yet
        </li>
        <li>
          <span className="rn-swatch omitted" aria-hidden="true" /> Dotted: omitted
        </li>
        <li>Dashed card: an undo or a lookup the platform added</li>
      </ul>
      <div ref={canvas} className="rn-canvas" role="group" aria-label="Graph of the run">
        <ReactFlow<CardNode>
          nodes={nodes}
          edges={edges}
          nodeTypes={NODE_TYPES}
          onNodesChange={onNodesChange}
          onNodeClick={(_, node) => onSelect(node.id)}
          onPaneClick={() => onSelect(undefined)}
          nodesDraggable={false}
          nodesConnectable={false}
          edgesFocusable={false}
          elementsSelectable
          deleteKeyCode={null}
          fitView
          fitViewOptions={{ padding: 0.1, duration: 0 }}
          minZoom={0.02}
          maxZoom={1.5}
          onlyRenderVisibleElements
          ariaLabelConfig={ARIA}
          colorMode="system"
        >
          <InitialViewport canvas={canvas} detail={policy.detail} />
          <Controls showInteractive={false} />
          {policy.minimap ? <MiniMap pannable zoomable nodeColor={overviewColor} style={{ width: 150, height: 96 }} /> : null}
        </ReactFlow>
      </div>
      <div ref={after} tabIndex={-1} className="rn-after" aria-label="After the graph" role="group">
        {policy.minimap ? null : <span className="hint">The overview map is left out of large graphs.</span>}
      </div>
    </div>
  );
}
