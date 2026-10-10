import { readDocument } from '../../model/ir';
import type { IrDocument } from '../../model/ir';

/** A draft as the editor holds it: the document it edits, and what it could not read and so does not touch. */
export interface OpenedDraft {
  doc: IrDocument;
  /** Entries of `nodes` that are not nodes the Studio understands (no id, no type, an id used twice), as they were. */
  extras: readonly unknown[];
  /** JSON pointers of those entries in the draft as it was read. */
  unreadable: readonly string[];
}

export function openDraft(ir: unknown): OpenedDraft {
  const { doc, unreadable } = readDocument(ir);
  const raw = typeof ir === 'object' && ir !== null && Array.isArray((ir as Record<string, unknown>)['nodes']) ? ((ir as Record<string, unknown>)['nodes'] as unknown[]) : [];
  const extras = unreadable.flatMap((pointer) => {
    const index = Number(pointer.slice('/nodes/'.length));
    return Number.isInteger(index) && index < raw.length ? [raw[index]] : [];
  });
  return { doc, extras, unreadable };
}

/**
 * What is sent to the platform: the document, and after its nodes the entries the Studio left alone,
 * so that saving never silently drops something somebody else put in the draft. The platform's own
 * validation reports them.
 */
export function toWire(doc: IrDocument, extras: readonly unknown[]): Record<string, unknown> {
  return extras.length === 0 ? doc : { ...doc, nodes: [...doc.nodes, ...extras] };
}
