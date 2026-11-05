import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { SagatError, workflow } from '../src/index.js';
import type { UpdateDefinition } from '../src/index.js';

const PLAIN: UpdateDefinition = {
  argsSchema: { type: 'integer' },
  state: 'state',
  result: 'newState.balance',
  resultSchema: { type: 'integer' },
  disclosure: 'declared-business-result-v1',
};

/** The declaration of `balance.credit` as the builder writes it into the document. */
function declared(update: UpdateDefinition): Record<string, unknown> {
  const doc = workflow('account-state')
    .state({
      schema: { type: 'object', properties: { balance: { type: 'integer' } }, required: ['balance'], additionalProperties: false },
      initial: 'input.account',
      updates: { 'balance.credit': update },
    })
    .end('done')
    .build();
  const updates = (doc['state'] as Record<string, Record<string, Record<string, unknown>>>)['updates'];
  return updates?.['balance.credit'] ?? {};
}

test('an Update that opts in writes reapplyOnReset: true', () => {
  assert.equal(declared({ ...PLAIN, reapplyOnReset: true })['reapplyOnReset'], true);
});

test('the default is not written, so a document that does not opt in is the one made before the field', () => {
  assert.ok(!('reapplyOnReset' in declared(PLAIN)));
  assert.ok(!('reapplyOnReset' in declared({ ...PLAIN, reapplyOnReset: false })));
});

test('a value that is not a boolean is refused before the server sees it', () => {
  for (const wrong of ['yes', 1, null, {}, [true]]) {
    assert.throws(() => declared({ ...PLAIN, reapplyOnReset: wrong as unknown as boolean }), SagatError, `${JSON.stringify(wrong)} is not a boolean`);
  }
});
