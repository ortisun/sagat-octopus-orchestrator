import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import {
  ActivationCutter, ActivityError, Replayer, StubFailure, UnsupportedExportFormat, WorkflowFailure, WorkflowTestEnvironment,
  type TestEnvironmentOptions,
} from '../../src/workflows/index.js';
import { bundleOf, type TestWorkflow } from './support.js';

const order: TestWorkflow = async (ctx, input) => {
  const reserved = await ctx.activity('inventory.reserve', { sku: input.sku });
  const charged = await ctx.activity('payments.charge', { amount: input.amount });
  await ctx.sleep(86_400_000);
  return { reserved, charged, at: ctx.now().toISOString(), id: ctx.uuid() };
};

const stubs: TestEnvironmentOptions['activities'] = {
  'inventory.reserve': (input) => ({ reservation: `r-${(input as { sku: string }).sku}` }),
  'payments.charge': (input) => ({ receipt: (input as { amount: number }).amount * 100 }),
};

async function finished(options: TestEnvironmentOptions, input: unknown): Promise<WorkflowTestEnvironment> {
  const env = new WorkflowTestEnvironment(options).start(input);
  await env.runUntilBlocked();
  return env;
}

test('a run replayed from its history makes the same commands, cold or cached', async () => {
  const bundle = bundleOf({ order });
  const cached = await finished({ bundle, workflowType: 'order', activities: stubs }, { sku: 'A-1', amount: 3 });
  const cold = await finished({ bundle, workflowType: 'order', activities: stubs, coldReplay: true }, { sku: 'A-1', amount: 3 });
  assert.equal(cached.outcome.status, 'COMPLETED');
  assert.deepEqual(cold.outcome, cached.outcome);
  assert.deepEqual(cold.history, cached.history);
  // Every completion carried the chain the server holds, which the environment checks like the server.
  const verdict = await cached.replay();
  assert.equal(verdict.verdict, 'MATCH');
  assert.equal(verdict.verdict === 'MATCH' && verdict.commands, 4);
  const records = cached.history.filter((event) => event.kind === 'command.record').map((event) => event.kind === 'command.record' && [event.commandNumber, event.commandKind, event.target]);
  assert.deepEqual(records, [[1, 'activity', 'inventory.reserve'], [2, 'activity', 'payments.charge'], [3, 'timer_start', 'c-3'], [4, 'complete', '']]);
});

test('a replay that makes another command reports the command number, the actual kind and target', async () => {
  const env = await finished({ bundle: bundleOf({ order }), workflowType: 'order', activities: stubs }, { sku: 'A-1', amount: 3 });
  const changed = bundleOf({
    order: async (ctx, input) => {
      await ctx.activity('inventory.reserve', { sku: input.sku });
      await ctx.activity('payments.authorize', { amount: input.amount });
      return null;
    },
  });
  const verdict = await env.replay(changed);
  assert.deepEqual(verdict, {
    verdict: 'NONDETERMINISM', activation: 2, commandNumber: 2, actualKind: 'activity', actualTarget: 'payments.authorize', actualSagaScope: '',
    expectedKind: 'activity', expectedTarget: 'payments.charge', expectedSagaScope: '',
  });
});

test('a recorded command the function no longer makes is a mismatch', async () => {
  const env = await finished({ bundle: bundleOf({ order }), workflowType: 'order', activities: stubs }, { sku: 'A-1', amount: 3 });
  const shorter = bundleOf({
    order: async (ctx, input) => {
      await ctx.activity('inventory.reserve', { sku: input.sku });
      return 'stop';
    },
  });
  const verdict = await env.replay(shorter);
  // Activation 2 recorded command 2 (payments.charge); the changed function ends instead.
  assert.equal(verdict.verdict, 'NONDETERMINISM');
  assert.equal(verdict.verdict === 'NONDETERMINISM' && verdict.commandNumber, 2);
  assert.equal(verdict.verdict === 'NONDETERMINISM' && verdict.actualKind, 'complete');
});

test('patched takes the new path live and the old path on a history without its marker', async () => {
  const v1: TestWorkflow = async (ctx) => {
    await ctx.activity('reminder.send', 1);
    return 'v1';
  };
  const v2: TestWorkflow = async (ctx) => {
    if (ctx.patched('reminder-v2')) await ctx.activity('reminder.send-v2', 2);
    else await ctx.activity('reminder.send', 1);
    return ctx.patched('reminder-v2') ? 'v2' : 'v1';
  };
  const old = await finished({ bundle: bundleOf({ flow: v1 }), workflowType: 'flow', activities: { 'reminder.send': () => 'ok' } }, null);
  assert.equal((await old.replay(bundleOf({ flow: v2 }))).verdict, 'MATCH');
  const fresh = await finished({ bundle: bundleOf({ flow: v2 }), workflowType: 'flow', activities: { 'reminder.send-v2': () => 'ok' } }, null);
  assert.equal(fresh.outcome.output, 'v2');
  const markers = fresh.history.filter((event) => event.kind === 'marker.recorded');
  assert.equal(markers.length, 1, 'the second call of patched records nothing');
  assert.equal((await fresh.replay()).verdict, 'MATCH');
  // The old code replaying a history with the marker is a mismatch at the marker's command.
  const verdict = await fresh.replay(bundleOf({ flow: v1 }));
  assert.equal(verdict.verdict === 'NONDETERMINISM' && verdict.commandNumber, 1);
});

test('a side effect is computed once and replayed from its marker', async () => {
  const counter: TestWorkflow = async (ctx) => {
    const value = ctx.sideEffect(() => Math.random());
    await ctx.sleep(1000);
    return value;
  };
  const env = await finished({ bundle: bundleOf({ flow: counter }), workflowType: 'flow', coldReplay: true }, null);
  assert.equal(env.outcome.status, 'COMPLETED');
  const marker = env.history.find((event) => event.kind === 'marker.recorded');
  assert.equal(marker?.kind === 'marker.recorded' && marker.value, env.outcome.output);
  assert.equal((await env.replay()).verdict, 'MATCH');
});

test('a terminal command waits for the whole window', async () => {
  // The function returns while applying run.started; the signal of the same window is still applied (its handler
  // consumes it) before the terminal command, which comes last.
  const flow: TestWorkflow = async (ctx) => {
    ctx.onSignal('paid', () => { ctx.setState({ paid: true }); });
    return 'done';
  };
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow', stateMaxBytes: 1024 }).start(null);
  const paid = env.signal('paid', { amount: 1 });
  const outcome = await env.runUntilBlocked();
  assert.equal(outcome.status, 'COMPLETED');
  const kinds = env.completions[0]!.completion.commands.map((command) => [command.kind, command.body]);
  assert.deepEqual(kinds, [
    ['consume_signals', { signals: [{ name: 'paid', seqs: [paid] }] }],
    ['set_state', { state: { paid: true } }],
    ['complete', { output: 'done' }],
  ]);
});

test('a workflow failure ends the run with its code; an uncaught exception fails the workflow task', async () => {
  const failing: TestWorkflow = async (ctx, input) => {
    await ctx.activity('payments.charge', input);
    throw new WorkflowFailure(input.code, 'the card was declined');
  };
  const declined = await finished({ bundle: bundleOf({ flow: failing }), workflowType: 'flow', activities: { 'payments.charge': () => 1 } }, { code: 'CARD_DECLINED' });
  assert.deepEqual(declined.outcome.failure, { code: 'CARD_DECLINED', message: 'the card was declined' });
  const badCode = await finished({ bundle: bundleOf({ flow: failing }), workflowType: 'flow', activities: { 'payments.charge': () => 1 } }, { code: 'declined!' });
  assert.equal(badCode.outcome.failure?.code, 'WORKFLOW_FAILED');
  const buggy = await finished({
    bundle: bundleOf({ flow: async () => { const order: any = undefined; return order.total; } }), workflowType: 'flow',
  }, null);
  assert.equal(buggy.outcome.status, 'UNDER_REVIEW');
  assert.deepEqual(buggy.outcome.review, { cause: 'WORKFLOW_TASK_FAILED', failure: { cause: 'WORKFLOW_TASK_FAILED', exceptionType: 'TypeError' } });
  assert.equal(buggy.history.filter((event) => event.kind === 'workflow_task.started').length, 5, 'five consecutive failures, then the review');
});

test('a failed activity rejects with its code and the function can catch it', async () => {
  const flow: TestWorkflow = async (ctx) => {
    try {
      await ctx.activity('payments.charge', 1);
      return 'charged';
    } catch (error) {
      return { status: (error as ActivityError).status, code: (error as ActivityError).code, isError: error instanceof ActivityError };
    }
  };
  const env = await finished({
    bundle: bundleOf({ flow }), workflowType: 'flow', activities: { 'payments.charge': () => { throw new StubFailure('INSUFFICIENT_FUNDS'); } },
  }, null);
  assert.deepEqual(env.outcome.output, { status: 'failed', code: 'INSUFFICIENT_FUNDS', isError: true });
});

test('replay of an unknown export format is refused', async () => {
  const header = JSON.stringify({ exportFormat: 2, seed: '00'.repeat(32), gatedEntries: [], stateMaxBytes: 0 });
  await assert.rejects(Replayer.replay([header], { bundleCode: bundleOf({ order }).code, workflowType: 'order' }),
    (error: unknown) => error instanceof UnsupportedExportFormat && error.exportFormat === 2);
  await assert.rejects(Replayer.replay([JSON.stringify({ seed: '00'.repeat(32) })], { bundleCode: bundleOf({ order }).code, workflowType: 'order' }),
    UnsupportedExportFormat);
});

test('the replayer reads the lines of an export', async () => {
  const env = await finished({ bundle: bundleOf({ order }), workflowType: 'order', activities: stubs }, { sku: 'B-2', amount: 5 });
  async function* lines(): AsyncIterable<string> { for (const line of env.exportLines()) yield line; }
  const verdict = await Replayer.replay(lines(), { bundleCode: bundleOf({ order }).code, workflowType: 'order' });
  assert.equal(verdict.verdict, 'MATCH');
  assert.equal(verdict.verdict === 'MATCH' && verdict.chain.length, 64);
});

test('a history is cut into activations at the last start before each completion', () => {
  const cutter = new ActivationCutter();
  const done = cutter.push([
    { seq: 1, kind: 'run.started', input: null },
    { seq: 2, kind: 'workflow_task.started', atUs: 10 },
    { seq: 3, kind: 'signal.buffered', name: 'a' },
    { seq: 4, kind: 'workflow_task.started', atUs: 20 },
    { seq: 5, kind: 'workflow_task.completed' },
    { seq: 6, kind: 'command.record', commandNumber: 1, commandKind: 'timer_start', target: 'c-1' },
    { seq: 7, kind: 'timer.fired', commandNumber: 1 },
  ]);
  assert.deepEqual(done, []);
  const { activations, open } = cutter.flush();
  assert.equal(activations.length, 1);
  // The second start (4) is S(1): the signal at 3 belongs to activation 1, the failed task's start at 2 is skipped.
  assert.deepEqual(activations[0]!.window.map((event) => event.seq), [1, 2, 3, 4]);
  assert.equal(activations[0]!.started.seq, 4);
  assert.deepEqual(activations[0]!.records, [{ commandNumber: 1, kind: 'timer_start', target: 'c-1' }]);
  assert.deepEqual(open.window.map((event) => event.seq), [5, 6, 7]);
  assert.equal(open.windowStart, 4);
});

test('a recorded command the function no longer makes before it yields is a mismatch', async () => {
  // Activation 1 recorded two commands; the changed function makes the first and waits, so record 2 is left over.
  const both: TestWorkflow = async (ctx) => {
    await Promise.all([ctx.activity('a.first', 1), ctx.activity('b.second', 2)]);
    return 'done';
  };
  const env = await finished({ bundle: bundleOf({ flow: both }), workflowType: 'flow', activities: { 'a.first': () => 1, 'b.second': () => 2 } }, null);
  const verdict = await env.replay(bundleOf({ flow: async (ctx) => { await ctx.activity('a.first', 1); return 'done'; } }));
  assert.deepEqual(verdict, {
    verdict: 'NONDETERMINISM', activation: 1, commandNumber: 2, actualKind: '', actualTarget: '', expectedKind: 'activity', expectedTarget: 'b.second', expectedSagaScope: '',
  });
});

test('a cancelled timer is a timer_cancel command and its sleep rejects', async () => {
  const flow: TestWorkflow = async (ctx) => {
    const nap = ctx.sleep(60_000);
    const handle = setTimeout(() => undefined, 30_000);
    clearTimeout(handle);
    ctx.requestCancellation(nap);
    try {
      await nap;
      return 'slept';
    } catch (error) {
      return (error as Error).name;
    }
  };
  const env = await finished({ bundle: bundleOf({ flow }), workflowType: 'flow' }, null);
  assert.equal(env.outcome.output, 'TimerCancelled');
  assert.deepEqual(env.completions[0]!.completion.commands.map((command) => `${command.commandNumber}:${command.kind}:${command.target}`),
    ['1:timer_start:c-1', '2:timer_start:c-2', '3:timer_cancel:c-2', '4:timer_cancel:c-1', '5:complete:']);
  assert.equal((await env.replay()).verdict, 'MATCH');
});
