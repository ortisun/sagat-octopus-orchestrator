import { addNode, connect, setField } from '../../model/edit';
import { findNode } from '../../model/ir';
import type { IrDocument, IrNode, IrTrigger } from '../../model/ir';
import { kindOf } from '../../model/registry';
import type { HandleSpec } from '../../model/registry';

export interface Inserted {
  doc: IrDocument;
  /** The id of the new node. */
  id: string;
  /** What happened, in words for the live region. */
  note: string;
}

const EXCEPTIONAL = new Set(['error', 'timeout']);

function isEmpty(node: IrNode, handle: HandleSpec): boolean {
  const value = node[handle.field];
  return handle.multiple ? !Array.isArray(value) || value.length === 0 : typeof value !== 'string' || value === '';
}

/**
 * Adds a node of a kind. With `after`, the new node goes where that node leads next: into its first
 * free way out (never the error or timeout path), or between it and what it led to, so adding a step
 * after a step lengthens the chain instead of starting a second one. One call is one edit to undo.
 */
export function insertNode(doc: IrDocument, type: string, after?: string): Inserted {
  const added = addNode(doc, type);
  const created = added.nodes[added.nodes.length - 1];
  if (!created) throw new Error('addNode returned no node');
  const label = kindOf(type)?.label ?? type;
  const plain: Inserted = { doc: added, id: created.id, note: `Added ${label} "${created.id}".` };
  const source = after === undefined ? undefined : findNode(added, after);
  if (!source) return plain;

  const handles = kindOf(source.type)?.handles ?? [];
  const free = handles.find((h) => !EXCEPTIONAL.has(h.edge) && isEmpty(source, h));
  if (free) {
    return { doc: connect(added, source.id, free.field, created.id), id: created.id, note: `Added ${label} "${created.id}" after "${source.id}" (${free.label}).` };
  }
  const next = handles.find((h) => h.field === 'next');
  if (next && !next.multiple) {
    const old = source[next.field];
    const takesNext = (kindOf(created.type)?.handles ?? []).some((h) => h.field === 'next');
    if (typeof old === 'string' && old !== '' && takesNext) {
      const between = connect(connect(added, source.id, next.field, created.id), created.id, 'next', old);
      return { doc: between, id: created.id, note: `Added ${label} "${created.id}" between "${source.id}" and "${old}".` };
    }
    return {
      ...plain,
      note: `Added ${label} "${created.id}". "${source.id}" already leads to "${typeof old === 'string' ? old : '?'}", so it is not connected: link it where you want it.`,
    };
  }
  const multiple = handles.find((h) => h.multiple);
  if (multiple) {
    return { doc: connect(added, source.id, multiple.field, created.id), id: created.id, note: `Added ${label} "${created.id}" after "${source.id}" (${multiple.label}).` };
  }
  return { ...plain, note: `Added ${label} "${created.id}". "${source.id}" has no way out to link it from, so it is not connected.` };
}

/** The handle of a node by its field, for the words of a link. */
export function handleOf(node: IrNode, field: string): HandleSpec | undefined {
  return (kindOf(node.type)?.handles ?? []).find((h) => h.field === field);
}

// ---------------------------------------------------------------- triggers

export const TRIGGER_KINDS = ['manual', 'event', 'webhook', 'message', 'cron'] as const;
export type TriggerKind = (typeof TRIGGER_KINDS)[number];

export interface TriggerField {
  name: string;
  label: string;
  required: boolean;
  hint?: string;
}

/** The fields of each kind of trigger, from the schema. */
export const TRIGGER_FIELDS: Record<TriggerKind, readonly TriggerField[]> = {
  manual: [],
  event: [
    { name: 'eventType', label: 'Event type', required: true, hint: 'For example order.created.' },
    { name: 'source', label: 'Source', required: false, hint: 'Only events published by this endpoint, connector or api.' },
    { name: 'filter', label: 'Filter', required: false, hint: 'A condition over event and data; the event starts a run only when it holds.' },
  ],
  webhook: [
    { name: 'endpoint', label: 'Endpoint', required: true, hint: 'The slug of the webhook endpoint that receives the events.' },
    { name: 'eventType', label: 'Event type', required: false, hint: 'Narrows the events that start a run.' },
    { name: 'filter', label: 'Filter', required: false },
  ],
  message: [
    { name: 'connector', label: 'Connector', required: true, hint: 'The slug of the connector that consumes the messages.' },
    { name: 'eventType', label: 'Event type', required: false, hint: 'Narrows the events that start a run.' },
    { name: 'filter', label: 'Filter', required: false },
  ],
  cron: [
    { name: 'cron', label: 'Schedule', required: true, hint: 'Five fields: minute hour day-of-month month day-of-week.' },
    { name: 'timezone', label: 'Time zone', required: false, hint: 'An IANA name such as Europe/Lisbon. UTC when empty.' },
  ],
};

export function triggerKindOf(trigger: IrTrigger): TriggerKind | undefined {
  return TRIGGER_KINDS.find((k) => k === trigger.kind);
}

function omit<T extends object>(value: T, key: string): T {
  return Object.fromEntries(Object.entries(value).filter(([k]) => k !== key)) as T;
}

function withTriggers(doc: IrDocument, triggers: IrTrigger[]): IrDocument {
  const rest = omit(doc, 'triggers');
  return triggers.length === 0 ? rest : { ...rest, triggers };
}

export function addTrigger(doc: IrDocument, kind: TriggerKind): IrDocument {
  return withTriggers(doc, [...(doc.triggers ?? []), { kind }]);
}

export function removeTrigger(doc: IrDocument, index: number): IrDocument {
  return withTriggers(
    doc,
    (doc.triggers ?? []).filter((_, i) => i !== index),
  );
}

/** Changing the kind keeps the fields the new kind also has and drops the rest. */
export function setTriggerKind(doc: IrDocument, index: number, kind: TriggerKind): IrDocument {
  const keep = new Set(TRIGGER_FIELDS[kind].map((f) => f.name));
  return withTriggers(
    doc,
    (doc.triggers ?? []).map((t, i) => (i === index ? { kind, ...Object.fromEntries(Object.entries(t).filter(([key]) => keep.has(key))) } : t)),
  );
}

export function setTriggerField(doc: IrDocument, index: number, field: string, value: string): IrDocument {
  return withTriggers(
    doc,
    (doc.triggers ?? []).map((t, i) => {
      if (i !== index) return t;
      if (value === '') return omit(t, field);
      return { ...t, [field]: value };
    }),
  );
}

// ---------------------------------------------------------------- document

export function setDescription(doc: IrDocument, description: string): IrDocument {
  const rest = omit(doc.metadata, 'description');
  return { ...doc, metadata: description === '' ? rest : { ...rest, description } };
}

export function setSchema(doc: IrDocument, which: 'inputSchema' | 'outputSchema', schema: Record<string, unknown> | undefined): IrDocument {
  const rest = omit(doc, which);
  return schema === undefined ? rest : { ...rest, [which]: schema };
}

/** The retry policy of a node with a field set or cleared; an empty policy is no policy. */
export function setRetryField(doc: IrDocument, id: string, field: string, value: unknown): IrDocument {
  const node = findNode(doc, id);
  const current = node && typeof node['retry'] === 'object' && node['retry'] !== null && !Array.isArray(node['retry']) ? (node['retry'] as Record<string, unknown>) : {};
  const next = Object.fromEntries(Object.entries({ ...current, [field]: value }).filter(([, v]) => v !== undefined && v !== '' && v !== null));
  return setField(doc, id, 'retry', Object.keys(next).length === 0 ? undefined : next);
}

/** A name for a copy of a workflow: the name and `-copy`, then `-copy-2`…, within the 64 characters a name may have. */
export function copyName(name: string, attempt: number): string {
  const suffix = attempt === 0 ? '-copy' : `-copy-${attempt + 1}`;
  const base = name.slice(0, 64 - suffix.length).replace(/-+$/, '');
  return /^[a-z]/.test(base) ? `${base}${suffix}` : `workflow${suffix}`;
}
