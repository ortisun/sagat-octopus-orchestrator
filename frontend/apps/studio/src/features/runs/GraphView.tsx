import { Controls, MiniMap, ReactFlow } from '@xyflow/react';
import type { NodeChange } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { useMemo, useRef, useState } from 'react';
import type { RuntimeNode } from '../../api/types';
import { readableInitialViewport } from '../../model/initialViewport';
import { layoutGraph } from '../../model/layout';
import type { Point } from '../../model/layout';
import type { RunModel } from '../../model/runGraph';
import { flowEdges, flowNodes, layoutNodes } from './canvas';
import type { CardNode, ViewPolicy } from './canvas';
import { NODE_TYPES } from './RunNodeCard';

const ARIA = {
  'node.a11yDescription.default': 'Press Enter or Space to open the details of this step. The table view lists the same steps.',
  'node.a11yDescription.keyboardDisabled': 'The table view lists the same steps.',
  'edge.a11yDescription.default': 'A link between two steps.',
  'controls.ariaLabel': 'Graph view controls',
  'minimap.ariaLabel': 'Overview of the graph',
};

export function initialRunViewport(
  order: readonly string[],
  positions: ReadonlyMap<string, Point>,
  edges: readonly { from: string; to: string }[],
  nodes: ReadonlyMap<string, Pick<RuntimeNode, 'kind'>>,
) {
  const targets = new Set(edges.map((edge) => edge.to));
  const roots = order.filter((id) => positions.has(id) && !targets.has(id));
  const first =
    roots.find((id) => {
      const kind = nodes.get(id)?.kind;
      return kind === undefined || kind === 'FORWARD';
    }) ??
    roots[0] ??
    order.find((id) => positions.has(id));
  return readableInitialViewport(first === undefined ? undefined : positions.get(first));
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
 * the shape of the graph, so an update never moves anything, and the viewport is positioned only
 * when the canvas opens. Every step can be reached and chosen with the keyboard; the table lists
 * the same steps with the same actions for anyone who prefers a list.
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
  // The overview map sits over the corner of the graph, where a card may be: it is there when asked for (UX review 2026-10-14).
  const [overview, setOverview] = useState(false);
  const positions = useMemo(() => layoutGraph(layoutNodes(order, policy.detail), model.edges, 'LR'), [order, model.edges, policy.detail]);
  const initialViewport = useMemo(() => initialRunViewport(order, positions, model.edges, model.nodes), [order, positions, model.edges, model.nodes]);
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
        {policy.minimap ? (
          <button type="button" className="btn" aria-pressed={overview} onClick={() => setOverview((value) => !value)}>
            Overview map
          </button>
        ) : null}
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
      <div className="rn-canvas" role="group" aria-label="Graph of the run">
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
          defaultViewport={initialViewport}
          minZoom={0.02}
          maxZoom={1.5}
          onlyRenderVisibleElements
          ariaLabelConfig={ARIA}
          colorMode="system"
          attributionPosition="bottom-center"
        >
          <Controls showInteractive={false} />
          {policy.minimap && overview ? <MiniMap pannable zoomable nodeColor={overviewColor} style={{ width: 150, height: 96 }} /> : null}
        </ReactFlow>
      </div>
      <div ref={after} tabIndex={-1} className="rn-after" aria-label="After the graph" role="group">
        {policy.minimap ? null : <span className="hint">The overview map is left out of large graphs.</span>}
      </div>
    </div>
  );
}
