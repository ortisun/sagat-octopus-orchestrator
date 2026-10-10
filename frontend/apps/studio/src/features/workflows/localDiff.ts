import type { DefinitionDiff } from '../../api/types';
import { canonical } from '../../model/ir';
import type { IrDocument } from '../../model/ir';
import { linksOf } from '../../model/registry';

/**
 * A node-level comparison of two documents, in the shape of the platform's diff, for the one place
 * the platform cannot be asked: two versions of a draft that cannot both be saved. It does not
 * classify changes as breaking (the platform's rules for that live on the server), so `breaking` and
 * `attention` are empty and the caller must not present that as "nothing breaks".
 */
export function compareDocuments(from: IrDocument, to: IrDocument): Pick<DefinitionDiff, 'identical' | 'nodes' | 'edges' | 'changes'> {
  const before = new Map(from.nodes.map((n) => [n.id, n]));
  const after = new Map(to.nodes.map((n) => [n.id, n]));
  const added = [...after.keys()].filter((id) => !before.has(id));
  const removed = [...before.keys()].filter((id) => !after.has(id));
  const changed: { id: string; fields: string[] }[] = [];
  for (const [id, node] of after) {
    const old = before.get(id);
    if (!old) continue;
    const fields = [...new Set([...Object.keys(old), ...Object.keys(node)])].filter((key) => canonical(old[key]) !== canonical(node[key])).sort();
    if (fields.length > 0) changed.push({ id, fields });
  }

  const edges = (doc: IrDocument) => new Map(doc.nodes.flatMap((n) => linksOf(n)).map((l) => [`${l.from}\u0000${l.kind}\u0000${l.to}`, { from: l.from, to: l.to, kind: l.kind }]));
  const e0 = edges(from);
  const e1 = edges(to);

  const changes: { path: string; change: 'added' | 'removed' | 'changed' }[] = [];
  const keys = [...new Set([...Object.keys(from), ...Object.keys(to)])].filter((k) => k !== 'nodes').sort();
  for (const key of keys) {
    const a = from[key];
    const b = to[key];
    if (a === undefined && b !== undefined) changes.push({ path: `/${key}`, change: 'added' });
    else if (a !== undefined && b === undefined) changes.push({ path: `/${key}`, change: 'removed' });
    else if (canonical(a) !== canonical(b)) changes.push({ path: `/${key}`, change: 'changed' });
  }

  return {
    identical: added.length === 0 && removed.length === 0 && changed.length === 0 && changes.length === 0,
    nodes: { added, removed, changed },
    edges: {
      added: [...e1].filter(([key]) => !e0.has(key)).map(([, edge]) => edge),
      removed: [...e0].filter(([key]) => !e1.has(key)).map(([, edge]) => edge),
    },
    changes,
  };
}
