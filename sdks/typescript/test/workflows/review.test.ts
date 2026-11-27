import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import {
  CodeRun, CommandBatchLimitExceeded, InlineValuesExceeded, MAX_FUNCTION_METADATA_BYTES, WorkflowTestEnvironment, replayEvents,
  type HistoryEvent, type TestEnvironmentOptions,
} from '../../src/workflows/index.js';
import { metadataSize } from '../../src/workflows/context.js';
import { SEED, bundleOf, localBundle, type TestWorkflow } from './support.js';

// The findings of the correctness review of the workflow runtime (pull request 290), each held by a test.

async function both(options: Omit<TestEnvironmentOptions, 'coldReplay'>, input: unknown, drive?: (env: WorkflowTestEnvironment) => Promise<void>): Promise<WorkflowTestEnvironment[]> {
  const runs: WorkflowTestEnvironment[] = [];
  for (const coldReplay of [false, true]) {
    const env = new WorkflowTestEnvironment({ ...options, coldReplay }).start(input);
    if (drive) await drive(env); else await env.runUntilBlocked();
    runs.push(env);
  }
  return runs;
}

function commandsOf(env: WorkflowTestEnvironment): string[] {
  return env.completions.flatMap(({ completion }) => [
    ...completion.commands.map((command) => `${command.commandNumber}:${command.kind}:${command.target}`),
    ...(completion.deferredCommand ? [`deferred:${completion.deferredCommand.kind}`] : []),
  ]);
}

test('a side effect that draws from the generator leaves the run\'s stream where a replay finds it', async () => {
  const flow: TestWorkflow = async (ctx) => {
    const coin = ctx.sideEffect(() => Math.random());
    await ctx.activity('payments.pay', { id: ctx.uuid() });
    return { coin, after: ctx.uuid() };
  };
  const [cached, cold] = await both({ bundle: bundleOf({ flow }), workflowType: 'flow', activities: { 'payments.pay': () => 'paid' } }, null);
  assert.equal(cached!.outcome.status, 'COMPLETED');
  assert.equal(cold!.outcome.status, 'COMPLETED', JSON.stringify(cold!.outcome));
  assert.deepEqual(cold!.history, cached!.history);
});

test('a command inside a side effect is refused', async () => {
  const flow: TestWorkflow = async (ctx) => {
    try { ctx.sideEffect(() => ctx.sleep(1)); return 'made'; } catch (error) { return (error as Error).name; }
  };
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  assert.equal((await env.runUntilBlocked()).output, 'TypeError');
});

test('a signal handler that waits for another buffered signal takes each signal once', async () => {
  const flow: TestWorkflow = async (ctx) => {
    await ctx.sleep(10);
    const seen: unknown[] = [];
    ctx.onSignal('a', async () => { seen.push(await ctx.signal('b')); });
    seen.push(await ctx.signal('b'));
    return seen;
  };
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  env.signal('a', 'a1');
  env.signal('b', 'b1');
  await env.runUntilBlocked();
  assert.equal(env.outcome.status, 'RUNNING', 'the second wait for b has nothing to take');
  env.signal('b', 'b2');
  await env.runUntilBlocked();
  assert.deepEqual(env.outcome.output, ['b1', 'b2']);
  const consumed = env.completions.flatMap(({ completion }) => completion.commands).filter((command) => command.kind === 'consume_signals')
    .flatMap((command) => (command.body['signals'] as { seqs: number[] }[]).flatMap((entry) => entry.seqs));
  assert.equal(new Set(consumed).size, consumed.length, 'no signal is consumed twice');
});

test('aborting a scope aborts its open nested scopes, whose bodies can make no more commands there', async () => {
  const flow: TestWorkflow = async (ctx) => {
    try {
      await ctx.saga({}, async (outer) => {
        void outer.saga({}, async (inner) => { await inner.activity('a.first', 1); await inner.activity('b.second', 2); });
        await outer.activity('c.third', 3);
        throw new Error('give up');
      });
    } catch { /* compensated */ }
    await ctx.sleep(5);
    return 'done';
  };
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow', activities: { 'a.first': () => 1, 'c.third': () => 3 } }).start(null);
  await env.runUntilBlocked();
  const made = commandsOf(env);
  const abort = made.indexOf(made.find((entry) => entry.endsWith('saga_abort:s-1'))!);
  assert.ok(abort > 0, made.join(' '));
  for (const entry of made.slice(abort + 1)) assert.ok(!entry.endsWith(':s-2') && !entry.includes('b.second'), `${entry} after the abort of s-1`);
  const scoped = env.completions.flatMap(({ completion }) => completion.commands).filter((command, index, all) => index > all.findIndex((c) => c.kind === 'saga_abort') && command.sagaScope === 's-2');
  assert.deepEqual(scoped, []);
});

test('the function\'s commands may hold 48 KiB of metadata, and the call that would cross it throws', async () => {
  assert.equal(MAX_FUNCTION_METADATA_BYTES, 48 * 1024);
  const name = `a.${'x'.repeat(98)}`;
  const each = metadataSize('activity', name, { type: 'activity', activity: name, retry: { maxAttempts: 3, initialDelayMs: 100 } });
  const flow = `async (ctx) => { let made = 0; try { for (let i = 0; i < 250; i++) { void ctx.activity(${JSON.stringify(name)}, i, { retry: { maxAttempts: 3, initialDelayMs: 100 } }); made++; } } catch (error) { if (error.name !== 'CommandBatchLimitExceeded') throw error; } return made; }`;
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  assert.equal((await env.runUntilBlocked()).output, Math.floor(48 * 1024 / each));
});

test('a replay that read a withheld value stays inconclusive in later activations', async () => {
  const flow: TestWorkflow = async (ctx, input) => {
    await ctx.sleep(1);
    await ctx.activity(input.amount > 100 ? 'big.pay' : 'small.pay', null);
    return 'paid';
  };
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow', activities: { 'big.pay': () => 1 } }).start({ amount: 500 });
  await env.runUntilBlocked();
  // The export withholds the amount: the replay takes the other branch in activation 2.
  const exported = env.history.map((event) => (event.kind === 'run.started' ? { ...event, input: { amount: null }, redacted: true } : event)) as HistoryEvent[];
  const verdict = await replayEvents({ exportFormat: 1, seed: SEED, gatedEntries: [], stateMaxBytes: 0 }, exported, { bundleCode: bundleOf({ flow }).code, workflowType: 'flow' });
  assert.equal(verdict.verdict, 'INCONCLUSIVE_REDACTED');
});

test('a withheld side-effect value makes a replay inconclusive, never a mismatch', async () => {
  const flow: TestWorkflow = async (ctx) => {
    const amount = ctx.sideEffect(() => 500);
    await ctx.activity(amount > 100 ? 'big.pay' : 'small.pay', null);
    return 'paid';
  };
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow', activities: { 'big.pay': () => 1 } }).start(null);
  await env.runUntilBlocked();
  const exported = env.history.map((event) => (event.kind === 'marker.recorded' ? { ...event, value: null, redacted: true } : event)) as HistoryEvent[];
  const verdict = await replayEvents({ exportFormat: 1, seed: SEED, gatedEntries: [], stateMaxBytes: 0 }, exported, { bundleCode: bundleOf({ flow }).code, workflowType: 'flow' });
  assert.equal(verdict.verdict, 'INCONCLUSIVE_REDACTED');
});

test('the live activation applies only the events up to its start', () => {
  const bundle = localBundle({ flow: `async (ctx) => { ctx.onSignal('late', () => undefined); await ctx.sleep(1000); return 'done'; }` });
  const run = new CodeRun(bundle, { workflowType: 'flow', seed: SEED, stateMaxBytes: 0, gatedEntries: [] });
  const started: Extract<HistoryEvent, { kind: 'workflow_task.started' }> = { seq: 2, kind: 'workflow_task.started', atUs: 1 };
  const answer = run.activate([{ seq: 1, kind: 'run.started', input: null }, started, { seq: 3, kind: 'signal.buffered', name: 'late', payload: 1 }], started);
  assert.equal(answer.kind, 'completion');
  assert.deepEqual(answer.kind === 'completion' && answer.commands.map((command) => command.kind), ['timer_start']);
});

test('a signal committed during the task that returns is seen before the terminal command is accepted', async () => {
  const flow: TestWorkflow = async (ctx) => {
    let paid = false;
    ctx.onSignal('paid', () => { paid = true; });
    await ctx.sleep(1);
    return paid ? 'paid' : 'unpaid';
  };
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  let injected = false;
  env.onTaskStarted = (task) => { if (task === 2 && !injected) { injected = true; env.signal('paid', 1); } };
  await env.runUntilBlocked();
  assert.equal(env.completions[1]!.answer, 'ACCEPTED_DEFERRED', 'the terminal command waits for the unseen signal');
  assert.equal(env.outcome.status, 'COMPLETED');
  assert.equal(env.outcome.output, 'unpaid', 'the output was decided before the signal; the signal is consumed by its handler');
  // The deferred terminal command took no number, so the consumption of the next activation is command 2.
  assert.deepEqual(commandsOf(env).slice(-2), ['2:consume_signals:', 'deferred:complete']);
});

test('the test environment refuses what the server refuses', async () => {
  // A consumption of a signal that is not pending, a re-emission that differs from its deferral: under review.
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow: 'async (ctx) => { await ctx.sleep(1); return 1; }' }), workflowType: 'flow' }).start(null);
  assert.equal(env.check({ kind: 'consume_signals', target: '', body: { signals: [{ name: 'x', seqs: [99] }] } }), 'COMMAND_INVALID');
  assert.equal(env.check({ kind: 'saga_close', target: 's-9', body: { scope: 's-9' } }), 'COMMAND_INVALID');
  assert.equal(env.check({ kind: 'update_accept', target: '7', body: { admittedSeq: 7, accepted: true } }), 'COMMAND_INVALID');
});

test('cancelling a child, a wait or an approval rejects its promise', async () => {
  const flow: TestWorkflow = async (ctx) => {
    const outcomes: string[] = [];
    const settle = async (promise: Promise<unknown>): Promise<void> => { try { await promise; outcomes.push('settled'); } catch (error) { outcomes.push((error as Error).name); } };
    const child = ctx.child('shipping', {});
    const wait = ctx.waitForEvent('paid', 'k', { timeoutMs: 60_000 });
    const decision = ctx.approval('large', {});
    for (const promise of [child, wait, decision]) ctx.requestCancellation(promise);
    await Promise.all([settle(child), settle(wait), settle(decision)]);
    return outcomes;
  };
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  assert.deepEqual((await env.runUntilBlocked()).output, ['ChildWorkflowError', 'CommandCancelled', 'CommandCancelled']);
});

test('cancelling a sleep that already fired is a timer_cancel no-op', async () => {
  const flow: TestWorkflow = async (ctx) => {
    const nap = ctx.sleep(1);
    await nap;
    ctx.requestCancellation(nap);
    return 'woke';
  };
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  await env.runUntilBlocked();
  assert.deepEqual(commandsOf(env), ['1:timer_start:c-1', '2:timer_cancel:c-1', '3:complete:']);
});

test('a failure message is bounded and a wait\'s literal key counts as an inline value', async () => {
  const failing = 'async () => { throw new WorkflowFailure("TOO_LONG", "x".repeat(400000)); }';
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow: failing }), workflowType: 'flow' }).start(null);
  const outcome = await env.runUntilBlocked();
  assert.equal(outcome.failure?.code, 'TOO_LONG');
  assert.equal(outcome.failure?.message.length, 4096);
  const keyed = `async (ctx) => { try { void ctx.waitForEvent('paid', 'k'.repeat(200000), { timeoutMs: 1000 }); return 'made'; } catch (error) { return error.name; } }`;
  const second = new WorkflowTestEnvironment({ bundle: bundleOf({ flow: keyed }), workflowType: 'flow' }).start(null);
  assert.equal((await second.runUntilBlocked()).output, 'InlineValuesExceeded');
});

test('an update handler may continue as new, and a validator may change nothing', async () => {
  const handler = `async (ctx) => { ctx.onUpdate('roll', () => undefined, async () => ctx.continueAsNew({ n: 2 })); await ctx.signal('never'); }`;
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow: handler }), workflowType: 'flow' }).start(null);
  env.update('roll', {});
  assert.equal((await env.runUntilBlocked()).status, 'CONTINUED_AS_NEW');
  const validator = `async (ctx) => { ctx.onUpdate('peek', () => { ctx.setState({ seen: true }); }, async () => null); await ctx.signal('never'); }`;
  const second = new WorkflowTestEnvironment({ bundle: bundleOf({ flow: validator }), workflowType: 'flow', stateMaxBytes: 1024 }).start(null);
  second.update('peek', {});
  assert.equal((await second.runUntilBlocked()).review?.cause, 'VALIDATOR_COMMAND');
  const asynchronous = `async (ctx) => { ctx.onUpdate('slow', async () => undefined, async () => 'ran'); await ctx.signal('never'); }`;
  const third = new WorkflowTestEnvironment({ bundle: bundleOf({ flow: asynchronous }), workflowType: 'flow' }).start(null);
  const seq = third.update('slow', {});
  await third.runUntilBlocked();
  assert.deepEqual(third.updates.get(seq), { name: 'slow', status: 'REJECTED', reason: 'VALIDATION_FAILED' });
});

test('a successor reads the state its predecessor carried', async () => {
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow: 'async (ctx, input) => ({ input, carried: ctx.carriedState() })' }), workflowType: 'flow' })
    .start({ cursor: 8 }, { carriedState: { seen: 7 } });
  assert.deepEqual((await env.runUntilBlocked()).output, { input: { cursor: 8 }, carried: { seen: 7 } });
});

test('the command limit is judged before the values, and a refused call records no value', async () => {
  // The 257th call carries 3 MiB, which would travel as a blob: it is refused by the command limit, and its bytes must
  // not count, or the 2 MB output could no longer be carried (4 MiB of blobs at most).
  const flow = `async (ctx) => {
    for (let i = 0; i < 256; i++) void ctx.activity('fan.out', i);
    let first;
    try { void ctx.activity('fan.out', 'x'.repeat(3000000)); } catch (error) { first = error.name; }
    return { first, pad: 'y'.repeat(2000000) };
  }`;
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  const outcome = await env.runUntilBlocked();
  assert.equal(outcome.status, 'COMPLETED', JSON.stringify(outcome.review));
  assert.equal((outcome.output as { first: string }).first, 'CommandBatchLimitExceeded');
});

test('sequential saga scopes in one activation are bounded, and a seventeenth open scope throws', async () => {
  const flow = `async (ctx) => {
    let closed = 0;
    try { for (let i = 0; i < 40; i++) { await ctx.saga({}, async () => null); closed++; } } catch (error) { if (error.name !== 'CommandBatchLimitExceeded') throw error; }
    const open = [];
    let refused = 'no';
    try { for (let i = 0; i < 17; i++) open.push(ctx.saga({}, () => new Promise(() => undefined))); } catch (error) { refused = error.name; }
    return { closed, refused };
  }`;
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  assert.deepEqual((await env.runUntilBlocked()).output, { closed: 16, refused: 'CommandBatchLimitExceeded' });
});

test('a rich run makes the same history cached and cold', async () => {
  const flow: TestWorkflow = async (ctx, input) => {
    let total = 0;
    ctx.onSignal('add', (n) => { total += n as number; });
    ctx.onUpdate('peek', () => undefined, async () => { await ctx.sleep(5); return total; });
    const id = ctx.sideEffect(() => ctx.uuid());
    try {
      await ctx.saga({}, async (scope) => { await scope.activity('inventory.reserve', id); throw new Error('declined'); });
    } catch { /* compensated */ }
    if (ctx.patched('v2')) total += 1;
    await ctx.sleep(10);
    ctx.setState({ total });
    if (input.again) ctx.continueAsNew({ again: false }, { state: { total } });
    return total;
  };
  const options = { bundle: bundleOf({ flow }), workflowType: 'flow', stateMaxBytes: 1024, activities: { 'inventory.reserve': () => 'r' } };
  const [cached, cold] = await both(options, { again: true }, async (env) => {
    env.signal('add', 2);
    env.update('peek', {});
    await env.runUntilBlocked();
  });
  assert.equal(cached!.outcome.status, 'CONTINUED_AS_NEW');
  assert.deepEqual(cold!.history, cached!.history);
  assert.deepEqual(cold!.outcome, cached!.outcome);
  const verdict = await cached!.replay();
  assert.equal(verdict.verdict, 'MATCH');
  assert.equal(verdict.verdict === 'MATCH' && verdict.chain, cached!.chain, 'the replayer\'s chain is the one the server holds');
});

test('a gated call with signals buffered in an earlier activation throws at the same call live and in replay', async () => {
  const flow: TestWorkflow = async (ctx, input) => {
    await ctx.sleep(1);
    ctx.onSignal('tick', () => undefined);
    try { void ctx.activity('payments.charge', { pad: input.pad }); return 'fit'; } catch (error) { if (error instanceof InlineValuesExceeded) return 'refused'; throw error; }
  };
  const pad = 'x'.repeat(192 * 1024 - 64 * 1024 - 1000);
  const [cached, cold] = await both({ bundle: bundleOf({ flow }), workflowType: 'flow', stateMaxBytes: 64 * 1024, gatedEntries: ['payments.charge'] }, { pad }, async (env) => {
    for (let i = 0; i < 400; i++) env.signal('tick', i);
    await env.runUntilBlocked();
  });
  assert.equal(cached!.outcome.output, 'refused');
  assert.deepEqual(cold!.history, cached!.history);
  void CommandBatchLimitExceeded;
});
