import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { SagatError, workflow, type DatabaseOptions } from '../src/index.js';

test('managed database builder matches the shared typed wire fixture', () => {
  const fixture = JSON.parse(readFileSync(new URL('../../../../tests/contract/database-connections/typed-parameters.workflow.json', import.meta.url), 'utf8'));
  const { id, type: _type, ...options } = fixture.nodes[0];
  assert.deepEqual(workflow(fixture.metadata.name).database(id, options as DatabaseOptions).build(), fixture);
});

test('database authoring accepts references and types; never prints rejected content', () => {
  const base: DatabaseOptions = { connectionRef: { name: 'orders-db' }, operation: 'query', statement: 'SELECT $1', parameters: [{ literal: { type: 'integer', value: '9223372036854775807' } }] };
  const node = (workflow('database-chain').database('q', base).end('done').build()['nodes'] as Record<string, unknown>[])[0]!;
  assert.equal(node['next'], 'done'); assert.equal(node['retry'], undefined); assert.equal(node['secrets'], undefined);
  for (const changes of [
    { connectionRef: { name: 'postgres://credential-canary@host/db' } },
    { statement: '' }, { operation: 'credential-canary' },
    { parameters: [{ literal: { type: 'integer', value: 123 } }] },
    { parameters: [{ literal: { type: 'decimal', value: 'credential-canary' } }] },
    { connectionRef: { name: 'orders-db', revision: 0 } },
  ]) {
    assert.throws(() => workflow('database-invalid').database('q', { ...base, ...changes } as DatabaseOptions),
      (e: unknown) => e instanceof SagatError && e.code === 'VALIDATION_FAILED' && !e.message.includes('credential-canary'));
  }
});
