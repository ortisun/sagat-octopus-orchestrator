import { describe, expect, it } from 'vitest';
import type { GraphChanges, RuntimeEdge, RuntimeGraph, RuntimeNode } from '../api/types';
import { eventLabel, eventTone } from './events';
import { applyChanges, edgeState, modelFromGraph, needsAttention } from './runGraph';

const RUN = '019b5d6e-0000-7000-8000-000000000001';
const node = (id: string, status: RuntimeNode['status'], over: Partial<RuntimeNode> = {}): RuntimeNode => ({ id, type: 'activity', status, attempt: 0, ...over });
const graph = (seq: number, nodes: RuntimeNode[], edges: RuntimeEdge[] = []): RuntimeGraph => ({ runId: RUN, status: 'RUNNING', projectionSeq: seq, nodes, edges });
const change = (from: number, to: number, nodes: RuntimeNode[], over: Partial<GraphChanges> = {}): GraphChanges => ({ runId: RUN, status: 'RUNNING', fromSeq: from, projectionSeq: to, reset: false, events: [], nodes, ...over });

describe('applyChanges', () => {
  const base = () => modelFromGraph(graph(5, [node('a', 'SUCCEEDED'), node('b', 'READY'), node('c', 'NOT_STARTED')]));

  it('replaces the nodes that came and keeps the rest', () => {
    const result = applyChanges(base(), change(5, 8, [node('b', 'RUNNING'), node('c', 'READY')], { status: 'WAITING', events: [{ seq: 8, eventType: 'step.started', occurredAt: '2026-10-10T10:00:00Z', stepId: 'b' }] }));
    expect(result.kind).toBe('applied');
    if (result.kind !== 'applied') return;
    expect([...result.model.nodes.values()].map((n) => `${n.id}:${n.status}`)).toEqual(['a:SUCCEEDED', 'b:RUNNING', 'c:READY']);
    expect([result.model.projectionSeq, result.model.status, result.model.recent.length]).toEqual([8, 'WAITING', 1]);
  });

  it('does not change the model it was given', () => {
    const model = base();
    applyChanges(model, change(5, 6, [node('b', 'RUNNING')]));
    expect(model.nodes.get('b')?.status).toBe('READY');
    expect(model.projectionSeq).toBe(5);
  });

  it('takes a repeat for what it is', () => {
    expect(applyChanges(base(), change(3, 5, [node('b', 'RUNNING')])).kind).toBe('stale');
    expect(applyChanges(base(), change(2, 4, [node('b', 'RUNNING')])).kind).toBe('stale');
  });

  it('asks to load the graph again when a link is missing or the run is another', () => {
    expect(applyChanges(base(), change(6, 9, [])).kind).toBe('gap');
    expect(applyChanges(undefined, change(0, 3, [])).kind).toBe('gap');
    expect(applyChanges(base(), { ...change(5, 6, []), runId: 'other' }).kind).toBe('gap');
  });

  it('replaces everything on a reset', () => {
    const result = applyChanges(base(), change(0, 40, [node('z', 'SUCCEEDED')], { reset: true, edges: [{ from: 'z', to: 'z', kind: 'next' }], status: 'SUCCEEDED' }));
    expect(result.kind).toBe('applied');
    if (result.kind !== 'applied') return;
    expect([...result.model.nodes.keys()]).toEqual(['z']);
    expect(result.model.edges).toHaveLength(1);
    expect(result.model.projectionSeq).toBe(40);
    // A reset also starts a model from nothing.
    expect(applyChanges(undefined, change(0, 40, [node('z', 'SUCCEEDED')], { reset: true })).kind).toBe('applied');
  });

  it('remembers only the last events', () => {
    let model = base();
    for (let seq = 6; seq < 1200; seq++) {
      const result = applyChanges(model, change(seq - 1, seq, [], { events: [{ seq, eventType: 'step.started', occurredAt: '2026-10-10T10:00:00Z' }] }));
      if (result.kind === 'applied') model = result.model;
    }
    expect(model.recent).toHaveLength(500);
    expect(model.recent.at(-1)?.seq).toBe(1199);
  });
});

describe('edgeState', () => {
  const nodes = new Map<string, RuntimeNode>(
    [node('check', 'SUCCEEDED', { type: 'condition' }), node('pay', 'RUNNING'), node('skip', 'NOT_STARTED'), node('later', 'NOT_STARTED'), node('wait', 'WAITING'), node('fix', 'READY')].map((n) => [n.id, n]),
  );
  it('tells the way taken from the way not taken and the way not decided yet', () => {
    expect(edgeState({ from: 'check', to: 'pay', kind: 'true' }, nodes)).toBe('taken');
    expect(edgeState({ from: 'check', to: 'skip', kind: 'false' }, nodes)).toBe('not-taken');
    expect(edgeState({ from: 'pay', to: 'later', kind: 'next' }, nodes)).toBe('pending');
    expect(edgeState({ from: 'wait', to: 'later', kind: 'timeout' }, nodes)).toBe('pending');
    expect(edgeState({ from: 'check', to: 'fix', kind: 'error' }, nodes)).toBe('taken');
    expect(edgeState({ from: 'ghost', to: 'pay', kind: 'next' }, nodes)).toBe('pending');
  });
});

describe('needsAttention', () => {
  it('lists the steps a person should look at', () => {
    const model = modelFromGraph(graph(1, [node('a', 'SUCCEEDED'), node('b', 'FAILED'), node('c~c', 'WAITING', { waitingOn: 'operator' }), node('d', 'RETRY_WAIT')]));
    expect(needsAttention(model).map((n) => n.id)).toEqual(['b', 'c~c', 'd']);
  });
});

describe('events', () => {
  it('says every event the platform writes in words, and an unknown one as it is', () => {
    expect(eventLabel('step.retry_scheduled')).toMatch(/retry/);
    expect(eventLabel('something.new')).toBe('something.new');
    expect(eventTone('step.failed')).toBe('problem');
    expect(eventTone('step.retry_requested')).toBe('decision');
    expect(eventTone('step.completed')).toBe('normal');
  });
});
