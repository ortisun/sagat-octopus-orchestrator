import { describe, expect, it } from 'vitest';
import { databaseIssue } from './database';
import { addNode, renameNode, setField } from './edit';
import { emptyDocument } from './ir';
import { danglingReferences } from './references';

describe('database authoring', () => {
  it('emits the managed contract without activity or retry overrides', () => {
    const doc = addNode(emptyDocument('test'), 'database');
    expect(doc.nodes[0]).toEqual({ id: 'database_1', type: 'database', connectionRef: { name: '' }, operation: 'query', statement: 'SELECT 1::bigint', parameters: [] });
    expect(databaseIssue(doc.nodes[0]!)).toMatch(/connection/i);
    expect(databaseIssue({ ...doc.nodes[0]!, connectionRef: { name: 'orders-db' } })).toBeUndefined();
  });
  it('does not accept pending malformed parameter JSON as the previous valid request', () => {
    const node = { id: 'query', type: 'database', connectionRef: { name: 'orders-db' }, operation: 'query', statement: 'SELECT $1', parameters: '[' };
    expect(databaseIssue(node)).toMatch(/parameters/i);
    expect(databaseIssue({ ...node, parameters: [{ literal: { type: 'integer', value: '9223372036854775807' } }] })).toBeUndefined();
  });
  it('renames only expression bindings and reports deleted dependencies', () => {
    let doc = addNode(addNode(emptyDocument('test'), 'database', 'first'), 'database', 'second');
    doc = setField(doc, 'second', 'parameters', [{ type: 'integer', expression: 'steps.first.output.rows[0][0].value' }, { literal: { type: 'text', value: 'steps.first.output' } }]);
    doc = renameNode(doc, 'first', 'new-id');
    expect(doc.nodes[1]?.['parameters']).toEqual([{ type: 'integer', expression: 'steps["new-id"].output.rows[0][0].value' }, { literal: { type: 'text', value: 'steps.first.output' } }]);
    expect(danglingReferences(doc.nodes.slice(1))).toEqual([{ nodeId: 'second', field: 'parameters[0].expression', id: 'new-id' }]);
  });
});
