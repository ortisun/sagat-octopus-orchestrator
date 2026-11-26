import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import {
  CodeRun, WorkflowRuntimeUnavailable, WorkflowTestEnvironment, WorkflowThread, Xoshiro256, checkWorkflowRuntime,
  type HistoryEvent,
} from '../../src/workflows/index.js';
import { WorkflowSandbox } from '../../src/workflows/sandbox.js';
import { SEED, bundleOf, bundleSource, localBundle } from './support.js';

const START_US = 1_767_225_600_123_456;

function started(seq: number, atUs: number): Extract<HistoryEvent, { kind: 'workflow_task.started' }> {
  return { seq, kind: 'workflow_task.started', atUs, continueAsNewSuggested: false, historyLimitReached: false };
}

test('the context global is not a host object', () => {
  const bundle = localBundle({ noop: async () => null });
  const sandbox = new WorkflowSandbox(bundle, 2000);
  // Every value reachable from the global object has the context's own Object.prototype at the end of its chain.
  const foreign = sandbox.evaluate(`(() => {
    const root = Object.getPrototypeOf({});
    const found = [];
    for (const key of Reflect.ownKeys(globalThis)) {
      const value = globalThis[key];
      if (value === null || (typeof value !== 'object' && typeof value !== 'function')) continue;
      let proto = value;
      while (Object.getPrototypeOf(proto) !== null) proto = Object.getPrototypeOf(proto);
      if (proto !== root) found.push(String(key));
    }
    return found.join(',');
  })()`);
  assert.equal(foreign, '');
  assert.equal(sandbox.evaluate('typeof process + typeof require + typeof Buffer + typeof setImmediate'), 'undefinedundefinedundefinedundefined');
  assert.throws(() => sandbox.evaluate('this.constructor.constructor("return typeof process")()'), (error: unknown) => {
    // The error is the context's own EvalError: code generation from strings is refused.
    return (error as Error).name === 'EvalError';
  });
  assert.equal(sandbox.evaluate('typeof WeakRef + typeof FinalizationRegistry + typeof SharedArrayBuffer + typeof Atomics'),
    'undefinedundefinedundefinedundefined');
});

test('date now is the workflow time', async () => {
  const bundle = bundleOf({
    clock: async (ctx) => {
      const before = [Date.now(), new Date().getTime(), ctx.now().getTime(), typeof Date()];
      await ctx.sleep(60_000);
      return [...before, Date.now(), new Date(0).toISOString(), new Date() instanceof Date];
    },
  });
  const env = new WorkflowTestEnvironment({ bundle, workflowType: 'clock', startUs: START_US }).start(null);
  const outcome = await env.runUntilBlocked();
  assert.equal(outcome.status, 'COMPLETED');
  const starts = env.history.filter((event) => event.kind === 'workflow_task.started').map((event) => Math.trunc(event.atUs! / 1000));
  const output = outcome.output as unknown[];
  // The time of the first activation's start, truncated to milliseconds, then the second's.
  assert.deepEqual(output.slice(0, 4), [starts[0], starts[0], starts[0], 'string']);
  assert.equal(output[4], starts[1]);
  assert.equal(output[5], '1970-01-01T00:00:00.000Z');
  assert.equal(output[6], true);
  assert.ok(starts[1]! - starts[0]! >= 60_000);
});

test('the random numbers and identifiers of a run follow its seed', async () => {
  const bundle = bundleOf({
    dice: async (ctx) => [Math.random(), ctx.random(), ctx.randomInt(6), ctx.uuid()],
  });
  const env = new WorkflowTestEnvironment({ bundle, workflowType: 'dice', seed: SEED }).start(null);
  const outcome = await env.runUntilBlocked();
  const generator = new Xoshiro256(SEED);
  assert.deepEqual(outcome.output, [generator.random(), generator.random(), generator.randomInt(6), generator.uuid()]);
});

test('a module-level read of the clock or the generator is refused', async () => {
  for (const prelude of ['const loadedAt = Date.now();', 'const salt = Math.random();']) {
    await assert.rejects(WorkflowThread.shared().loadBundle(bundleOf({ noop: async () => null }, prelude).code),
      // The error's name crosses back, never its message.
      (error: unknown) => (error as Error).name === 'InvalidWorkflowBundle' && /\(NotInWorkflow\)/.test((error as Error).message));
  }
});

test('the locale and the time zone of workflow code are fixed', async () => {
  const bundle = bundleOf({
    format: async () => [
      (1234.5).toLocaleString(), new Intl.NumberFormat().resolvedOptions().locale,
      Intl.DateTimeFormat().resolvedOptions().timeZone, new Date(0).toLocaleString(), 'a'.localeCompare('B'),
    ],
  });
  const env = new WorkflowTestEnvironment({ bundle, workflowType: 'format' }).start(null);
  const outcome = await env.runUntilBlocked();
  assert.deepEqual(outcome.output, ['1,234.5', 'en-US', 'UTC', '1/1/1970, 12:00:00 AM', -1]);
});

test('promise race resolves in journal order', async () => {
  const bundle = localBundle({
    race: async (ctx) => {
      const a = ctx.activity('a.slow', 1);
      const b = ctx.activity('b.fast', 2);
      const winner = await Promise.race([a.then(() => 'a'), b.then(() => 'b')]);
      const both = await Promise.all([a, b]);
      return { winner, both };
    },
  });
  const init = { workflowType: 'race', seed: SEED, stateMaxBytes: 0, gatedEntries: [] };
  const live = new CodeRun(bundle, init);
  const first = live.activate([{ seq: 1, kind: 'run.started', input: null }, started(2, START_US)], started(2, START_US));
  assert.equal(first.kind, 'completion');
  live.commit('ACCEPTED');
  // b's outcome is journaled before a's: b wins, whatever order the activities were scheduled in.
  const window: HistoryEvent[] = [
    { seq: 3, kind: 'workflow_task.completed' },
    { seq: 4, kind: 'command.record', commandNumber: 1, commandKind: 'activity', target: 'a.slow' },
    { seq: 5, kind: 'command.record', commandNumber: 2, commandKind: 'activity', target: 'b.fast' },
    { seq: 6, kind: 'step.outcome', commandNumber: 2, status: 'completed', output: 'B' },
    { seq: 7, kind: 'step.outcome', commandNumber: 1, status: 'completed', output: 'A' },
    started(8, START_US + 5_000_000),
  ];
  const second = live.activate(window, started(8, START_US + 5_000_000));
  assert.equal(second.kind, 'completion');
  assert.deepEqual(second.kind === 'completion' && second.commands.map((command) => [command.kind, command.body]),
    [['complete', { output: { winner: 'b', both: ['A', 'B'] } }]]);
});

test('an infinite loop stops at the activation deadline', async () => {
  // A loop in the function's code and an endless chain of microtasks: both stopped, on the workflow thread, while this
  // test runner has async hooks enabled on its own thread (where the stop would corrupt the hook stack).
  for (const loop of ['async () => { for (;;) {} }', 'async () => { const spin = () => Promise.resolve().then(spin); spin(); await new Promise(() => {}); }']) {
    const thread = WorkflowThread.shared();
    const bundle = await thread.loadBundle(bundleOf({ spin: loop }).code);
    const run = await thread.createRun(bundle, { workflowType: 'spin', seed: SEED, stateMaxBytes: 0, gatedEntries: [] }, { activationDeadlineMs: 100 });
    const begun = Date.now();
    const answer = await run.activate([{ seq: 1, kind: 'run.started', input: null }, started(2, START_US)], started(2, START_US));
    assert.deepEqual(answer.kind === 'failure' && answer.failure, { cause: 'ACTIVATION_DEADLINE' });
    assert.ok(Date.now() - begun < 2000, 'the deadline stopped the loop');
    assert.equal(run.broken, true);
  }
});

test('registration refuses a process time zone other than UTC', () => {
  assert.doesNotThrow(() => checkWorkflowRuntime({ nodeVersion: () => '22.8.0', timeZone: () => 'UTC' }));
  assert.doesNotThrow(() => checkWorkflowRuntime({ nodeVersion: () => '22.8.0', timeZone: () => 'Etc/UTC' }));
  for (const zone of ['Asia/Kolkata', 'Africa/Abidjan', 'Europe/London']) {
    assert.throws(() => checkWorkflowRuntime({ nodeVersion: () => '22.8.0', timeZone: () => zone }),
      (error: unknown) => error instanceof WorkflowRuntimeUnavailable && error.reason === 'TIME_ZONE');
  }
});

test('registration refuses Node below 22.8', () => {
  for (const version of ['20.18.0', '22.7.0', '21.9.9']) {
    assert.throws(() => checkWorkflowRuntime({ nodeVersion: () => version, timeZone: () => 'UTC' }),
      (error: unknown) => error instanceof WorkflowRuntimeUnavailable && error.reason === 'NODE_VERSION');
  }
  assert.doesNotThrow(() => checkWorkflowRuntime({ nodeVersion: () => '22.8.0', timeZone: () => 'UTC' }));
  assert.doesNotThrow(() => checkWorkflowRuntime({ nodeVersion: () => '24.0.0', timeZone: () => 'UTC' }));
});

test('a bundle loaded outside a workflow context refuses to install', async () => {
  // The bundle's install call refuses a global that the host did not mark: evaluated as a plain script it throws.
  const vm = await import('node:vm');
  const plain = vm.createContext({});
  assert.throws(() => vm.runInContext(bundleSource({ noop: async () => null }), plain), /runs only inside the SDK's workflow context/);
});

test('an unhandled rejection in workflow code leaves the workflow thread running', async () => {
  const flow = 'async (ctx) => { Promise.reject(new Error("nobody listens")); const nap = ctx.sleep(5); ctx.requestCancellation(nap); await ctx.sleep(1); return "alive"; }';
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  assert.equal((await env.runUntilBlocked()).output, 'alive');
  const again = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  assert.equal((await again.runUntilBlocked()).output, 'alive', 'the same thread serves the next run');
});
