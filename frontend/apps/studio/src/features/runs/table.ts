import type { RuntimeNode } from '../../api/types';
import { isAttention } from './display';

export type SortKey = 'status' | 'id';

export interface TableQuery {
  sort: SortKey;
  descending: boolean;
  /** Only what failed, waits for a person or is about to be retried (`needsAttention`). */
  attentionOnly: boolean;
  /** Part of a node id, in any case. */
  text: string;
}

export const DEFAULT_QUERY: TableQuery = { sort: 'status', descending: false, attentionOnly: false, text: '' };

const collator = new Intl.Collator('en', { numeric: true });

/**
 * The ids of the nodes in a reader's order (`step-2` before `step-10`). Sorting is the expensive
 * part of showing 50 000 rows, so it is done once per graph and the table only filters and
 * groups this order.
 */
export function idOrder(nodes: ReadonlyMap<string, RuntimeNode>): string[] {
  return [...nodes.keys()].sort(collator.compare);
}

/** Where a node sits when sorted by status: what needs a person first, what is done last. */
export function statusRank(node: RuntimeNode): number {
  if (node.waitingOn === 'operator') return 0;
  switch (node.status) {
    case 'FAILED':
      return 1;
    case 'RETRY_WAIT':
      return 2;
    case 'RUNNING':
    case 'LEASED':
      return 3;
    case 'WAITING':
      return 4;
    case 'READY':
    case 'PENDING':
      return 5;
    case 'NOT_STARTED':
      return 6;
    case 'SUCCEEDED':
      return 7;
    case 'CANCELLED':
      return 8;
    case 'SKIPPED':
      return 9;
    default:
      return 10;
  }
}

const RANKS = 11;

/**
 * The rows of the table: every node of the run (or those that pass the filters), in the order
 * asked for. Linear in the number of nodes given the order of ids, so a delta of the stream
 * re-sorts 50 000 nodes without a pause.
 */
export function tableNodes(nodes: ReadonlyMap<string, RuntimeNode>, order: readonly string[], query: TableQuery): RuntimeNode[] {
  const ids = order.length === nodes.size ? order : idOrder(nodes);
  const needle = query.text.trim().toLowerCase();
  const kept: RuntimeNode[] = [];
  for (const id of ids) {
    const node = nodes.get(id);
    if (node === undefined) continue;
    if (query.attentionOnly && !isAttention(node)) continue;
    if (needle !== '' && !id.toLowerCase().includes(needle)) continue;
    kept.push(node);
  }
  let rows = kept;
  if (query.sort === 'status') {
    // A stable grouping by rank over the id order: no comparison sort.
    const buckets: RuntimeNode[][] = Array.from({ length: RANKS }, () => []);
    for (const node of kept) (buckets[statusRank(node)] as RuntimeNode[]).push(node);
    rows = buckets.flat();
  }
  return query.descending ? rows.reverse() : rows;
}
