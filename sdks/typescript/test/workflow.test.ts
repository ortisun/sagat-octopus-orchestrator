import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { SagatError, canonicalJson, flow, workflow } from '../src/index.js';

test('a chain of steps is linked in order and starts at the first', () => {
  const doc = workflow('order-flow')
    .description('charges and ships')
    .step('charge', 'payments.charge', { retry: { maxAttempts: 3, strategy: 'exponential', initialDelayMs: 100, maxDelayMs: 1000, jitter: true } })
    .sleep('pause', 1500)
    .step('ship', 'shipping.ship')
    .build();
  assert.equal(doc['entrypoint'], 'charge');
  assert.deepEqual(doc['metadata'], { name: 'order-flow', description: 'charges and ships' });
  assert.deepEqual(doc['nodes'], [
    { id: 'charge', type: 'activity', activity: 'payments.charge', retry: { maxAttempts: 3, strategy: 'exponential', initialDelayMs: 100, maxDelayMs: 1000, jitter: true }, next: 'pause' },
    { id: 'pause', type: 'timer', durationMs: 1500, next: 'ship' },
    { id: 'ship', type: 'activity', activity: 'shipping.ship' },
  ]);
});

test('absent options are left out and never null', () => {
  const doc = workflow('plain-flow').step('a', 'act.a', { compensateWith: undefined }).build();
  assert.deepEqual(doc['nodes'], [{ id: 'a', type: 'activity', activity: 'act.a' }]);
});

test('compensation, reconciliation and the other step options are written as the schema names them', () => {
  const doc = workflow('saga-flow')
    .step('reserve', 'limit.reserve', {
      compensateWith: 'limit.release',
      reconcileWith: 'limit.lookup',
      idempotencyTemplate: 'reserve:${run.id}',
      secrets: ['payments-key'],
      compensationRetry: { maxAttempts: 5 },
      compensationTimeoutMs: 10000,
      highRisk: true,
    })
    .build();
  assert.deepEqual((doc['nodes'] as unknown[])[0], {
    id: 'reserve', type: 'activity', activity: 'limit.reserve', compensateWith: 'limit.release', reconcileWith: 'limit.lookup',
    idempotencyTemplate: 'reserve:${run.id}', secrets: ['payments-key'], compensationRetry: { maxAttempts: 5 },
    compensationTimeoutMs: 10000, highRisk: true,
  });
});

test('a parallel lists the first node of each branch and continues with the next node of its flow', () => {
  const doc = workflow('fan-out')
    .parallel('split', 'all', [flow().step('card', 'pay.card').step('card-2', 'pay.card2'), flow().step('stock', 'stock.hold')])
    .step('done', 'order.done')
    .build();
  assert.deepEqual(doc['nodes'], [
    { id: 'split', type: 'parallel', branches: ['card', 'stock'], join: 'all', next: 'done' },
    { id: 'done', type: 'activity', activity: 'order.done' },
    { id: 'card', type: 'activity', activity: 'pay.card', next: 'card-2' },
    { id: 'card-2', type: 'activity', activity: 'pay.card2' },
    { id: 'stock', type: 'activity', activity: 'stock.hold' },
  ]);
});

test('a saga begins at its body, holds no end and continues with the next node of its flow', () => {
  const doc = workflow('saga-flow')
    .saga('tx', { onCompensationFailure: 'fail', cancelPolicy: 'abort_no_compensation' }, flow().step('reserve', 'a.reserve', { compensateWith: 'a.release' }).step('submit', 'a.submit'))
    .step('notify', 'a.notify')
    .build();
  assert.deepEqual((doc['nodes'] as unknown[])[0], { id: 'tx', type: 'saga', begin: 'reserve', onCompensationFailure: 'fail', cancelPolicy: 'abort_no_compensation', next: 'notify' });
  assert.deepEqual((doc['nodes'] as Array<Record<string, unknown>>).map((n) => n['id']), ['tx', 'notify', 'reserve', 'submit']);
});

test('wait for an event, invoke a child, decide and end', () => {
  const doc = workflow('mixed-flow')
    .waitForEvent('paid', { eventType: 'payment.confirmed', correlationExpr: 'input.orderId', timeoutMs: 60000, onTimeout: 'cancel', lookbackMs: 1000 })
    .invokeChild('ship', 'shipping-flow', { version: 2, cancelPolicy: 'detach', input: 'input.order' })
    .condition('big', 'input.amount > 100', 'review', 'cancel')
    .step('review', 'order.review', { onError: 'cancel' })
    .step('cancel', 'order.cancel')
    .end('done', { outputExpr: 'steps.ship.output' })
    .build();
  const nodes = doc['nodes'] as Array<Record<string, unknown>>;
  assert.deepEqual(nodes[0], { id: 'paid', type: 'waitEvent', eventType: 'payment.confirmed', correlationExpr: 'input.orderId', timeoutMs: 60000, onTimeout: 'cancel', lookbackMs: 1000, next: 'ship' });
  assert.deepEqual(nodes[1], { id: 'ship', type: 'child', workflowId: 'shipping-flow', version: 2, cancelPolicy: 'detach', input: 'input.order', next: 'big' });
  assert.deepEqual(nodes[5], { id: 'done', type: 'end', outputExpr: 'steps.ship.output' });
});

test('triggers, labels and schemas go where the document keeps them', () => {
  const doc = workflow('with-meta')
    .label('team', 'payments')
    .trigger({ kind: 'event', eventType: 'order.created', source: 'api' })
    .inputSchema({ type: 'object' })
    .outputSchema({ type: 'object' })
    .step('a', 'act.a')
    .build();
  assert.deepEqual(doc['metadata'], { name: 'with-meta', labels: { team: 'payments' } });
  assert.deepEqual(doc['triggers'], [{ kind: 'event', eventType: 'order.created', source: 'api' }]);
  assert.deepEqual(doc['inputSchema'], { type: 'object' });
});

test('what the server would refuse and the builder can see is refused here', () => {
  const refused = (build: () => unknown, pattern: RegExp): void => {
    assert.throws(build, (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED' && pattern.test(error.message));
  };
  refused(() => workflow('Bad Name'), /workflow name/);
  refused(() => workflow('ab'), /workflow name/);
  refused(() => workflow('empty-flow').build(), /at least one node/);
  refused(() => workflow('dup-ids').step('a', 'x').step('a', 'y').build(), /used twice/);
  refused(() => workflow('dup-nested').step('a', 'x').parallel('p', 'all', [flow().step('a', 'y'), flow().step('b', 'z')]).build(), /used twice/);
  refused(() => workflow('one-branch').parallel('p', 'all', [flow().step('a', 'x')]), /two branches/);
  refused(() => workflow('empty-branch').parallel('p', 'all', [flow().step('a', 'x'), flow()]), /empty branch/);
  refused(() => workflow('empty-saga').saga('s', {}, flow()), /empty body/);
  refused(() => workflow('saga-end').saga('s', {}, flow().step('a', 'x').end('e')), /end node/);
  refused(() => workflow('both').step('a', 'x', { irreversible: true, compensateWith: 'undo' }), /irreversible/);
  refused(() => workflow('bad-secret').step('a', 'x', { secrets: ['Bad Secret'] }), /secret/);
  refused(() => workflow('bad-timer').sleep('t', 0), /duration/);
  refused(() => workflow('dangling').condition('c', 'true', 'nowhere', 'a').step('a', 'x').build(), /not a node/);
  refused(() => workflow('dangling-timeout').waitForEvent('w', { eventType: 'e', correlationExpr: 'x', timeoutMs: 1, onTimeout: 'nowhere' }).build(), /not a node/);
});

test('the canonical text sorts keys at every depth and leaves out whitespace', () => {
  assert.equal(canonicalJson({ b: 1, a: { d: [2, { z: 1, y: 2 }], c: null }, e: undefined }), '{"a":{"c":null,"d":[2,{"y":2,"z":1}]},"b":1}');
  const text = workflow('canon-flow').step('a', 'act.a', { retry: { maxAttempts: 2 } }).toJson();
  assert.ok(text.startsWith('{"apiVersion":'));
  assert.ok(!text.includes(' '));
});
