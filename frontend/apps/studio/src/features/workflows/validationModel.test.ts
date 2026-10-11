import { describe, expect, it } from 'vitest';
import { countsByNode, nodeIdsOf, resolveNodeId, summaryLine, viewOf } from './validationModel';

describe('validation model', () => {
  const ids = ['check', 'pay', undefined, 'done'];

  it('takes the node a problem names', () => {
    expect(resolveNodeId({ nodeId: 'pay', path: '/nodes/0/x' }, ids)).toBe('pay');
  });

  it('finds the node of a schema problem from its position in the document that was checked', () => {
    expect(resolveNodeId({ nodeId: null, path: '/nodes/1/activity' }, ids)).toBe('pay');
    expect(resolveNodeId({ path: '/nodes/3' }, ids)).toBe('done');
    // An entry the Studio could not read has no id to point at.
    expect(resolveNodeId({ path: '/nodes/2/type' }, ids)).toBeUndefined();
    expect(resolveNodeId({ path: '/nodes/99/type' }, ids)).toBeUndefined();
  });

  it('does not take a pointer to something else for a node', () => {
    expect(resolveNodeId({ path: '/entrypoint' }, ids)).toBeUndefined();
    expect(resolveNodeId({ path: '/triggers/0/kind' }, ids)).toBeUndefined();
    expect(resolveNodeId({ path: '/nodesX/1' }, ids)).toBeUndefined();
    expect(resolveNodeId({ nodeId: '', path: null }, ids)).toBeUndefined();
  });

  it('reads the order of nodes of the document that was sent', () => {
    expect(nodeIdsOf({ nodes: [{ id: 'a' }, 'junk', { type: 'end' }, { id: 'b' }] })).toEqual(['a', undefined, undefined, 'b']);
    expect(nodeIdsOf({})).toEqual([]);
  });

  it('turns an answer into problems and warnings with their nodes', () => {
    const view = viewOf(
      {
        valid: false,
        problems: [
          { severity: 'error', code: 'SCHEMA', message: 'missing activity', path: '/nodes/1/activity' },
          { severity: 'error', code: 'CYCLE', message: 'loop', nodeId: 'check' },
        ],
        warnings: [{ severity: 'warning', code: 'RECURSIVE_CHILD', message: 'child', nodeId: null, path: null }],
      },
      ids,
    );
    expect(view.problems.map((p) => [p.code, p.nodeId])).toEqual([
      ['SCHEMA', 'pay'],
      ['CYCLE', 'check'],
    ]);
    expect(view.warnings[0]?.nodeId).toBeUndefined();
    expect(view.problems[0]?.path).toBe('/nodes/1/activity');
    const counts = countsByNode(view.problems);
    expect(counts.get('pay')).toBe(1);
    expect(counts.get('check')).toBe(1);
    expect(counts.has('done')).toBe(false);
  });

  it('counts findings per node, ignoring those about no node', () => {
    const counts = countsByNode([{ nodeId: 'a' }, { nodeId: 'a' }, { nodeId: undefined }, { nodeId: '' }]);
    expect([...counts]).toEqual([['a', 2]]);
  });

  it('writes the summary line with the right plurals', () => {
    expect(summaryLine(2, 1, 3)).toBe('2 problems · 1 warning · 3 pieces of advice');
    expect(summaryLine(1, 0, 1)).toBe('1 problem · 0 warnings · 1 piece of advice');
  });
});
