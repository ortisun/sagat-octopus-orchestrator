import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { WorkflowTestEnvironment, type HistoryEvent, type WorkflowContext } from '../../src/workflows/index.js';
import { bundleOf, type TestWorkflow } from './support.js';

// The rules of the contract of Task 044-2 that the runtime and the test environment follow (Decisions 044-317 and
// 044-318 of research.md, after Decisions 044-244 to 044-251).

const records = (env: WorkflowTestEnvironment): unknown[] =>
  env.history.flatMap((event) => (event.kind === 'command.record' ? [[event.commandNumber, event.commandKind, event.target, event.sagaScope ?? '']] : []));

// The same commands, kinds and targets either way; only the scope the activity is scheduled in differs. A workflow
// function is passed to the bundle as its source text, so the variant is written into the text.
const twoScopes = (inner: 'first' | 'second'): string => `async (ctx) => {
  let ready;
  const opened = new Promise((resolve) => { ready = resolve; });
  const outer = ctx.saga({}, async (first) => {
    await opened;
    ${inner === 'first' ? "await first.activity('inventory.reserve', { sku: 'A-1' });" : ''}
    return first.scope;
  });
  const other = ctx.saga({}, async (second) => {
    ready();
    ${inner === 'second' ? "await second.activity('inventory.reserve', { sku: 'A-1' });" : ''}
    return second.scope;
  });
  return [await outer, await other];
}`;

test('a command record carries the saga scope of its activity', async () => {
  const env = new WorkflowTestEnvironment({
    bundle: bundleOf({ flow: twoScopes('first') }), workflowType: 'flow', activities: { 'inventory.reserve': () => 'r-1' },
  }).start(null);
  const outcome = await env.runUntilBlocked();
  assert.equal(outcome.status, 'COMPLETED');
  assert.deepEqual(records(env).slice(0, 3), [[1, 'saga_open', 's-1', ''], [2, 'saga_open', 's-2', ''], [3, 'activity', 'inventory.reserve', 's-1']]);
  assert.equal((await env.replay()).verdict, 'MATCH');
});

test('a replay that schedules an activity in another saga scope is a mismatch that names both scopes', async () => {
  const env = new WorkflowTestEnvironment({
    bundle: bundleOf({ flow: twoScopes('first') }), workflowType: 'flow', activities: { 'inventory.reserve': () => 'r-1' },
  }).start(null);
  await env.runUntilBlocked();
  const verdict = await env.replay(bundleOf({ flow: twoScopes('second') }));
  assert.deepEqual(verdict, {
    verdict: 'NONDETERMINISM', activation: 1, commandNumber: 3, actualKind: 'activity', actualTarget: 'inventory.reserve',
    actualSagaScope: 's-2', expectedKind: 'activity', expectedTarget: 'inventory.reserve', expectedSagaScope: 's-1',
  });
});

test('a validator that refuses an update rejects it with VALIDATION_FAILED', async () => {
  const flow: TestWorkflow = async (ctx) => {
    ctx.onUpdate('add', () => { throw new Error('refused'); }, async () => 1);
    await ctx.sleep(1000);
    return null;
  };
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  const seq = env.update('add', {});
  await env.runUntilBlocked();
  assert.deepEqual(env.updates.get(seq), { name: 'add', status: 'REJECTED', reason: 'VALIDATION_FAILED' });
});

test('a script takes timeoutMs in its fragment', async () => {
  const digest = `sha256:${'ab'.repeat(32)}`;
  const flow = `async (ctx) => await ctx.script('${digest}', { rows: [1] }, { retry: { maxAttempts: 2 }, timeoutMs: 5000 })`;
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow', scripts: { [digest]: () => 7 } }).start(null);
  assert.equal((await env.runUntilBlocked()).output, 7);
  const script = env.completions[0]!.completion.commands[0]!;
  assert.deepEqual(script.body, { type: 'script', artifactDigest: digest, input: { rows: [1] }, retry: { maxAttempts: 2 }, timeoutMs: 5000 });
});

test('a wait for an event requires timeoutMs in its type', () => {
  // Compiled, never run: the line below must not type-check (Decision 044-247).
  const typed = (ctx: WorkflowContext): unknown => {
    // @ts-expect-error timeoutMs is required
    return ctx.waitForEvent('paid', 'k', {});
  };
  assert.equal(typeof typed, 'function');
});

const cancelOne = `async (ctx) => {
  const work = ctx.activity('payments.capture', { amount: 5 });
  ctx.requestCancellation(work);
  try { return { output: await work }; } catch (error) { return { error: error.name }; }
}`;

test('the function\'s cancellation of an activity no worker took cancels it at once', async () => {
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow: cancelOne }), workflowType: 'flow' }).start(null);
  assert.deepEqual((await env.runUntilBlocked()).output, { error: 'CommandCancelled' });
  const outcome = env.history.find((event) => event.kind === 'step.outcome') as Extract<HistoryEvent, { kind: 'step.outcome' }>;
  assert.equal(outcome.status, 'cancelled');
  assert.equal(outcome.errorCode, 'CANCELLED');
  assert.equal(outcome.origin, 'effect');
  assert.equal((await env.replay()).verdict, 'MATCH');
});

test('the function\'s cancellation of an activity whose attempt began leaves its outcome to the attempt', async () => {
  // The activity is scheduled in one activation and cancelled in the next, after a worker leased it.
  const flow = `async (ctx) => {
    const work = ctx.activity('payments.capture', { amount: 5 });
    await ctx.sleep(1000);
    ctx.requestCancellation(work);
    try { return { output: await work }; } catch (error) { return { error: error.name }; }
  }`;
  for (const ending of ['completed', 'cancelled'] as const) {
    const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
    env.onTaskStarted = (task) => { if (task === 2) env.beginStep(1); };
    assert.equal((await env.runUntilBlocked()).status, 'RUNNING');
    assert.ok(env.completions[1]!.completion.commands.some((command) => command.kind === 'step_cancel'), 'the second activation cancels');
    assert.equal(env.history.filter((event) => event.kind === 'step.outcome').length, 0, 'the cancel journals no outcome');
    if (ending === 'completed') env.completeStep(1, 'captured');
    else env.failStep(1, 'CANCELLED', 'cancelled');
    const outcome = await env.runUntilBlocked();
    assert.deepEqual(outcome.output, ending === 'completed' ? { output: 'captured' } : { error: 'CommandCancelled' });
    assert.equal((await env.replay()).verdict, 'MATCH');
  }
});

test('a child cancelled before its run starts ends cancelled at once; a started one waits for its run', async () => {
  const flow = `async (ctx) => {
    const shipping = ctx.child('shipping', {});
    await ctx.sleep(1000);
    ctx.requestCancellation(shipping);
    try { return await shipping; } catch (error) { return error.name + ':' + error.status; }
  }`;
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  assert.equal((await env.runUntilBlocked()).output, 'ChildWorkflowError:cancelled');
  const finished = env.history.find((event) => event.kind === 'child.finished');
  assert.equal(finished?.origin, 'effect');
  const started = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  started.onTaskStarted = (task) => { if (task === 2) started.beginStep(1); };
  assert.equal((await started.runUntilBlocked()).status, 'RUNNING');
  assert.equal(started.history.filter((event) => event.kind === 'child.finished').length, 0);
  assert.throws(() => started.beginStep(2), /timer_start step has no attempt/);
});

test('a run starts with run.started, then its delay, then the signal or update it was started with', async () => {
  const flow: TestWorkflow = async (ctx) => {
    const seen: string[] = [];
    ctx.onSignal('poke', (payload) => { seen.push(`signal:${String(payload)}`); });
    ctx.onUpdate('add', () => undefined, async (args) => { seen.push(`update:${String(args)}`); return seen.length; });
    await ctx.sleep(1);
    return seen;
  };
  const signalled = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null, { signal: { name: 'poke', payload: 'p' } });
  assert.deepEqual(signalled.history.map((event) => [event.seq, event.kind]), [[1, 'run.started'], [2, 'signal.buffered']]);
  assert.deepEqual((await signalled.runUntilBlocked()).output, ['signal:p']);
  const updated = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null, { update: { name: 'add', args: 'u' } });
  assert.deepEqual(updated.history.map((event) => [event.seq, event.kind]), [[1, 'run.started'], [2, 'update.admitted']]);
  assert.deepEqual((await updated.runUntilBlocked()).output, ['update:u']);
  // A delayed start journals run.start_delayed, which is not visible: the history skips its sequence.
  const delayed = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow', startUs: 1_000_000_000 })
    .start(null, { delayMs: 60_000, signal: { name: 'poke', payload: 'd' } });
  assert.deepEqual(delayed.history.map((event) => [event.seq, event.kind]), [[1, 'run.started'], [3, 'signal.buffered']]);
  assert.deepEqual((await delayed.runUntilBlocked()).output, ['signal:d']);
  const firstTask = delayed.history.find((event) => event.kind === 'workflow_task.started')!;
  assert.ok((firstTask.atUs ?? 0) >= 1_000_000_000 + 60_000_000, 'the first workflow task waits for the delay');
  assert.equal((await delayed.replay()).verdict, 'MATCH');
});
