import { graphlib, layout } from '@dagrejs/dagre';

export interface Point {
  x: number;
  y: number;
}

export interface LayoutNode {
  id: string;
  width: number;
  height: number;
}

export interface LayoutEdge {
  from: string;
  to: string;
}

export type Direction = 'LR' | 'TB';

/** Above this many nodes the careful layout is too slow to be interactive (D147). */
export const LAYERED_ABOVE = 1500;

const GAP_X = 64;
const GAP_Y = 32;

/**
 * Where to draw each node (top-left corner). Positions are computed, not stored: the document
 * stays what the platform stores, a refresh does not move anything, and two people see the same
 * picture (D147, G148).
 */
export function layoutGraph(nodes: readonly LayoutNode[], edges: readonly LayoutEdge[], direction: Direction): Map<string, Point> {
  if (nodes.length > LAYERED_ABOVE) return layeredLayout(nodes, edges, direction);
  const graph = new graphlib.Graph({ multigraph: false, compound: false });
  graph.setGraph({ rankdir: direction, nodesep: GAP_Y, ranksep: GAP_X, marginx: 16, marginy: 16 });
  graph.setDefaultEdgeLabel(() => ({}));
  const known = new Set<string>();
  for (const node of nodes) {
    graph.setNode(node.id, { width: node.width, height: node.height });
    known.add(node.id);
  }
  for (const edge of edges) {
    if (known.has(edge.from) && known.has(edge.to) && edge.from !== edge.to) graph.setEdge(edge.from, edge.to);
  }
  layout(graph);
  const out = new Map<string, Point>();
  for (const node of nodes) {
    const placed = graph.node(node.id) as Point;
    out.set(node.id, { x: Math.round(placed.x - node.width / 2), y: Math.round(placed.y - node.height / 2) });
  }
  return out;
}

/**
 * A cheap layered layout: a node goes in the column of the longest path that reaches it, and in
 * the order it came in within it. Linear in the size of the graph, and it tolerates a cycle (the
 * nodes of one are placed after whatever reached them first).
 */
export function layeredLayout(nodes: readonly LayoutNode[], edges: readonly LayoutEdge[], direction: Direction): Map<string, Point> {
  const index = new Map(nodes.map((n, i) => [n.id, i]));
  const out: number[][] = nodes.map(() => []);
  const indegree = nodes.map(() => 0);
  for (const edge of edges) {
    const from = index.get(edge.from);
    const to = index.get(edge.to);
    if (from === undefined || to === undefined || from === to) continue;
    (out[from] as number[]).push(to);
    indegree[to] = (indegree[to] as number) + 1;
  }
  const rank = nodes.map(() => 0);
  const queue: number[] = [];
  nodes.forEach((_, i) => {
    if (indegree[i] === 0) queue.push(i);
  });
  const placed = new Set<number>();
  let head = 0;
  const drain = () => {
    while (head < queue.length) {
      const at = queue[head++] as number;
      placed.add(at);
      for (const to of out[at] as number[]) {
        // A link back to a node already placed closes a cycle: it must not push that node along.
        if (placed.has(to)) continue;
        rank[to] = Math.max(rank[to] as number, (rank[at] as number) + 1);
        indegree[to] = (indegree[to] as number) - 1;
        if (indegree[to] === 0) queue.push(to);
      }
    }
  };
  drain();
  // What is left sits on a cycle: start each from the first one not placed yet.
  for (let i = 0; i < nodes.length; i++) {
    if (placed.has(i)) continue;
    indegree[i] = 0;
    queue.push(i);
    drain();
  }
  const rows = new Map<number, number>();
  const result = new Map<string, Point>();
  const width = Math.max(...nodes.map((n) => n.width), 1);
  const height = Math.max(...nodes.map((n) => n.height), 1);
  nodes.forEach((node, i) => {
    const column = rank[i] as number;
    const row = rows.get(column) ?? 0;
    rows.set(column, row + 1);
    result.set(node.id, direction === 'LR' ? { x: column * (width + GAP_X), y: row * (height + GAP_Y) } : { x: row * (width + GAP_Y), y: column * (height + GAP_X) });
  });
  return result;
}
