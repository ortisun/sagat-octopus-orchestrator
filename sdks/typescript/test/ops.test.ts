import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { body, ops, workflow, type CollectionOptions, type Operator } from '../src/index.js';

const line = () => body().step('price', 'line.price').returnValue('return', 'steps.price.output');

test('the operator factory writes one stage per operator, keyed by its name', () => {
  assert.deepEqual(ops.filter('element.total > 0'), { filter: { where: 'element.total > 0' } });
  assert.deepEqual(ops.map('element.lines'), { map: { select: 'element.lines' } });
  assert.deepEqual(ops.flatMap('element'), { flatMap: { select: 'element' } });
  assert.deepEqual(ops.skip(0), { skip: { count: 0 } });
  assert.deepEqual(ops.take(1000), { take: { count: 1000 } });
  assert.deepEqual(ops.distinct('element.sku'), { distinct: { by: 'element.sku' } });
  assert.deepEqual(ops.distinct(), { distinct: {} });
  assert.deepEqual(ops.window(25), { window: { count: 25 } });
});

test('the factory only assembles: out-of-range values are left for the server to refuse', () => {
  assert.deepEqual(ops.window(0), { window: { count: 0 } });
  assert.deepEqual(ops.take(1001), { take: { count: 1001 } });
});

test('forEach and map carry the chain in the order given, as a copy', () => {
  const chain: Operator[] = [ops.filter('element.ok'), ops.distinct(), ops.window(2)];
  const doc = workflow('chained-flow').body('line', line())
    .forEach('lines', 'input.lines', 'line', 'bodyResult', { maxItems: 9, maxConcurrency: 2, operators: chain })
    .map('prices', 'input.lines', 'line', 'bodyResult', { maxItems: 9, maxConcurrency: 2, operators: [ops.take(3)] })
    .end('done').build();
  const nodes = doc['nodes'] as Record<string, unknown>[];
  assert.deepEqual(nodes[0]!['operators'], [{ filter: { where: 'element.ok' } }, { distinct: {} }, { window: { count: 2 } }]);
  assert.deepEqual(nodes[1]!['operators'], [{ take: { count: 3 } }]);
  assert.equal(nodes[1]!['type'], 'forEach');
  (nodes[0]!['operators'] as Record<string, unknown>[])[0] = { take: { count: 1 } };
  assert.deepEqual(chain[0], { filter: { where: 'element.ok' } });
});

test('every present collection call still builds, with no operators key', () => {
  const present: CollectionOptions = { maxItems: 17, maxConcurrency: 3 };
  const doc = workflow('plain-flow').body('line', line())
    .forEach('lines', 'input.lines', 'line', 'bodyResult', present)
    .map('prices', 'input.lines', 'line', 'bodyResult', { maxItems: 17, maxConcurrency: 3 })
    .forEach('empty', 'input.lines', 'line', 'bodyResult', { ...present, operators: [] })
    .end('done').build();
  for (const node of (doc['nodes'] as Record<string, unknown>[]).slice(0, 3)) {
    assert.equal('operators' in node, false);
  }
});
