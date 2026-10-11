import { describe, expect, it } from 'vitest';
import type { RuntimeNode } from '../../api/types';
import { modelFromGraph, needsAttention } from '../../model/runGraph';
import { DEFAULT_QUERY, idOrder, statusRank, tableNodes } from './table';
import type { TableQuery } from './table';

const node = (id: string, status: RuntimeNode['status'], over: Partial<RuntimeNode> = {}): RuntimeNode => ({ id, type: 'activity', status, attempt: 0, ...over });
const mapOf = (nodes: RuntimeNode[]) => new Map(nodes.map((n) => [n.id, n]));
const q = (over: Partial<TableQuery> = {}): TableQuery => ({ ...DEFAULT_QUERY, ...over });

const SAMPLE = [
  node('step-10', 'SUCCEEDED'),
  node('step-2', 'FAILED', { failureCode: 'HTTP_500' }),
  node('step-1', 'RUNNING'),
  node('hold', 'WAITING', { waitingOn: 'operator' }),
  node('later', 'NOT_STARTED'),
  node('retrying', 'RETRY_WAIT'),
  node('skipped', 'SKIPPED'),
];

describe('idOrder', () => {
  it('puts step-2 before step-10', () => {
    expect(idOrder(mapOf(SAMPLE))).toEqual(['hold', 'later', 'retrying', 'skipped', 'step-1', 'step-2', 'step-10']);
  });
});

describe('tableNodes', () => {
  const nodes = mapOf(SAMPLE);
  const order = idOrder(nodes);

  it('by status puts what needs a person first, then failures, retries, work in progress, and what is done last', () => {
    expect(tableNodes(nodes, order, q()).map((n) => n.id)).toEqual(['hold', 'step-2', 'retrying', 'step-1', 'later', 'step-10', 'skipped']);
  });

  it('keeps the order of ids inside a status (the sort is stable)', () => {
    const many = mapOf([node('b', 'FAILED'), node('a', 'FAILED'), node('c', 'FAILED')]);
    expect(tableNodes(many, idOrder(many), q()).map((n) => n.id)).toEqual(['a', 'b', 'c']);
  });

  it('by id, either way', () => {
    expect(tableNodes(nodes, order, q({ sort: 'id' })).map((n) => n.id)).toEqual(order);
    expect(tableNodes(nodes, order, q({ sort: 'id', descending: true })).map((n) => n.id)).toEqual([...order].reverse());
  });

  it('descending by status puts what is done first', () => {
    const rows = tableNodes(nodes, order, q({ descending: true })).map((n) => n.id);
    expect(rows[0]).toBe('skipped');
    expect(rows.at(-1)).toBe('hold');
  });

  it('"only what needs attention" is what the model says needs attention', () => {
    const model = modelFromGraph({ runId: 'r', projectionSeq: 1, nodes: SAMPLE, edges: [] });
    const rows = tableNodes(model.nodes, order, q({ attentionOnly: true })).map((n) => n.id);
    expect(rows.sort()).toEqual(needsAttention(model).map((n) => n.id).sort());
    expect(rows).toHaveLength(3);
  });

  it('finds nodes by a part of the id, in any case, and combines it with the other filter', () => {
    expect(tableNodes(nodes, order, q({ text: 'STEP' })).map((n) => n.id)).toEqual(['step-2', 'step-1', 'step-10']);
    expect(tableNodes(nodes, order, q({ text: ' step ', attentionOnly: true })).map((n) => n.id)).toEqual(['step-2']);
    expect(tableNodes(nodes, order, q({ text: 'nothing like it' }))).toEqual([]);
  });

  it('treats a hostile id as text', () => {
    const odd = mapOf([node('<img src=x onerror=alert(1)>', 'FAILED'), node(".*", 'SUCCEEDED')]);
    expect(tableNodes(odd, idOrder(odd), q({ text: '.*' })).map((n) => n.id)).toEqual(['.*']);
    expect(tableNodes(odd, idOrder(odd), q({ text: '<img' }))).toHaveLength(1);
  });

  it('copes with an order that is out of date (a node the stream added)', () => {
    const extra = mapOf([...SAMPLE, node('new', 'READY')]);
    const rows = tableNodes(extra, order, q({ sort: 'id' }));
    expect(rows).toHaveLength(8);
    expect(rows.some((n) => n.id === 'new')).toBe(true);
  });

  it('ranks an unknown status last', () => {
    expect(statusRank(node('x', 'SOMETHING_NEW' as RuntimeNode['status']))).toBe(10);
  });
});

describe('a run of 50 000 steps (D147, edge case: graph with 50k nodes)', () => {
  const STATUSES: RuntimeNode['status'][] = ['SUCCEEDED', 'SUCCEEDED', 'SUCCEEDED', 'NOT_STARTED', 'RUNNING', 'FAILED', 'RETRY_WAIT', 'WAITING', 'SKIPPED', 'READY'];
  const big = mapOf(Array.from({ length: 50_000 }, (_, i) => node(`step-${i}`, STATUSES[i % STATUSES.length] as RuntimeNode['status'], i % 997 === 0 ? { waitingOn: 'operator' } : {})));

  it('is ordered once and then filtered and grouped fast enough to follow a stream', () => {
    const t0 = performance.now();
    const order = idOrder(big);
    const sortedOnce = performance.now() - t0;
    expect(order).toHaveLength(50_000);
    expect(order[0]).toBe('step-0');
    expect(order[10]).toBe('step-10');

    const t1 = performance.now();
    let rows: RuntimeNode[] = [];
    for (let i = 0; i < 20; i++) rows = tableNodes(big, order, q());
    const perDelta = (performance.now() - t1) / 20;

    expect(rows).toHaveLength(50_000);
    // What needs a person comes first.
    expect(rows[0]?.waitingOn).toBe('operator');
    expect(rows.slice(0, 51).every((n) => n.waitingOn === 'operator')).toBe(true);
    // Generous bounds (a slow CI machine must not fail them): the point is "milliseconds, not seconds".
    expect(sortedOnce).toBeLessThan(3000);
    expect(perDelta).toBeLessThan(250);
  });

  it('filters to the few that need attention without walking the table twice', () => {
    const order = idOrder(big);
    const rows = tableNodes(big, order, q({ attentionOnly: true }));
    expect(rows.length).toBeGreaterThan(5000);
    expect(rows.every((n) => n.status === 'FAILED' || n.status === 'RETRY_WAIT' || n.waitingOn === 'operator')).toBe(true);
    expect(tableNodes(big, order, q({ text: 'step-49999' })).map((n) => n.id)).toEqual(['step-49999']);
  });
});
