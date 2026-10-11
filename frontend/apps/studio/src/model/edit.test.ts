import { describe, expect, it } from 'vitest';
import { EditError, addNode, connect, disconnect, orphans, removeNode, renameNode, setEntrypoint, setField, uniqueId, wouldCreateCycle } from './edit';
import { emptyDocument, findNode, readDocument } from './ir';
import type { IrDocument } from './ir';
import { advise } from './advice';
import { danglingReferences } from './references';
import { linksOf } from './registry';

function flow(): IrDocument {
  return {
    ...emptyDocument('order-flow'),
    entrypoint: 'check',
    nodes: [
      { id: 'check', type: 'condition', expression: 'input.amount > 10', ifTrue: 'pay', ifFalse: 'done' },
      { id: 'pay', type: 'activity', activity: 'payment.submit', next: 'both', onError: 'done' },
      { id: 'both', type: 'parallel', join: 'all', branches: ['a', 'b'], next: 'done' },
      { id: 'a', type: 'activity', activity: 'a.run' },
      { id: 'b', type: 'activity', activity: 'b.run' },
      { id: 'done', type: 'end' },
    ],
  };
}

const refs = (doc: IrDocument) => doc.nodes.flatMap(linksOf).map((l) => `${l.from}.${l.field}${l.index === undefined ? '' : `[${l.index}]`}->${l.to}`);

describe('addNode', () => {
  it('adds a node of a kind with a fresh id, and makes the first one the entrypoint', () => {
    const first = addNode(emptyDocument('x-flow'), 'activity');
    expect(first.nodes).toEqual([{ id: 'activity_1', type: 'activity', activity: '' }]);
    expect(first.entrypoint).toBe('activity_1');
    const second = addNode(first, 'activity');
    expect(second.nodes.map((n) => n.id)).toEqual(['activity_1', 'activity_2']);
    expect(second.entrypoint).toBe('activity_1');
  });

  it('does not change the document it was given', () => {
    const doc = flow();
    const before = JSON.stringify(doc);
    addNode(doc, 'timer');
    removeNode(doc, 'pay');
    renameNode(doc, 'pay', 'charge');
    connect(doc, 'a', 'next', 'b');
    expect(JSON.stringify(doc)).toBe(before);
  });

  it('knows the kinds of the schema and refuses others', () => {
    expect(() => addNode(emptyDocument('x-flow'), 'teleport')).toThrow(EditError);
    for (const type of ['activity', 'script', 'condition', 'parallel', 'timer', 'waitEvent', 'child', 'saga', 'end', 'foreach', 'humanApproval']) {
      expect(addNode(emptyDocument('x-flow'), type).nodes[0]?.type).toBe(type);
    }
  });

  it('picks an id that is free, with the hint when it is', () => {
    const doc = flow();
    expect(uniqueId(doc, 'pay')).toBe('pay_2');
    expect(uniqueId(doc, 'refund')).toBe('refund');
    expect(uniqueId(doc, 'a b!')).toBe('a_b_');
    expect(uniqueId(doc, '')).toBe('node');
    expect(addNode(doc, 'end', 'done').nodes.at(-1)?.id).toBe('done_2');
  });
});

describe('removeNode', () => {
  it('takes the node out and every link that led to it', () => {
    const { doc, cleared } = removeNode(flow(), 'a');
    expect(findNode(doc, 'a')).toBeUndefined();
    expect(findNode(doc, 'both')?.['branches']).toEqual(['b']);
    expect(cleared.map((l) => `${l.from}.${l.field}`)).toEqual(['both.branches']);
  });

  it('clears a single link by removing the field, and moves the entrypoint', () => {
    const { doc } = removeNode(flow(), 'check');
    expect(doc.entrypoint).toBe('pay');
    const afterPay = removeNode(flow(), 'pay').doc;
    expect(findNode(afterPay, 'check')?.['ifTrue']).toBeUndefined();
    expect(refs(afterPay)).not.toContain('check.ifTrue->pay');
  });

  it('leaves an empty entrypoint when the last node goes', () => {
    const one = addNode(emptyDocument('x-flow'), 'end');
    expect(removeNode(one, 'end_1').doc.entrypoint).toBe('');
  });
});

describe('renameNode', () => {
  it('follows every reference and the entrypoint', () => {
    const doc = renameNode(flow(), 'check', 'decide');
    expect(doc.entrypoint).toBe('decide');
    const doc2 = renameNode(doc, 'done', 'finish');
    expect(refs(doc2).filter((r) => r.endsWith('->done'))).toEqual([]);
    expect(refs(doc2)).toContain('decide.ifFalse->finish');
    expect(refs(doc2)).toContain('pay.onError->finish');
    expect(refs(doc2)).toContain('both.next->finish');
  });

  it('refuses an id that is taken, malformed or reserved', () => {
    for (const bad of ['pay', '', 'a b', 'x/y', '~c', 'pay~c', '-x', 'a'.repeat(65), 'é']) {
      expect(() => renameNode(flow(), 'check', bad)).toThrow(EditError);
    }
    expect(() => renameNode(flow(), 'nope', 'x')).toThrow(EditError);
    expect(renameNode(flow(), 'check', 'check').nodes).toEqual(flow().nodes);
  });
});

describe('connect and disconnect', () => {
  it('sets the field that holds the link, and replaces what was there', () => {
    const doc = connect(flow(), 'a', 'next', 'done');
    expect(findNode(doc, 'a')?.next).toBe('done');
    expect(findNode(connect(doc, 'a', 'next', 'b'), 'a')?.next).toBe('b');
    expect(findNode(connect(flow(), 'check', 'ifFalse', 'a'), 'check')?.['ifFalse']).toBe('a');
  });

  it('adds to a list without repeating', () => {
    const doc = connect(flow(), 'both', 'branches', 'done');
    expect(findNode(doc, 'both')?.['branches']).toEqual(['a', 'b', 'done']);
    expect(() => connect(doc, 'both', 'branches', 'done')).toThrow(/already/);
    expect(findNode(connect(flow(), 'both', 'branches', 'done', 0), 'both')?.['branches']).toEqual(['done', 'a', 'b']);
  });

  it('refuses a link that would make a cycle, a link to itself, to nothing, or from a field the node does not have', () => {
    expect(() => connect(flow(), 'done', 'next', 'check')).toThrow(EditError); // end has no way out
    expect(() => connect(flow(), 'a', 'next', 'check')).toThrow(/cycle/);
    expect(() => connect(flow(), 'a', 'next', 'a')).toThrow(/itself/);
    expect(() => connect(flow(), 'a', 'next', 'ghost')).toThrow(/no node/);
    expect(() => connect(flow(), 'ghost', 'next', 'a')).toThrow(/no node/);
    expect(() => connect(flow(), 'a', 'ifTrue', 'b')).toThrow(EditError);
    try {
      connect(flow(), 'a', 'next', 'check');
    } catch (error) {
      expect((error as EditError).code).toBe('CYCLE');
    }
  });

  it('knows which links close a cycle', () => {
    expect(wouldCreateCycle(flow(), 'a', 'check')).toBe(true);
    expect(wouldCreateCycle(flow(), 'a', 'b')).toBe(false);
    expect(wouldCreateCycle(flow(), 'check', 'check')).toBe(true);
  });

  it('removes a link by field, or one entry of a list', () => {
    expect(findNode(disconnect(flow(), 'pay', 'next'), 'pay')?.next).toBeUndefined();
    expect(findNode(disconnect(flow(), 'both', 'branches', 0), 'both')?.['branches']).toEqual(['b']);
    expect(disconnect(flow(), 'a', 'next').nodes).toEqual(flow().nodes);
  });
});

describe('setField, setEntrypoint, orphans', () => {
  it('sets a field and removes it when the value is gone', () => {
    const doc = setField(flow(), 'pay', 'timeoutMs', 5000);
    expect(findNode(doc, 'pay')?.['timeoutMs']).toBe(5000);
    for (const empty of [undefined, '', null]) {
      expect(findNode(setField(doc, 'pay', 'timeoutMs', empty), 'pay')).not.toHaveProperty('timeoutMs');
    }
  });

  it('does not let a field edit rewire the graph or rename a node behind the editor\'s back', () => {
    expect(() => setField(flow(), 'pay', 'id', 'other')).toThrow(EditError);
    expect(() => setField(flow(), 'pay', 'type', 'end')).toThrow(EditError);
    expect(() => setField(flow(), 'pay', 'next', 'a')).toThrow(EditError);
    expect(() => setField(flow(), 'both', 'branches', ['a'])).toThrow(EditError);
    expect(() => setField(flow(), 'ghost', 'x', 1)).toThrow(/no node/);
  });

  it('sets the entrypoint to a node that exists', () => {
    expect(setEntrypoint(flow(), 'pay').entrypoint).toBe('pay');
    expect(() => setEntrypoint(flow(), 'ghost')).toThrow(EditError);
  });

  it('lists the nodes nothing reaches from the entrypoint', () => {
    expect(orphans(flow())).toEqual([]);
    const doc = addNode(flow(), 'timer', 'lonely');
    expect(orphans(doc)).toEqual(['lonely']);
    expect(orphans(disconnect(flow(), 'check', 'ifTrue'))).toEqual(['pay', 'both', 'a', 'b']);
  });
});

describe('readDocument', () => {
  it('keeps what it does not understand and says which nodes it could not read', () => {
    const { doc, unreadable } = readDocument({
      apiVersion: 'x',
      metadata: { name: 'x-flow', owner: 'team' },
      extra: { kept: true },
      nodes: [{ id: 'a', type: 'end' }, { type: 'end' }, 'junk', { id: 'a', type: 'end' }, { id: 5, type: 'end' }],
    });
    expect(doc.nodes.map((n) => n.id)).toEqual(['a']);
    expect(unreadable).toEqual(['/nodes/1', '/nodes/2', '/nodes/3', '/nodes/4']);
    expect(doc['extra']).toEqual({ kept: true });
    expect(doc.metadata['owner']).toBe('team');
  });

  it('opens anything', () => {
    for (const value of [null, undefined, 3, 'x', [], {}]) {
      expect(readDocument(value).doc.nodes).toEqual([]);
    }
  });
});

describe('expressions that read the output of a step follow its rename and are told of its removal', () => {
  const doc: IrDocument = {
    apiVersion: 'sagat-octopus-orchestrator.io/v1',
    kind: 'Workflow',
    metadata: { name: 'reads' },
    entrypoint: 'charge',
    nodes: [
      { id: 'charge', type: 'activity', activity: 'pay.charge', idempotencyTemplate: 'k:${steps.charge.output.id}', next: 'check' },
      { id: 'check', type: 'condition', expression: 'steps.charge.output.ok && steps["other-step"].output.ok', ifTrue: 'done', ifFalse: 'done' },
      { id: 'done', type: 'end', outputExpr: 'steps.charge.output' },
    ],
  };

  it('renames the reads in every expression field, with the links', () => {
    const renamed = renameNode(doc, 'charge', 'payment');
    expect(renamed.nodes.map((n) => n.id)).toEqual(['payment', 'check', 'done']);
    expect(renamed.nodes[0]?.['idempotencyTemplate']).toBe('k:${steps.payment.output.id}');
    expect(renamed.nodes[1]?.['expression']).toBe('steps.payment.output.ok && steps["other-step"].output.ok');
    expect(renamed.nodes[2]?.['outputExpr']).toBe('steps.payment.output');
    expect(danglingReferences(renamed.nodes).map((d) => d.id)).toEqual(['other-step']);
  });

  it('uses the index form when the new id is not a name', () => {
    const renamed = renameNode(doc, 'charge', 'charge-card');
    expect(renamed.nodes[2]?.['outputExpr']).toBe('steps["charge-card"].output');
  });

  it('says which expressions still read a step that was removed, and the advice names them', () => {
    const removed = removeNode(doc, 'charge');
    expect(removed.dangling).toEqual([
      { nodeId: 'check', field: 'expression' },
      { nodeId: 'done', field: 'outputExpr' },
    ]);
    const advice = advise(removed.doc).filter((a) => a.code === 'UNKNOWN_STEP_REFERENCE');
    expect(advice.map((a) => a.nodeId)).toEqual(['check', 'check', 'done']);
    expect(advice[0]?.message).toContain('there is no step with that id');
  });
});
