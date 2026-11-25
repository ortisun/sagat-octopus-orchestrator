import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { IrDocument, IrNode } from '../../model/ir';
import { canonical, readDocument } from '../../model/ir';
import { WORKFLOW, addPolicy, limitCeiling, mayDeclareGates, policyOf, policyProblems, readPolicy, removePolicy, setPolicyField } from './flowControl';
import type { Gate, Member, Owner } from './flowControl';

// Feature 022 (FR-022-035, FR-022-050): the Studio writes the same IR as the six SDKs. The shared fixtures are the
// documents the SDKs' own tests build byte for byte; here the Studio builds them member by member, in an order that is
// not the IR's, and must end on the same text.

const FIXTURES = `${process.cwd()}/../../../tests/contract/flow-control-step-policies/fixtures`;
const fixture = (name: string): IrDocument => readDocument(JSON.parse(readFileSync(`${FIXTURES}/${name}`, 'utf8'))).doc;

const GATES: readonly Gate[] = ['concurrency', 'rateLimit'];
/** The members in the reverse of the canonical order, so a writer that kept the typing order would fail. */
const TYPING_ORDER: readonly Member[] = ['queueTimeoutMs', 'per', 'limit', 'key', 'group'];

/** The fixture without any gate, and the list of what was taken out. */
function stripped(doc: IrDocument): { bare: IrDocument; gates: Array<{ owner: Owner; gate: Gate; policy: Record<string, unknown> }> } {
  const gates: Array<{ owner: Owner; gate: Gate; policy: Record<string, unknown> }> = [];
  const top: Record<string, unknown> = { ...doc };
  for (const gate of GATES) {
    if (top[gate] !== undefined) {
      gates.push({ owner: WORKFLOW, gate, policy: top[gate] as Record<string, unknown> });
      delete top[gate];
    }
  }
  const nodes = doc.nodes.map((node) => {
    const rest: IrNode = { ...node };
    for (const gate of GATES) {
      if (rest[gate] !== undefined) {
        gates.push({ owner: { level: 'node', nodeId: node.id }, gate, policy: rest[gate] as Record<string, unknown> });
        delete rest[gate];
      }
    }
    return rest;
  });
  return { bare: { ...(top as IrDocument), nodes }, gates };
}

describe('the Studio writes the shared flow-control fixtures', () => {
  for (const name of ['provider-sync.workflow.json', 'account-serial.workflow.json']) {
    it(`builds ${name} member by member and ends on the fixture's text`, () => {
      const expected = fixture(name);
      const { bare, gates } = stripped(expected);
      expect(gates.length).toBeGreaterThan(0);
      let doc = bare;
      for (const { owner, gate, policy } of gates) {
        doc = addPolicy(doc, owner, gate);
        for (const member of TYPING_ORDER) {
          if (policy[member] !== undefined) doc = setPolicyField(doc, owner, gate, member, policy[member]);
        }
      }
      expect(canonical(doc)).toBe(canonical(expected));
      for (const { owner, gate, policy } of gates) {
        // Byte for byte: the policy's own text, members in the canonical (alphabetical) order the fixtures use.
        expect(JSON.stringify(policyOf(doc, owner, gate))).toBe(JSON.stringify(policy));
        expect(policyProblems(gate, policy)).toEqual([]);
      }
    });
  }

  it('reads back what it wrote and removes a gate entirely', () => {
    const doc = fixture('account-serial.workflow.json');
    const node = { level: 'node', nodeId: 'notify' } as const;
    expect(readPolicy(policyOf(doc, node, 'concurrency'))).toEqual({ state: 'policy', policy: { key: 'steps.price.output.region', limit: 2 }, unknownMembers: [] });
    const without = removePolicy(removePolicy(doc, node, 'concurrency'), WORKFLOW, 'concurrency');
    expect('concurrency' in without).toBe(false);
    expect('concurrency' in (without.nodes.find((n) => n.id === 'notify') ?? {})).toBe(false);
    expect(readPolicy(policyOf(without, WORKFLOW, 'concurrency'))).toEqual({ state: 'absent' });
  });

  it('turning a gate on writes an empty policy and invents no member (decision D022-150 item 1)', () => {
    const doc = addPolicy(fixture('account-serial.workflow.json'), { level: 'node', nodeId: 'price' }, 'rateLimit');
    expect(doc.nodes.find((n) => n.id === 'price')?.['rateLimit']).toEqual({});
    expect(policyProblems('rateLimit', {})).toEqual([
      { member: 'limit', text: 'Enter the most grants per window.' },
      { member: 'per', text: 'Choose a window: a second, a minute or an hour.' },
    ]);
    expect(policyProblems('concurrency', {})).toEqual([{ member: 'limit', text: 'Enter the most holders at once.' }]);
  });

  it('clearing a member removes it, and a member it does not know is kept after the known ones', () => {
    const owner = WORKFLOW;
    let doc: IrDocument = { ...fixture('account-serial.workflow.json'), concurrency: { burst: 5, limit: 3, group: 'g' } };
    doc = setPolicyField(doc, owner, 'concurrency', 'group', undefined);
    doc = setPolicyField(doc, owner, 'concurrency', 'key', 'input.accountId');
    expect(JSON.stringify(doc['concurrency'])).toBe('{"key":"input.accountId","limit":3,"burst":5}');
    expect(readPolicy(doc['concurrency'])).toEqual({ state: 'policy', policy: { key: 'input.accountId', limit: 3, burst: 5 }, unknownMembers: ['burst'] });
  });

  it('a gate value that is not an object is unreadable and stays as it is', () => {
    for (const value of ['serial', 3, [1], null, true]) expect(readPolicy(value)).toEqual({ state: 'unreadable' });
    const doc: IrDocument = { ...fixture('account-serial.workflow.json'), rateLimit: 'fast' };
    expect(setPolicyField(doc, WORKFLOW, 'rateLimit', 'limit', 3)['rateLimit']).toBe('fast');
  });
});

describe('the checks beside the fields are the platform bounds and nothing else (decision D022-150 item 2)', () => {
  const limitOf = (gate: Gate, policy: Record<string, unknown>) => policyProblems(gate, policy).filter((p) => p.member === 'limit');

  it('a concurrency limit is a whole number from 1 to 10,000', () => {
    expect(limitOf('concurrency', { limit: 1 })).toEqual([]);
    expect(limitOf('concurrency', { limit: 10_000 })).toEqual([]);
    expect(limitOf('concurrency', { limit: 0 })).toEqual([{ member: 'limit', text: 'Enter a whole number from 1 to 10000.' }]);
    expect(limitOf('concurrency', { limit: 10_001 })).toEqual([{ member: 'limit', text: 'Enter a whole number from 1 to 10000.' }]);
    expect(limitOf('concurrency', { limit: 2.5 })).toEqual([{ member: 'limit', text: 'Enter a whole number from 1 to 10000.' }]);
    expect(limitOf('concurrency', { limit: '2' })).toEqual([{ member: 'limit', text: 'Enter a whole number from 1 to 10000.' }]);
  });

  it('a rate limit is bounded by the ceiling of its window: 100 a second, 6,000 a minute, 10,000 an hour', () => {
    expect([limitCeiling('rateLimit', 'second'), limitCeiling('rateLimit', 'minute'), limitCeiling('rateLimit', 'hour')]).toEqual([100, 6_000, 10_000]);
    expect(limitOf('rateLimit', { limit: 100, per: 'second' })).toEqual([]);
    expect(limitOf('rateLimit', { limit: 101, per: 'second' })).toEqual([{ member: 'limit', text: 'Enter a whole number from 1 to 100 (the most a second allows).' }]);
    expect(limitOf('rateLimit', { limit: 6_000, per: 'minute' })).toEqual([]);
    expect(limitOf('rateLimit', { limit: 6_001, per: 'minute' })).toEqual([{ member: 'limit', text: 'Enter a whole number from 1 to 6000 (the most a minute allows).' }]);
    expect(limitOf('rateLimit', { limit: 10_000, per: 'hour' })).toEqual([]);
    expect(limitOf('rateLimit', { limit: 10_001, per: 'hour' })).toEqual([{ member: 'limit', text: 'Enter a whole number from 1 to 10000 (the most an hour allows).' }]);
  });

  it('a window is second, minute or hour; another is unsupported', () => {
    expect(policyProblems('rateLimit', { limit: 1, per: 'day' })).toEqual([{ member: 'per', text: 'The window "day" is not one the platform accepts: choose a second, a minute or an hour.' }]);
  });

  it('a group follows the pattern of an explicit group name', () => {
    const groupOf = (group: unknown) => policyProblems('concurrency', { limit: 1, group }).filter((p) => p.member === 'group');
    for (const good of ['orders-by-account', 'a', 'scripts.a-b', `a${'b'.repeat(63)}`]) expect(groupOf(good)).toEqual([]);
    for (const bad of ['Not A Group', '1orders', '=order-flow', `a${'b'.repeat(64)}`, '', 7]) {
      expect(groupOf(bad)).toEqual([{ member: 'group', text: 'A group starts with a lowercase letter and holds lowercase letters, digits, dots or dashes (up to 64).' }]);
    }
  });

  it('a queue timeout is a whole number of milliseconds from 1 to one day', () => {
    const timeoutOf = (queueTimeoutMs: unknown) => policyProblems('concurrency', { limit: 1, queueTimeoutMs }).filter((p) => p.member === 'queueTimeoutMs');
    expect(timeoutOf(1)).toEqual([]);
    expect(timeoutOf(86_400_000)).toEqual([]);
    for (const bad of [0, 86_400_001, 1.5, '600000']) {
      expect(timeoutOf(bad)).toEqual([{ member: 'queueTimeoutMs', text: 'A queue timeout is from 1 ms to one day (86400000 ms).' }]);
    }
  });

  it('a key is an expression of 1 to 1,024 bytes, counted in bytes', () => {
    const keyOf = (key: unknown) => policyProblems('rateLimit', { limit: 1, per: 'hour', key }).filter((p) => p.member === 'key');
    expect(keyOf('input.providerId')).toEqual([]);
    expect(keyOf('x'.repeat(1024))).toEqual([]);
    // 342 characters of three bytes each: 1,026 bytes.
    expect(keyOf('€'.repeat(342))).toEqual([{ member: 'key', text: 'A key is an expression of 1 to 1024 bytes.' }]);
    expect(keyOf('')).toEqual([{ member: 'key', text: 'A key is an expression of 1 to 1024 bytes.' }]);
  });
});

describe('where a gate may be declared', () => {
  it('on an activity or a script node, and on no other kind', () => {
    const node = (type: string): IrNode => ({ id: 'n', type });
    expect(mayDeclareGates(node('activity'))).toBe(true);
    expect(mayDeclareGates(node('script'))).toBe(true);
    for (const type of ['database', 'timer', 'condition', 'child', 'end', 'forEach', 'unknownKind']) expect(mayDeclareGates(node(type))).toBe(false);
  });
});
