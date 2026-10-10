import { describe, expect, it } from 'vitest';
import { addNode } from '../../model/edit';
import { openDraft, toWire } from './documentIO';

const raw = {
  apiVersion: 'sagat-octopus-orchestrator.io/v1',
  kind: 'Workflow',
  metadata: { name: 'order-flow' },
  entrypoint: 'a',
  customThing: { kept: true },
  nodes: [{ id: 'a', type: 'end' }, { type: 'end' }, { id: 'a', type: 'end', note: 'twin' }, 'nonsense'],
};

describe('opening a draft', () => {
  it('keeps what it cannot read and puts it back after the nodes it can', () => {
    const opened = openDraft(raw);
    expect(opened.doc.nodes).toHaveLength(1);
    expect(opened.unreadable).toEqual(['/nodes/1', '/nodes/2', '/nodes/3']);
    expect(opened.extras).toEqual([{ type: 'end' }, { id: 'a', type: 'end', note: 'twin' }, 'nonsense']);
    const wire = toWire(opened.doc, opened.extras);
    expect((wire['nodes'] as unknown[]).slice(1)).toEqual(opened.extras);
    expect(wire['customThing']).toEqual({ kept: true });
  });

  it('adds nothing to a draft it can read entirely', () => {
    const opened = openDraft({ ...raw, nodes: [{ id: 'a', type: 'end' }] });
    expect(opened.extras).toEqual([]);
    expect(toWire(opened.doc, opened.extras)).toBe(opened.doc);
  });

  it('survives a draft that is not a document at all', () => {
    const opened = openDraft(null);
    expect(opened.doc.nodes).toEqual([]);
    expect(opened.extras).toEqual([]);
  });

  it('puts the extras after the nodes the author adds', () => {
    const opened = openDraft(raw);
    const wire = toWire(addNode(opened.doc, 'timer', 't1'), opened.extras);
    expect((wire['nodes'] as { id?: string }[]).map((n) => n.id)).toEqual(['a', 't1', undefined, 'a', undefined]);
  });
});
