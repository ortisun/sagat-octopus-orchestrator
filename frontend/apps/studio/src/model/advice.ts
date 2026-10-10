import type { IrDocument, IrNode } from './ir';
import { linksOf } from './registry';
import { orphans } from './edit';

/**
 * What the platform does not check and a careful author would want to know. Advice is computed
 * in the browser, shown apart from the platform's problems, and never stops a save or a publish.
 */
export interface Advice {
  code: 'NO_TIMEOUT' | 'NO_COMPENSATION' | 'IRREVERSIBLE_IN_SAGA' | 'WIDE_FORK' | 'UNREACHABLE' | 'NO_TRIGGER';
  /** The node it is about, empty for the document. */
  nodeId: string;
  severity: 'advice';
  message: string;
}

const WIDE_FORK = 32;

function reachable(doc: IrDocument, start: string): Set<string> {
  const byId = new Map(doc.nodes.map((n) => [n.id, n]));
  const seen = new Set<string>();
  const stack = [start];
  while (stack.length > 0) {
    const id = stack.pop() as string;
    if (seen.has(id) || !byId.has(id)) continue;
    seen.add(id);
    for (const link of linksOf(byId.get(id) as IrNode)) stack.push(link.to);
  }
  return seen;
}

export function advise(doc: IrDocument): Advice[] {
  const out: Advice[] = [];
  const add = (code: Advice['code'], nodeId: string, message: string) => out.push({ code, nodeId, severity: 'advice', message });

  if ((doc.triggers ?? []).length === 0) {
    add('NO_TRIGGER', '', 'The workflow has no trigger: runs can only be started through the API. Add a manual, event, webhook, message or cron trigger.');
  }

  // The body of a saga is everything reachable from where it begins; the platform keeps it closed.
  const inSaga = new Set<string>();
  for (const node of doc.nodes) {
    if (node.type === 'saga' && typeof node['begin'] === 'string') for (const id of reachable(doc, node['begin'])) inSaga.add(id);
  }

  for (const node of doc.nodes) {
    const talksToAWorker = node.type === 'activity' || node.type === 'script' || node.type === 'child';
    if (talksToAWorker && node['timeoutMs'] === undefined) {
      add('NO_TIMEOUT', node.id, `"${node.id}" has no timeout: if a worker hangs, the step waits until its lease is lost. Set the longest one attempt may take.`);
    }
    if (inSaga.has(node.id) && (node.type === 'activity' || node.type === 'script')) {
      if (node['irreversible'] === true) {
        add('IRREVERSIBLE_IN_SAGA', node.id, `"${node.id}" cannot be undone: if a later step of the saga fails, the saga ends partly undone and a person is asked to decide.`);
      } else if (node['compensateWith'] === undefined || node['compensateWith'] === '') {
        add('NO_COMPENSATION', node.id, `"${node.id}" is inside a saga but nothing undoes it. Name the activity that does, or mark the step as one that cannot be undone.`);
      }
    }
    if (node.type === 'parallel' && Array.isArray(node['branches']) && node['branches'].length > WIDE_FORK) {
      add('WIDE_FORK', node.id, `"${node.id}" starts ${node['branches'].length} branches at once. Make sure the workers can take that many, or split the fork.`);
    }
  }

  for (const id of orphans(doc)) {
    add('UNREACHABLE', id, `"${id}" is not reachable from the first step: nothing will ever run it. Connect it, or remove it.`);
  }
  return out;
}
