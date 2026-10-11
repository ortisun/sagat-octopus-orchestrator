import { NODE_ID } from './ir';
import type { IrDocument, IrNode } from './ir';
import { danglingReferences, expressionFields, renameStepReference } from './references';
import { NODE_KINDS, kindOf, linksOf } from './registry';
import type { Link } from './registry';

export type EditErrorCode = 'UNKNOWN_KIND' | 'UNKNOWN_NODE' | 'BAD_ID' | 'ID_TAKEN' | 'NOT_A_HANDLE' | 'SELF' | 'CYCLE' | 'DUPLICATE' | 'RESERVED_FIELD';

/** Why an edit was refused, in words for the person who tried it. */
export class EditError extends Error {
  readonly code: EditErrorCode;
  constructor(code: EditErrorCode, message: string) {
    super(message);
    this.name = 'EditError';
    this.code = code;
  }
}

function node(doc: IrDocument, id: string): IrNode {
  const found = doc.nodes.find((n) => n.id === id);
  if (!found) throw new EditError('UNKNOWN_NODE', `There is no node "${id}".`);
  return found;
}

function replace(doc: IrDocument, id: string, change: (n: IrNode) => IrNode): IrDocument {
  return { ...doc, nodes: doc.nodes.map((n) => (n.id === id ? change(n) : n)) };
}

/** An id that is free, as close to `hint` as the rules of ids allow. */
export function uniqueId(doc: IrDocument, hint: string): string {
  let base = hint.replace(/[^A-Za-z0-9_.-]/g, '_').slice(0, 56);
  if (base === '') base = 'node';
  if (!/^[A-Za-z0-9]/.test(base)) base = `n${base}`;
  const taken = new Set(doc.nodes.map((n) => n.id));
  if (!taken.has(base)) return base;
  for (let n = 2; ; n++) {
    const candidate = `${base}_${n}`;
    if (!taken.has(candidate)) return candidate;
  }
}

function defaultId(doc: IrDocument, type: string): string {
  const taken = new Set(doc.nodes.map((n) => n.id));
  for (let n = 1; ; n++) {
    const candidate = `${type}_${n}`;
    if (!taken.has(candidate)) return candidate;
  }
}

/** Adds a node of a kind. The first node of a document is its entrypoint. */
export function addNode(doc: IrDocument, type: string, id?: string): IrDocument {
  const spec = kindOf(type);
  if (!spec) throw new EditError('UNKNOWN_KIND', `"${type}" is not a kind of node. The kinds are ${NODE_KINDS.map((k) => k.type).join(', ')}.`);
  const chosen = id === undefined ? defaultId(doc, type) : uniqueId(doc, id);
  return { ...doc, entrypoint: doc.entrypoint === '' || !doc.nodes.some((n) => n.id === doc.entrypoint) ? chosen : doc.entrypoint, nodes: [...doc.nodes, spec.make(chosen)] };
}

/**
 * Takes a node out, and every link that led to it. Returns the links it cleared, and the
 * expressions that still read the output of the step (`steps.<id>`), which the Studio cannot
 * rewrite into anything and says so.
 */
export function removeNode(doc: IrDocument, id: string): { doc: IrDocument; cleared: Link[]; dangling: Array<{ nodeId: string; field: string }> } {
  node(doc, id);
  const cleared: Link[] = [];
  const nodes: IrNode[] = [];
  for (const n of doc.nodes) {
    if (n.id === id) continue;
    let next = n;
    for (const link of linksOf(n)) {
      if (link.to !== id) continue;
      cleared.push(link);
      next = unlink(next, link.field, link.index === undefined ? undefined : link.to);
    }
    nodes.push(next);
  }
  const entrypoint = doc.entrypoint === id ? (nodes[0]?.id ?? '') : doc.entrypoint;
  const dangling = danglingReferences(nodes)
    .filter((reference) => reference.id === id)
    .map(({ nodeId, field }) => ({ nodeId, field }));
  return { doc: { ...doc, entrypoint, nodes }, cleared, dangling };
}

/** The node without one of its properties. */
function without(n: IrNode, field: string): IrNode {
  return Object.fromEntries(Object.entries(n).filter(([key]) => key !== field)) as IrNode;
}

/** Removes the link held by `field` (for a list, the entry equal to `target`). */
function unlink(n: IrNode, field: string, target: string | undefined): IrNode {
  const value = n[field];
  if (Array.isArray(value) && target !== undefined) {
    const index = value.indexOf(target);
    return index === -1 ? n : { ...n, [field]: value.filter((_, i) => i !== index) };
  }
  return without(n, field);
}

/** Renames a node and follows every reference to it. */
export function renameNode(doc: IrDocument, from: string, to: string): IrDocument {
  node(doc, from);
  if (from === to) return doc;
  if (!NODE_ID.test(to)) throw new EditError('BAD_ID', 'An id is 1 to 64 letters, digits, dots, dashes or underscores, and starts with a letter or a digit.');
  if (doc.nodes.some((n) => n.id === to)) throw new EditError('ID_TAKEN', `There is already a node "${to}".`);
  const rename = (value: unknown): unknown => (value === from ? to : value);
  const nodes = doc.nodes.map((n) => {
    let next: IrNode = n.id === from ? { ...n, id: to } : n;
    for (const link of linksOf(n)) {
      if (link.to !== from) continue;
      const value = next[link.field];
      next = { ...next, [link.field]: Array.isArray(value) ? value.map(rename) : to };
    }
    // The expressions that read the output of the step read the output of the renamed one.
    for (const { field, template, value } of expressionFields(next)) {
      const renamed = renameStepReference(value, from, to, template);
      if (renamed !== value) next = { ...next, [field]: renamed };
    }
    return next;
  });
  return { ...doc, entrypoint: doc.entrypoint === from ? to : doc.entrypoint, nodes };
}

/** Whether a link from `from` to `to` would close a cycle (the engine runs no cycle). */
export function wouldCreateCycle(doc: IrDocument, from: string, to: string): boolean {
  if (from === to) return true;
  const byId = new Map(doc.nodes.map((n) => [n.id, n]));
  const seen = new Set<string>();
  const stack = [to];
  while (stack.length > 0) {
    const id = stack.pop() as string;
    if (id === from) return true;
    if (seen.has(id)) continue;
    seen.add(id);
    const current = byId.get(id);
    if (current) for (const link of linksOf(current)) stack.push(link.to);
  }
  return false;
}

/** Makes `from` lead to `to` through `field` (a single link replaces the one it had). */
export function connect(doc: IrDocument, from: string, field: string, to: string, index?: number): IrDocument {
  const source = node(doc, from);
  node(doc, to);
  const handle = (kindOf(source.type)?.handles ?? []).find((h) => h.field === field);
  if (!handle) throw new EditError('NOT_A_HANDLE', `A ${kindOf(source.type)?.label ?? source.type} node has no "${field}" link.`);
  if (from === to) throw new EditError('SELF', 'A node cannot lead to itself.');
  if (wouldCreateCycle(doc, from, to)) throw new EditError('CYCLE', `That link would make a cycle: "${to}" already leads back to "${from}".`);
  if (handle.multiple) {
    const list = Array.isArray(source[field]) ? (source[field] as string[]) : [];
    if (list.includes(to)) throw new EditError('DUPLICATE', `"${to}" is already one of the ${handle.label.toLowerCase()}es of "${from}".`);
    const next = index === undefined ? [...list, to] : [...list.slice(0, index), to, ...list.slice(index)];
    return replace(doc, from, (n) => ({ ...n, [field]: next }));
  }
  return replace(doc, from, (n) => ({ ...n, [field]: to }));
}

/** Removes the link held by `field`; for a list, the entry at `index`. */
export function disconnect(doc: IrDocument, from: string, field: string, index?: number): IrDocument {
  const source = node(doc, from);
  const value = source[field];
  if (Array.isArray(value)) {
    if (index === undefined) return replace(doc, from, (n) => unlink(n, field, undefined));
    return replace(doc, from, (n) => ({ ...n, [field]: (n[field] as unknown[]).filter((_, i) => i !== index) }));
  }
  if (value === undefined) return doc;
  return replace(doc, from, (n) => unlink(n, field, undefined));
}

const RESERVED = new Set(['id', 'type']);

/** Sets a property of a node; an empty value removes it. Links go through [`connect`]. */
export function setField(doc: IrDocument, id: string, field: string, value: unknown): IrDocument {
  const source = node(doc, id);
  if (RESERVED.has(field)) throw new EditError('RESERVED_FIELD', `"${field}" is changed by renaming the node or adding another one.`);
  const handles = kindOf(source.type)?.handles ?? [];
  if (field === 'next' || field === 'onError' || handles.some((h) => h.field === field)) {
    throw new EditError('RESERVED_FIELD', `"${field}" is a link: draw it or remove it on the canvas.`);
  }
  return replace(doc, id, (n) => {
    if (value === undefined || value === null || value === '') return without(n, field);
    return { ...n, [field]: value };
  });
}

export function setEntrypoint(doc: IrDocument, id: string): IrDocument {
  node(doc, id);
  return { ...doc, entrypoint: id };
}

/** The nodes that nothing reaches from the entrypoint, in document order. */
export function orphans(doc: IrDocument): string[] {
  const byId = new Map(doc.nodes.map((n) => [n.id, n]));
  const seen = new Set<string>();
  const stack = [doc.entrypoint];
  while (stack.length > 0) {
    const id = stack.pop() as string;
    if (seen.has(id) || !byId.has(id)) continue;
    seen.add(id);
    for (const link of linksOf(byId.get(id) as IrNode)) stack.push(link.to);
  }
  return doc.nodes.filter((n) => !seen.has(n.id)).map((n) => n.id);
}
