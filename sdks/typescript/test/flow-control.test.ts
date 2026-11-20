import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { canonicalJson, workflow, type ConcurrencyPolicy, type RateLimitPolicy } from '../src/index.js';

// Feature 022 (FR-022-035, FR-022-050): `concurrency` and `rateLimit` at the workflow level and on activity and script
// steps. The builders assemble the policy with the IR's names; the server checks the bounds at publication.

const fixture = (name: string): Record<string, unknown> =>
  JSON.parse(readFileSync(new URL(`../../../../tests/contract/flow-control-step-policies/fixtures/${name}`, import.meta.url), 'utf8'));

const providerSync = () => workflow('provider-sync')
  .concurrency({ key: 'input.accountId', limit: 1 })
  .rateLimit({ group: 'provider-starts', key: 'input.providerId', limit: 100, per: 'minute', queueTimeoutMs: 900000 })
  .step('fetch', 'provider.fetch', { rateLimit: { key: 'input.providerId', limit: 10, per: 'second' } })
  .end('done');

const accountSerial = () => workflow('account-serial')
  .concurrency({ group: 'orders-by-account', key: 'input.accountId', limit: 1, queueTimeoutMs: 600000 })
  .step('price', 'catalog.price')
  .step('notify', 'crm.notify', { concurrency: { key: 'steps.price.output.region', limit: 2 } })
  .end('done');

test('the builders write the shared fixtures, byte for byte in canonical form', () => {
  for (const [built, name] of [[providerSync(), 'provider-sync.workflow.json'], [accountSerial(), 'account-serial.workflow.json']] as const) {
    const expected = fixture(name);
    assert.deepEqual(built.build(), expected, name);
    assert.equal(built.toJson(), canonicalJson(expected as never), name);
  }
});

test('a workflow and a step without a policy carry no field, so earlier documents are unchanged', () => {
  const doc = workflow('plain-flow').step('a', 'act.a').script('s', `sha256:${'0'.repeat(64)}`).end('done').build();
  assert.equal(doc['concurrency'], undefined);
  assert.equal(doc['rateLimit'], undefined);
  for (const node of doc['nodes'] as Record<string, unknown>[]) {
    assert.equal(node['concurrency'], undefined);
    assert.equal(node['rateLimit'], undefined);
  }
});

test('a script step carries both policies, every member when given', () => {
  const concurrency: ConcurrencyPolicy = { key: 'input.tenant', limit: 10000, group: 'scripts.a-b', queueTimeoutMs: 86400000 };
  const rateLimit: RateLimitPolicy = { key: 'input.tenant', limit: 3, per: 'hour', group: 'scripts.a-b', queueTimeoutMs: 1 };
  const node = (workflow('gated-script').script('s', `sha256:${'0'.repeat(64)}`, { concurrency, rateLimit }).end('done').build()['nodes'] as Record<string, unknown>[])[0]!;
  assert.deepEqual(node['concurrency'], { key: 'input.tenant', limit: 10000, group: 'scripts.a-b', queueTimeoutMs: 86400000 });
  assert.deepEqual(node['rateLimit'], { key: 'input.tenant', limit: 3, per: 'hour', group: 'scripts.a-b', queueTimeoutMs: 1 });
});

test('the policy is copied when given, carries only its own members, and the last workflow-level call wins', () => {
  const policy = { limit: 2, burst: 5 } as ConcurrencyPolicy;
  const rate: RateLimitPolicy = { limit: 4, per: 'second' };
  const builder = workflow('copied-flow').concurrency({ limit: 9 }).concurrency(policy).rateLimit(rate)
    .step('a', 'act.a', { concurrency: policy, rateLimit: rate }).end('done');
  (policy as { limit: number }).limit = 7;
  (rate as { per: string }).per = 'hour';
  const doc = builder.build();
  assert.deepEqual(doc['concurrency'], { limit: 2 });
  assert.deepEqual(doc['rateLimit'], { limit: 4, per: 'second' });
  const node = (doc['nodes'] as Record<string, unknown>[])[0]!;
  assert.deepEqual(node['concurrency'], { limit: 2 });
  assert.deepEqual(node['rateLimit'], { limit: 4, per: 'second' });
});

test('the builders only assemble: bounds, the ceiling of a window and the scope are left for the server', () => {
  const doc = workflow('unchecked-flow')
    .concurrency({ limit: 0, group: 'Not A Group' })
    .rateLimit({ limit: 101, per: 'second', queueTimeoutMs: 86400001 })
    .step('a', 'act.a', { concurrency: { limit: 10001, key: '' } })
    .end('done').build();
  assert.deepEqual(doc['concurrency'], { limit: 0, group: 'Not A Group' });
  assert.deepEqual(doc['rateLimit'], { limit: 101, per: 'second', queueTimeoutMs: 86400001 });
  assert.deepEqual((doc['nodes'] as Record<string, unknown>[])[0]!['concurrency'], { limit: 10001, key: '' });
});
