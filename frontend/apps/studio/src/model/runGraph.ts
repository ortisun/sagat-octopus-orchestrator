import type { GraphChanges, GraphEvent, RuntimeEdge, RuntimeGraph, RuntimeNode } from '../api/types';

/** The run as the Studio holds it: the graph, and the sequence it is at. */
export interface RunModel {
  runId: string;
  status: string;
  projectionSeq: number;
  chain: RuntimeGraph['chain'];
  nodes: ReadonlyMap<string, RuntimeNode>;
  edges: readonly RuntimeEdge[];
  /** The last events seen in changes, newest last (bounded). */
  recent: readonly GraphEvent[];
}

const RECENT = 500;

export function modelFromGraph(graph: RuntimeGraph): RunModel {
  return {
    runId: graph.runId,
    status: graph.status ?? 'RUNNING',
    projectionSeq: graph.projectionSeq,
    chain: graph.chain ?? null,
    nodes: new Map(graph.nodes.map((n) => [n.id, n])),
    edges: graph.edges,
    recent: [],
  };
}

export type Applied = { kind: 'applied'; model: RunModel } | { kind: 'stale'; model: RunModel } | { kind: 'gap'; model: RunModel | undefined };

/**
 * Applies what the platform says changed. A change is relative to a sequence: if the Studio is
 * behind it, a link is missing (`gap`: load the graph again); if the Studio is already at or past
 * it, it is a repeat and changes nothing (`stale`); a `reset` replaces everything.
 */
export function applyChanges(model: RunModel | undefined, changes: GraphChanges): Applied {
  if (changes.reset) {
    return {
      kind: 'applied',
      model: {
        runId: changes.runId,
        status: changes.status,
        projectionSeq: changes.projectionSeq,
        chain: changes.chain ?? null,
        nodes: new Map(changes.nodes.map((n) => [n.id, n])),
        edges: changes.edges ?? [],
        recent: [],
      },
    };
  }
  if (model === undefined || model.runId !== changes.runId) return { kind: 'gap', model };
  if (changes.projectionSeq <= model.projectionSeq) return { kind: 'stale', model };
  if (changes.fromSeq > model.projectionSeq) return { kind: 'gap', model };
  const nodes = new Map(model.nodes);
  for (const node of changes.nodes) nodes.set(node.id, node);
  const recent = [...model.recent, ...changes.events].slice(-RECENT);
  return { kind: 'applied', model: { ...model, status: changes.status, projectionSeq: changes.projectionSeq, chain: changes.chain === undefined ? model.chain : changes.chain, nodes, recent } };
}

const FINISHED = new Set(['SUCCEEDED', 'FAILED', 'CANCELLED', 'SKIPPED']);

export type EdgeState =
  /** The run went this way. */
  | 'taken'
  /** The step before it has not finished: the run may still go this way. */
  | 'pending'
  /** The step before it finished and the run did not go this way (the other side of a condition, a branch skipped). */
  | 'not-taken';

/**
 * Whether the run went along an edge, told from the states of its two ends: the run is the
 * journal's projection, and an edge that no event names is read, not recorded.
 */
export function edgeState(edge: RuntimeEdge, nodes: ReadonlyMap<string, RuntimeNode>): EdgeState {
  const from = nodes.get(edge.from);
  const to = nodes.get(edge.to);
  if (from === undefined || to === undefined) return 'pending';
  const started = to.status !== 'NOT_STARTED' && to.status !== 'SKIPPED';
  if (edge.kind === 'compensation' || edge.kind === 'reconcile') return started ? 'taken' : from.status === 'NOT_STARTED' ? 'pending' : 'not-taken';
  if (edge.kind === 'error') return started ? 'taken' : FINISHED.has(from.status) ? 'not-taken' : 'pending';
  if (started) return 'taken';
  return FINISHED.has(from.status) ? 'not-taken' : 'pending';
}

/** Nodes in the order a reader wants them: the ones that need attention first. */
export function needsAttention(model: RunModel): RuntimeNode[] {
  return [...model.nodes.values()].filter((n) => n.status === 'FAILED' || n.waitingOn === 'operator' || n.status === 'RETRY_WAIT');
}
