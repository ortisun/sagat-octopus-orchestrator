import { describe, expect, it } from 'vitest';
import { advise } from './advice';
import { emptyDocument } from './ir';
import type { IrDocument } from './ir';

function saga(): IrDocument {
  return {
    ...emptyDocument('pay-flow'),
    entrypoint: 'tx',
    nodes: [
      { id: 'tx', type: 'saga', begin: 'reserve', next: 'done' },
      { id: 'reserve', type: 'activity', activity: 'limit.reserve', compensateWith: 'limit.release', timeoutMs: 5000, next: 'submit' },
      { id: 'submit', type: 'activity', activity: 'payment.submit', timeoutMs: 5000, next: 'ship' },
      { id: 'ship', type: 'activity', activity: 'goods.ship', timeoutMs: 5000, irreversible: true },
      { id: 'done', type: 'end' },
    ],
  };
}

const codes = (doc: IrDocument) => advise(doc).map((a) => `${a.code}:${a.nodeId}`);

describe('advise', () => {
  it('has nothing to say about a document that is as careful as it can be', () => {
    const doc = saga();
    doc.nodes[2] = { id: 'submit', type: 'activity', activity: 'payment.submit', timeoutMs: 5000, compensateWith: 'payment.return', next: 'ship' };
    expect(codes(doc)).toEqual(['IRREVERSIBLE_IN_SAGA:ship']);
  });

  it('points at a step of a saga that nothing can undo', () => {
    expect(codes(saga())).toContain('NO_COMPENSATION:submit');
    expect(codes(saga())).not.toContain('NO_COMPENSATION:reserve');
    expect(codes(saga())).not.toContain('NO_COMPENSATION:ship');
  });

  it('asks for a timeout on what talks to a worker, a script or a child', () => {
    const doc = saga();
    doc.nodes[1] = { id: 'reserve', type: 'activity', activity: 'limit.reserve', compensateWith: 'limit.release', next: 'submit' };
    expect(codes(doc)).toContain('NO_TIMEOUT:reserve');
    const other: IrDocument = { ...emptyDocument('x-flow'), entrypoint: 'c', nodes: [{ id: 'c', type: 'child', workflowId: 'other-flow', next: 's' }, { id: 's', type: 'script', artifactDigest: 'sha256:x' }] };
    expect(codes(other)).toEqual(['NO_TIMEOUT:c', 'NO_TIMEOUT:s']);
  });

  it('warns about a very wide fork, and about steps nothing reaches', () => {
    const wide: IrDocument = {
      ...emptyDocument('x-flow'),
      entrypoint: 'p',
      nodes: [
        { id: 'p', type: 'parallel', join: 'all', branches: Array.from({ length: 40 }, (_, i) => `b${i}`) },
        ...Array.from({ length: 40 }, (_, i) => ({ id: `b${i}`, type: 'end' })),
        { id: 'lonely', type: 'end' },
      ],
    };
    expect(codes(wide)).toEqual(['WIDE_FORK:p', 'UNREACHABLE:lonely']);
  });

  it('says when nothing but a person can start the workflow', () => {
    const doc = saga();
    doc.triggers = [];
    expect(advise(doc).find((a) => a.code === 'NO_TRIGGER')).toMatchObject({ nodeId: '' });
  });

  it('is advice, not a verdict: every item says what to do', () => {
    for (const item of advise(saga())) {
      expect(item.message.length).toBeGreaterThan(20);
      expect(item.severity).toBe('advice');
    }
  });
});
