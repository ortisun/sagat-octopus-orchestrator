import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { WorkflowRunCache, type RunHandle } from '../../src/workflows/index.js';

function handle(windowStart: number, flags: { broken?: boolean; closed?: boolean } = {}): RunHandle & { disposed: boolean } {
  const run = {
    windowStart, acceptedCommands: 0, chain: new Uint8Array(32), broken: flags.broken ?? false, closed: flags.closed ?? false,
    deferredBodyDigest: undefined, disposed: false,
    replay: async () => ({ kind: 'replayed' as const }),
    activate: async () => { throw new Error('not used'); },
    commit: async () => undefined,
    abandon: async () => undefined,
    bodyDigests: async () => [],
    dispose(): void { run.disposed = true; },
  };
  return run;
}

test('a cache whose state is not at the window start is dropped', () => {
  const cache = new WorkflowRunCache();
  const run = handle(40);
  cache.put('run-1', run);
  // The next lease's window starts at 57: another worker completed an activation meanwhile.
  assert.equal(cache.take('run-1', 57), undefined);
  assert.equal(run.disposed, true);
  assert.equal(cache.size, 0);
  const current = handle(57);
  cache.put('run-1', current);
  assert.equal(cache.take('run-1', 57), current);
  assert.equal(current.disposed, false);
});

test('a broken or closed run is never reused', () => {
  const cache = new WorkflowRunCache();
  const broken = handle(10, { broken: true });
  cache.put('run-1', broken);
  assert.equal(broken.disposed, true);
  assert.equal(cache.take('run-1', 10), undefined);
  const closed = handle(10, { closed: true });
  cache.put('run-2', closed);
  assert.equal(cache.take('run-2', 10), undefined);
});

test('the cache evicts the least recently used run beyond its size and reports it', () => {
  const evicted: string[] = [];
  const cache = new WorkflowRunCache(2, (runId) => evicted.push(runId));
  const a = handle(1);
  cache.put('a', a);
  cache.put('b', handle(1));
  cache.put('a', cache.take('a', 1)!);
  cache.put('c', handle(1));
  assert.deepEqual(evicted, ['b']);
  assert.equal(a.disposed, false);
  assert.equal(cache.size, 2);
});
