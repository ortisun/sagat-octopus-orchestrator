import { describe, expect, it } from 'vitest';
import { EditError, addNode, connect } from '../../model/edit';
import { emptyDocument, findNode } from '../../model/ir';
import type { IrDocument } from '../../model/ir';
import { linksOf } from '../../model/registry';
import { addTrigger, copyName, insertNode, removeTrigger, setDescription, setRetryField, setSchema, setTriggerField, setTriggerKind } from './operations';

function chain(): IrDocument {
  let doc = emptyDocument('x-flow');
  doc = addNode(doc, 'activity', 'a');
  doc = addNode(doc, 'activity', 'b');
  doc = addNode(doc, 'end', 'z');
  doc = connect(doc, 'a', 'next', 'b');
  return connect(doc, 'b', 'next', 'z');
}

describe('insertNode', () => {
  it('adds a node alone when nothing is selected', () => {
    const out = insertNode(chain(), 'timer');
    expect(out.id).toBe('timer_1');
    expect(findNode(out.doc, 'timer_1')).toBeDefined();
    expect(out.note).toContain('Added Timer "timer_1"');
    expect(linksOf(findNode(out.doc, 'timer_1')!)).toEqual([]);
  });

  it('puts a new node in the first free way out of the selected one', () => {
    let doc = addNode(emptyDocument('x-flow'), 'activity', 'a');
    const out = insertNode(doc, 'timer', 'a');
    expect(findNode(out.doc, 'a')?.['next']).toBe(out.id);
    expect(out.note).toContain('after "a" (Next)');
    // A condition: its true branch first, then its false branch.
    doc = addNode(emptyDocument('x-flow'), 'condition', 'c');
    const first = insertNode(doc, 'end', 'c');
    expect(findNode(first.doc, 'c')?.['ifTrue']).toBe(first.id);
    const second = insertNode(first.doc, 'end', 'c');
    expect(findNode(second.doc, 'c')?.['ifFalse']).toBe(second.id);
  });

  it('puts the first node after a saga at its beginning, and after a parallel as a branch', () => {
    const saga = insertNode(addNode(emptyDocument('x-flow'), 'saga', 's'), 'activity', 's');
    expect(findNode(saga.doc, 's')?.['begin']).toBe(saga.id);
    const par = insertNode(addNode(emptyDocument('x-flow'), 'parallel', 'p'), 'activity', 'p');
    expect(findNode(par.doc, 'p')?.['branches']).toEqual([par.id]);
  });

  it('never takes the error path for "after"', () => {
    let doc = addNode(emptyDocument('x-flow'), 'activity', 'a');
    doc = addNode(doc, 'end', 'z');
    doc = connect(doc, 'a', 'next', 'z');
    const out = insertNode(doc, 'activity', 'a');
    expect(findNode(out.doc, 'a')?.['onError']).toBeUndefined();
  });

  it('lengthens the chain when the next link is taken: between the selected node and what it led to', () => {
    const out = insertNode(chain(), 'timer', 'a');
    expect(findNode(out.doc, 'a')?.['next']).toBe(out.id);
    expect(findNode(out.doc, out.id)?.['next']).toBe('b');
    expect(out.note).toContain('between "a" and "b"');
  });

  it('does not cut a chain for a node that cannot continue it', () => {
    const out = insertNode(chain(), 'end', 'a');
    expect(findNode(out.doc, 'a')?.['next']).toBe('b');
    expect(linksOf(findNode(out.doc, out.id)!)).toEqual([]);
    expect(out.note).toContain('not connected');
  });

  it('says so when the selected node has no way out at all', () => {
    const out = insertNode(chain(), 'timer', 'z');
    expect(findNode(out.doc, 'z')).toEqual({ id: 'z', type: 'end' });
    expect(out.note).toContain('no way out');
  });

  it('refuses a kind that does not exist', () => {
    expect(() => insertNode(chain(), 'teleport')).toThrow(EditError);
  });

  it('is one edit: the original document is untouched', () => {
    const doc = chain();
    const before = JSON.stringify(doc);
    insertNode(doc, 'timer', 'a');
    expect(JSON.stringify(doc)).toBe(before);
  });
});

describe('triggers', () => {
  it('adds, changes and removes triggers without touching the rest', () => {
    let doc = emptyDocument('x-flow');
    expect(doc.triggers).toEqual([{ kind: 'manual' }]);
    doc = addTrigger(doc, 'cron');
    doc = setTriggerField(doc, 1, 'cron', '0 9 * * 1');
    doc = setTriggerField(doc, 1, 'timezone', 'Europe/Lisbon');
    expect(doc.triggers?.[1]).toEqual({ kind: 'cron', cron: '0 9 * * 1', timezone: 'Europe/Lisbon' });
    doc = setTriggerField(doc, 1, 'timezone', '');
    expect(doc.triggers?.[1]).toEqual({ kind: 'cron', cron: '0 9 * * 1' });
    doc = removeTrigger(doc, 0);
    expect(doc.triggers).toEqual([{ kind: 'cron', cron: '0 9 * * 1' }]);
    doc = removeTrigger(doc, 0);
    expect('triggers' in doc).toBe(false);
  });

  it('keeps the fields a new kind also has and drops the others', () => {
    let doc = addTrigger(emptyDocument('x-flow'), 'event');
    doc = setTriggerField(doc, 1, 'eventType', 'order.created');
    doc = setTriggerField(doc, 1, 'source', 'api');
    doc = setTriggerKind(doc, 1, 'webhook');
    expect(doc.triggers?.[1]).toEqual({ kind: 'webhook', eventType: 'order.created' });
    doc = setTriggerKind(doc, 1, 'manual');
    expect(doc.triggers?.[1]).toEqual({ kind: 'manual' });
  });
});

describe('document settings', () => {
  it('sets and clears the description and the schemas', () => {
    let doc = setDescription(emptyDocument('x-flow'), 'Takes an order');
    expect(doc.metadata.description).toBe('Takes an order');
    doc = setDescription(doc, '');
    expect('description' in doc.metadata).toBe(false);
    doc = setSchema(doc, 'inputSchema', { type: 'object' });
    expect(doc.inputSchema).toEqual({ type: 'object' });
    doc = setSchema(doc, 'inputSchema', undefined);
    expect('inputSchema' in doc).toBe(false);
  });

  it('keeps a retry policy only while it says something', () => {
    let doc = addNode(emptyDocument('x-flow'), 'activity', 'a');
    doc = setRetryField(doc, 'a', 'maxAttempts', 5);
    doc = setRetryField(doc, 'a', 'strategy', 'exponential');
    expect(findNode(doc, 'a')?.['retry']).toEqual({ maxAttempts: 5, strategy: 'exponential' });
    doc = setRetryField(doc, 'a', 'maxAttempts', undefined);
    expect(findNode(doc, 'a')?.['retry']).toEqual({ strategy: 'exponential' });
    doc = setRetryField(doc, 'a', 'strategy', undefined);
    expect(findNode(doc, 'a')?.['retry']).toBeUndefined();
  });
});

describe('copyName', () => {
  it('adds -copy, then -copy-2, within the 64 characters a name may have', () => {
    expect(copyName('order-flow', 0)).toBe('order-flow-copy');
    expect(copyName('order-flow', 1)).toBe('order-flow-copy-2');
    const long = 'a'.repeat(64);
    for (let attempt = 0; attempt < 4; attempt++) {
      const name = copyName(long, attempt);
      expect(name.length).toBeLessThanOrEqual(64);
      expect(name).toMatch(/^[a-z][a-z0-9-]{2,63}$/);
    }
    expect(copyName('abc-', 0)).toBe('abc-copy');
  });
});
