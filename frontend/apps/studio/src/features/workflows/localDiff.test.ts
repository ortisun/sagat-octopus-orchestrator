import { describe, expect, it } from 'vitest';
import { addNode, connect, removeNode, setField } from '../../model/edit';
import { emptyDocument } from '../../model/ir';
import type { IrDocument } from '../../model/ir';
import { compareDocuments } from './localDiff';
import { setDescription } from './operations';

function base(): IrDocument {
  let doc = emptyDocument('x-flow');
  doc = addNode(doc, 'activity', 'a');
  doc = addNode(doc, 'activity', 'b');
  doc = addNode(doc, 'end', 'z');
  doc = connect(doc, 'a', 'next', 'b');
  return connect(doc, 'b', 'next', 'z');
}

describe('compareDocuments', () => {
  it('says two equal documents are identical, whatever the order of their keys', () => {
    const doc = base();
    expect(compareDocuments(doc, doc).identical).toBe(true);
    const reordered: IrDocument = { ...doc, nodes: doc.nodes.map((n) => Object.fromEntries(Object.entries(n).reverse()) as typeof n) };
    expect(compareDocuments(doc, reordered).identical).toBe(true);
  });

  it('lists nodes added, removed and changed, and which fields changed', () => {
    const from = base();
    let to = setField(from, 'a', 'activity', 'payment.submit');
    to = removeNode(to, 'z').doc;
    to = addNode(to, 'timer', 't');
    const diff = compareDocuments(from, to);
    expect(diff.identical).toBe(false);
    expect(diff.nodes.added).toEqual(['t']);
    expect(diff.nodes.removed).toEqual(['z']);
    expect(diff.nodes.changed).toContainEqual({ id: 'a', fields: ['activity'] });
    expect(diff.nodes.changed).toContainEqual({ id: 'b', fields: ['next'] });
  });

  it('lists the links added and removed', () => {
    const from = base();
    const to = connect(removeNode(from, 'b').doc, 'a', 'next', 'z');
    const diff = compareDocuments(from, to);
    expect(diff.edges.added).toEqual([{ from: 'a', to: 'z', kind: 'next' }]);
    expect(diff.edges.removed).toContainEqual({ from: 'a', to: 'b', kind: 'next' });
    expect(diff.edges.removed).toContainEqual({ from: 'b', to: 'z', kind: 'next' });
  });

  it('lists what changed outside the nodes', () => {
    const from = base();
    const to = setDescription({ ...from, entrypoint: 'b', triggers: undefined }, 'hello');
    const diff = compareDocuments(from, to);
    expect(diff.changes).toContainEqual({ path: '/entrypoint', change: 'changed' });
    expect(diff.changes).toContainEqual({ path: '/metadata', change: 'changed' });
    expect(diff.changes).toContainEqual({ path: '/triggers', change: 'removed' });
  });

  it('does not judge: it claims nothing about breaking changes', () => {
    const diff: Record<string, unknown> = compareDocuments(base(), setField(base(), 'a', 'activity', 'x'));
    expect('breaking' in diff).toBe(false);
    expect('attention' in diff).toBe(false);
  });
});
