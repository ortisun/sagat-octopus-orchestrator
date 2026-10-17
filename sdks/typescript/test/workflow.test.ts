import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { SagatError, body, canonicalJson, flow, workflow } from '../src/index.js';

test('classified result schemas and workflow limits are preserved by their typed carriers', () => {
  const schema = { type: 'object', 'x-sagat-classification': 'internal', properties: { value: { type: 'string' } } } as const;
  const doc = workflow('classified-flow').limits({ maxInstances: 23, maxDepth: 4 })
    .outputSchema(schema).step('a', 'act.a', { outputSchema: schema })
    .script('s', `sha256:${'0'.repeat(64)}`, { outputSchema: schema })
    .waitForEvent('w', { eventType: 'done', correlationExpr: 'input.id', timeoutMs: 10, outputSchema: schema })
    .invokeChild('c', 'child-flow', { outputSchema: schema }).end('done').build();
  assert.deepEqual(doc['limits'], { maxInstances: 23, maxDepth: 4 });
  assert.deepEqual((doc['nodes'] as Record<string, unknown>[]).slice(0, 4).map((node) => node['outputSchema']), [schema, schema, schema, schema]);
});

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
  assert.equal(nodes[2]?.['next'], undefined, 'a condition names both branches and has no next');
  assert.deepEqual(nodes[0], { id: 'paid', type: 'waitEvent', eventType: 'payment.confirmed', correlationExpr: 'input.orderId', timeoutMs: 60000, onTimeout: 'cancel', lookbackMs: 1000, next: 'ship' });
  assert.deepEqual(nodes[1], { id: 'ship', type: 'child', workflowId: 'shipping-flow', version: 2, cancelPolicy: 'detach', input: 'input.order', next: 'big' });
  assert.deepEqual(nodes[5], { id: 'done', type: 'end', outputExpr: 'steps.ship.output' });
});

test('a switch keeps case order, copies its cases and has no implicit next', () => {
  const cases = [
    { id: 'nonnegative', when: 'input.amount >= 0', target: 'positive' },
    { id: 'one', when: 'input.amount == 1', target: 'also-positive' },
  ];
  const builder = workflow('switch-flow')
    .switch('route', cases, 'fallback')
    .step('positive', 'amount.positive')
    .step('also-positive', 'amount.also-positive')
    .step('fallback', 'amount.fallback');
  cases[0]!.target = 'tampered';

  const nodes = builder.build()['nodes'] as Array<Record<string, unknown>>;
  assert.equal(nodes[0]?.['next'], undefined);
  assert.deepEqual(nodes[0], {
    id: 'route',
    type: 'switch',
    cases: [
      { id: 'nonnegative', when: 'input.amount >= 0', target: 'positive' },
      { id: 'one', when: 'input.amount == 1', target: 'also-positive' },
    ],
    default: 'fallback',
  });
  assert.doesNotThrow(() => workflow('local-case-ids')
    .switch('first', [{ id: 'same', when: 'true', target: 'done' }], 'done')
    .switch('second', [{ id: 'same', when: 'false', target: 'done' }], 'done')
    .end('done')
    .build());
});

test('collections use named bodies and map writes the same forEach node', () => {
  const line = body().step('price', 'line.price').returnValue('return', 'steps.price.output');
  const tax = body().step('price', 'tax.price').returnValue('return', 'steps.price.output');
  const document = workflow('collection-flow')
    .body('line', line)
    .body('tax', tax)
    .forEach('notify-lines', 'input.lines', 'line', 'bodyResult', { maxItems: 17, maxConcurrency: 3 })
    .map('prices', 'input.lines', 'tax', 'bodyResult.amount', { maxItems: 17, maxConcurrency: 2 })
    .end('done', { outputExpr: 'steps.prices.output' })
    .build();

  assert.deepEqual(document['nodes'], [
    { id: 'notify-lines', type: 'forEach', items: 'input.lines', body: 'line', result: 'bodyResult', maxItems: 17, maxConcurrency: 3, next: 'prices' },
    { id: 'prices', type: 'forEach', items: 'input.lines', body: 'tax', result: 'bodyResult.amount', maxItems: 17, maxConcurrency: 2, next: 'done' },
    { id: 'done', type: 'end', outputExpr: 'steps.prices.output' },
  ]);
  assert.deepEqual(document['bodies'], {
    line: {
      entrypoint: 'price',
      nodes: [
        { id: 'price', type: 'activity', activity: 'line.price', next: 'return' },
        { id: 'return', type: 'return', result: 'steps.price.output' },
      ],
    },
    tax: {
      entrypoint: 'price',
      nodes: [
        { id: 'price', type: 'activity', activity: 'tax.price', next: 'return' },
        { id: 'return', type: 'return', result: 'steps.price.output' },
      ],
    },
  });
});

test('named bodies cannot reference themselves', () => {
  const recursive = body().forEach('nested', 'input.items', 'line', 'bodyResult', { maxItems: 2, maxConcurrency: 1 }).returnValue('return', 'steps.nested.output');
  assert.throws(
    () => workflow('recursive-body').body('line', recursive).forEach('lines', 'input.items', 'line', 'bodyResult', { maxItems: 2, maxConcurrency: 1 }).end('done').build(),
    (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED' && /body reference cycle/.test(error.message),
  );
});

test('while and repeatUntil write bounded pre-test and post-test loop nodes', () => {
  const iteration = body().step('increment', 'counter.increment').returnValue('return', 'steps.increment.output');
  const document = workflow('loop-flow')
    .body('iteration', iteration)
    .while('pre', 'carry < 3', 'iteration', 'input.start', 'bodyResult', {
      maxIterations: 3, timeoutMs: 60_000, delayMs: 250, onError: 'failed',
    })
    .repeatUntil('post', 'carry >= 5', 'iteration', 'steps.pre.output', 'bodyResult', {
      maxIterations: 5, timeoutMs: 120_000,
    })
    .end('done')
    .end('failed')
    .build();

  assert.deepEqual(document['nodes'], [
    { id: 'pre', type: 'while', condition: 'carry < 3', body: 'iteration', initial: 'input.start', carry: 'bodyResult', maxIterations: 3, timeoutMs: 60_000, delayMs: 250, onError: 'failed', next: 'post' },
    { id: 'post', type: 'repeatUntil', condition: 'carry >= 5', body: 'iteration', initial: 'steps.pre.output', carry: 'bodyResult', maxIterations: 5, timeoutMs: 120_000, next: 'done' },
    { id: 'done', type: 'end' },
    { id: 'failed', type: 'end' },
  ]);
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
  refused(() => workflow('no-switch-cases').switch('s', [], 'a'), /between 1 and 32 cases/);
  refused(() => workflow('too-many-cases').switch('s', Array.from({ length: 33 }, (_, i) => ({ id: `case-${i}`, when: 'true', target: 'a' })), 'a'), /between 1 and 32 cases/);
  refused(() => workflow('duplicate-cases').switch('s', [{ id: 'same', when: 'true', target: 'a' }, { id: 'same', when: 'false', target: 'b' }], 'a'), /case id/);
  refused(() => workflow('empty-case-id').switch('s', [{ id: '', when: 'true', target: 'a' }], 'a'), /without an id/);
  refused(() => workflow('empty-case-when').switch('s', [{ id: 'one', when: '', target: 'a' }], 'a'), /when expression/);
  refused(() => workflow('empty-case-target').switch('s', [{ id: 'one', when: 'true', target: '' }], 'a'), /needs a target/);
  refused(() => workflow('empty-switch-default').switch('s', [{ id: 'one', when: 'true', target: 'a' }], ''), /default target/);
  refused(() => workflow('dangling-switch-case').switch('s', [{ id: 'one', when: 'true', target: 'nowhere' }], 'a').step('a', 'x').build(), /not a node/);
  refused(() => workflow('dangling-switch-default').switch('s', [{ id: 'one', when: 'true', target: 'a' }], 'nowhere').step('a', 'x').build(), /not a node/);
  refused(() => workflow('dangling-timeout').waitForEvent('w', { eventType: 'e', correlationExpr: 'x', timeoutMs: 1, onTimeout: 'nowhere' }).build(), /not a node/);
});

test('the canonical text sorts keys at every depth and leaves out whitespace', () => {
  assert.equal(canonicalJson({ b: 1, a: { d: [2, { z: 1, y: 2 }], c: null }, e: undefined }), '{"a":{"c":null,"d":[2,{"y":2,"z":1}]},"b":1}');
  const text = workflow('canon-flow').step('a', 'act.a', { retry: { maxAttempts: 2 } }).toJson();
  assert.ok(text.startsWith('{"apiVersion":'));
  assert.ok(!text.includes(' '));
});
