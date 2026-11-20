import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { SagatError, workflow, type IcebergWriteOptions } from '../src/index.js';

const base: IcebergWriteOptions = {
  connectionRef: { name: 'lake-main', revision: 3 },
  table: { namespace: ['sales'], name: 'orders' },
  records: { inline: 'steps.extract.output.rows' },
};

test('the iceberg write builder matches the shared valid document', () => {
  const fixture = JSON.parse(readFileSync(new URL('../../../../tests/contract/iceberg-writes/fixtures/ir/valid/append-inline.json', import.meta.url), 'utf8'));
  const built = workflow(fixture.metadata.name).step('extract', 'orders.extract').icebergWrite('load', base).end('done').build();
  assert.deepEqual(built, fixture);
});

test('the mode is always append, and a minimal write carries nothing it was not given', () => {
  const node = (workflow('lake').icebergWrite('w', base).end('done').build()['nodes'] as Record<string, unknown>[])[0]!;
  assert.equal(node['mode'], 'append');
  assert.equal(node['type'], 'icebergWrite');
  for (const absent of ['ref', 'mapping', 'limits', 'compensation', 'ignoreUnknownFields', 'retry', 'secrets', 'overwrite', 'upsert']) assert.equal(node[absent], undefined, absent);
});

test('every option is carried, and the three record sources are accepted', () => {
  const full: IcebergWriteOptions = {
    ...base,
    ref: 'staging',
    mapping: [{ field: '/amount', column: 'amount' }, { field: '/customer/name', column: 'customer_name' }],
    ignoreUnknownFields: true,
    limits: { maxRecordBytes: 1024, maxRecords: 100000000, timeoutMs: 3600000 },
    compensation: 'revert',
    onError: 'recover',
  };
  const node = (workflow('lake').icebergWrite('w', full).step('recover', 'orders.recover').end('done').build()['nodes'] as Record<string, unknown>[])[0]!;
  assert.equal(node['ref'], 'staging');
  assert.deepEqual(node['limits'], { maxRecordBytes: 1024, maxRecords: 100000000, timeoutMs: 3600000 });
  assert.equal(node['compensation'], 'revert');
  assert.equal(node['onError'], 'recover');
  for (const records of [{ inline: 'input.rows' }, { blobs: 'steps.a.output.parts' }, { databaseResult: 'read' }]) {
    assert.doesNotThrow(() => workflow('lake').icebergWrite('w', { ...base, records } as IcebergWriteOptions));
  }
});

test('the node is a snapshot: later changes to the options do not reach it', () => {
  const options: IcebergWriteOptions = { ...base, mapping: [{ field: '/a', column: 'a' }] };
  const flow = workflow('lake').icebergWrite('w', options);
  options.mapping![0]!.column = 'changed';
  options.table.name = 'changed';
  const node = (flow.end('done').build()['nodes'] as Record<string, unknown>[])[0]!;
  assert.deepEqual(node['mapping'], [{ field: '/a', column: 'a' }]);
  assert.deepEqual(node['table'], { namespace: ['sales'], name: 'orders' });
});

test('what the server would refuse is refused here with one fixed message that shows no content', () => {
  const canary = 'credential-canary';
  const refused: Array<Record<string, unknown>> = [
    { connectionRef: { name: `postgres://${canary}@host/db` } },
    { connectionRef: { name: 'lake-main', revision: 0 } },
    { connectionRef: { name: 'lake--main' } },
    { table: { namespace: [], name: 'orders' } },
    { table: { namespace: ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i'], name: 'orders' } },
    { table: { namespace: ['sales'], name: '' } },
    { table: { namespace: ['sales'], name: '..' } },
    { table: { namespace: ['sales\u0000'], name: 'orders' } },
    { table: { namespace: ['sales'], name: 'orders', catalog: 'other' } },
    { table: { namespace: 'sales', name: 'orders' } },
    { ref: '' },
    { ref: 'r'.repeat(256) },
    { records: {} },
    { records: { inline: 'a', blobs: 'b' } },
    { records: { files: 'a' } },
    { records: { inline: '' } },
    { records: { inline: 'x'.repeat(1025) } },
    { records: { inline: 'é'.repeat(513) } },
    { records: { inline: 5 } },
    { records: { databaseResult: '' } },
    { mapping: [{ field: 'amount', column: 'amount' }] },
    { mapping: [{ field: '/a//b', column: 'a' }] },
    { mapping: [{ field: '/a~2', column: 'a' }] },
    { mapping: [{ field: `/${'a/'.repeat(16)}a`, column: 'a' }] },
    { mapping: [{ field: '/a', column: '' }] },
    { mapping: [{ field: '/a', column: 'c' }, { field: '/b', column: 'c' }] },
    { mapping: [{ field: '/a', column: 'c' }, { field: '/a', column: 'd' }] },
    { mapping: [{ field: '/a', column: 'c', type: 'int' }] },
    { mapping: 'x' },
    { ignoreUnknownFields: 'yes' },
    { limits: { maxRecords: 0 } },
    { limits: { maxRecords: 100000001 } },
    { limits: { maxRecordBytes: 1023 } },
    { limits: { timeoutMs: 999 } },
    { limits: { timeoutMs: 1.5 } },
    { limits: { maxCommitRetries: 4 } },
    { compensation: 'undo' },
    { onError: '' },
    { mode: 'overwrite' },
    { overwrite: { scope: 'table' } },
    { upsert: {} },
    { recordSchema: {} },
    { retry: {} },
  ];
  for (const changes of refused) {
    assert.throws(() => workflow('lake').icebergWrite('w', { ...base, ...changes } as IcebergWriteOptions),
      (e: unknown) => e instanceof SagatError && e.code === 'VALIDATION_FAILED' && e.message === 'invalid iceberg write' && !e.message.includes(canary), JSON.stringify(changes));
  }
});

test('the edges that are accepted are accepted', () => {
  const edges: Array<Record<string, unknown>> = [
    { table: { namespace: ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'], name: 'n'.repeat(255) } },
    { ref: 'r'.repeat(255) },
    { records: { inline: 'x'.repeat(1024) } },
    { records: { inline: 'é'.repeat(512) } },
    { mapping: [{ field: `/${'a/'.repeat(15)}a`, column: 'c'.repeat(255) }] },
    { mapping: [{ field: '/a~0b/c~1d', column: 'c' }] },
    { mapping: [] },
    { limits: { maxRecordBytes: 2147483648, maxRecords: 1, timeoutMs: 1000 } },
    { limits: {} },
    { connectionRef: { name: 'abc' } },
  ];
  for (const changes of edges) assert.doesNotThrow(() => workflow('lake').icebergWrite('w', { ...base, ...changes } as IcebergWriteOptions), JSON.stringify(changes));
});

test('a node chains like any other: next is linked by the flow', () => {
  const nodes = workflow('lake').step('extract', 'orders.extract').icebergWrite('load', base).step('notify', 'notify.done').end('done').build()['nodes'] as Record<string, unknown>[];
  assert.equal(nodes[0]!['next'], 'load');
  assert.equal(nodes[1]!['next'], 'notify');
});
