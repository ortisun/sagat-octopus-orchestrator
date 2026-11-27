import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import {
  CommandBatchLimitExceeded, InlineValuesExceeded, MAX_INLINE_BYTES, SagaCompensated, WorkflowFailure, WorkflowTestEnvironment, prepare,
  type TestEnvironmentOptions, type WorkflowCommand,
} from '../../src/workflows/index.js';
import { bundleOf, type TestWorkflow } from './support.js';

async function run(options: TestEnvironmentOptions, input: unknown, before?: (env: WorkflowTestEnvironment) => void): Promise<WorkflowTestEnvironment> {
  const env = new WorkflowTestEnvironment(options).start(input);
  before?.(env);
  await env.runUntilBlocked();
  return env;
}

function kindsOf(env: WorkflowTestEnvironment): string[][] {
  return env.completions.map(({ completion }) => [
    ...completion.commands.map((command) => `${command.commandNumber}:${command.kind}:${command.target}`),
    ...(completion.deferredCommand ? [`deferred:${completion.deferredCommand.kind}`] : []),
  ]);
}

const kilobytes = (n: number): string => 'x'.repeat(n * 1024);

test('a throw inside a saga body aborts the scope first', async () => {
  const flow: TestWorkflow = async (ctx) => {
    try {
      await ctx.saga({ onCompensationFailure: 'manual_review' }, async (saga) => {
        await saga.activity('inventory.reserve', 1);
        throw new Error('the card was declined');
      });
      return 'kept';
    } catch (error) {
      return { compensated: error instanceof SagaCompensated, cause: (error as Error & { cause?: Error }).cause?.message };
    }
  };
  const env = await run({ bundle: bundleOf({ flow }), workflowType: 'flow', activities: { 'inventory.reserve': () => 'r' } }, null);
  assert.deepEqual(env.outcome.output, { compensated: true, cause: 'the card was declined' });
  assert.deepEqual(kindsOf(env), [
    ['1:saga_open:s-1', '2:activity:inventory.reserve'],
    ['3:saga_abort:s-1'],
    ['4:complete:'],
  ]);
  const open = env.completions[0]!.completion.commands[1]!;
  assert.equal(open.sagaScope, 's-1', 'an activity of the scope names its scope, beside its fragment');
  assert.equal(open.body['scope'], undefined);
  assert.equal((await env.replay()).verdict, 'MATCH');
});

test('close saga waits for outstanding members', async () => {
  const flow: TestWorkflow = async (ctx) => {
    const value = await ctx.saga({}, async (saga) => {
      void saga.activity('inventory.reserve', 1);
      await saga.activity('payments.charge', 2);
      return 'body done';
    });
    return value;
  };
  // The reserve has no stub: it stays open after the charge completed and the body returned.
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow', activities: { 'payments.charge': () => 'c' } }).start(null);
  await env.runUntilBlocked();
  assert.equal(env.outcome.status, 'RUNNING');
  assert.deepEqual(kindsOf(env), [['1:saga_open:s-1', '2:activity:inventory.reserve', '3:activity:payments.charge'], []]);
  env.completeStep(2, 'r');
  await env.runUntilBlocked();
  assert.equal(env.outcome.output, 'body done');
  assert.deepEqual(kindsOf(env)[2], ['4:saga_close:s-1', '5:complete:']);
  assert.equal((await env.replay()).verdict, 'MATCH');
});

test('a terminal command aborts every open scope first', async () => {
  const flow: TestWorkflow = async (ctx) => {
    void ctx.saga({}, async (outer) => {
      await outer.activity('a.first', 1);
      await outer.saga({}, async (inner) => { await inner.activity('b.second', 2); await new Promise(() => undefined); });
    });
    await ctx.sleep(10);
    await ctx.sleep(10);
    throw new WorkflowFailure('GIVE_UP', 'stop');
  };
  const env = await run({ bundle: bundleOf({ flow }), workflowType: 'flow', activities: { 'a.first': () => 1, 'b.second': () => 2 } }, null);
  assert.equal(env.outcome.status, 'FAILED');
  const terminal = env.completions.find(({ completion }) => completion.commands.some((command) => command.kind === 'fail'))!.completion.commands;
  // Innermost first, in the completion of the terminal command, which the server defers until the scopes settle.
  assert.deepEqual(terminal.map((command) => `${command.kind}:${command.target}`).slice(-3), ['saga_abort:s-4', 'saga_abort:s-1', 'fail:']);
  assert.equal(env.completions.find(({ completion }) => completion.commands.some((command) => command.kind === 'fail'))!.answer, 'ACCEPTED_DEFERRED');
  assert.equal(env.completions.at(-1)!.completion.deferredCommand?.kind, 'fail', 'the deferred command is re-emitted last');
  assert.equal((await env.replay()).verdict, 'MATCH');
});

test('signals buffered before the function waits are consumed by its wait', async () => {
  const flow: TestWorkflow = async (ctx) => {
    await ctx.sleep(1000);
    const first = await ctx.signal('paid');
    const second = await ctx.signal('paid');
    return [first, second];
  };
  const env = await run({ bundle: bundleOf({ flow }), workflowType: 'flow' }, null, (e) => { e.signal('paid', 1); e.signal('paid', 2); });
  assert.deepEqual(env.outcome.output, [1, 2]);
  const consumption = env.completions.flatMap(({ completion }) => completion.commands).filter((command) => command.kind === 'consume_signals');
  assert.deepEqual(consumption.map((command) => command.body), [{ signals: [{ name: 'paid', seqs: [2, 3] }] }]);
  assert.equal((await env.replay()).verdict, 'MATCH');
});

test('an update is validated when admitted, accepted, handled and completed', async () => {
  const flow: TestWorkflow = async (ctx) => {
    let total = 0;
    ctx.onUpdate('addLine', (args: any) => { if (typeof args?.amount !== 'number') throw new Error('no amount'); }, async (args: any) => {
      const price = await ctx.activity('pricing.quote', args);
      total += price as number;
      return { total };
    });
    await ctx.signal('close');
    return total;
  };
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow', activities: { 'pricing.quote': (args) => (args as { amount: number }).amount * 2 } }).start(null);
  const good = env.update('addLine', { amount: 5 });
  const bad = env.update('addLine', { nothing: true });
  const unknown = env.update('removeLine', {});
  await env.runUntilBlocked();
  env.signal('close', null);
  await env.runUntilBlocked();
  assert.equal(env.outcome.output, 10);
  assert.deepEqual(env.updates.get(good), { name: 'addLine', status: 'COMPLETED', result: { total: 10 } });
  assert.deepEqual(env.updates.get(bad), { name: 'addLine', status: 'REJECTED', reason: 'VALIDATION_FAILED' });
  assert.deepEqual(env.updates.get(unknown), { name: 'removeLine', status: 'REJECTED', reason: 'HANDLER_NOT_REGISTERED' });
  assert.equal((await env.replay()).verdict, 'MATCH');
});

test('a validator that makes a command fails the workflow task with VALIDATOR_COMMAND', async () => {
  const flow: TestWorkflow = async (ctx) => {
    ctx.onUpdate('sneaky', () => { void ctx.activity('payments.charge', 1); }, async () => null);
    await ctx.signal('never');
  };
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  env.update('sneaky', {});
  await env.runUntilBlocked();
  assert.equal(env.outcome.status, 'UNDER_REVIEW');
  assert.equal(env.outcome.review?.cause, 'VALIDATOR_COMMAND');
});

test('a cancellation request reaches the function, which acknowledges it after its cleanup', async () => {
  const flow: TestWorkflow = async (ctx) => {
    await Promise.race([ctx.sleep(3_600_000), ctx.cancellation.promise]);
    if (ctx.cancellation.requested) {
      await ctx.activity('cleanup.release', 1);
      ctx.acknowledgeCancellation();
    }
    return 'slept';
  };
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow', activities: { 'cleanup.release': () => 'ok' } }).start(null);
  await env.runUntilBlocked(1);
  env.cancel();
  await env.runUntilBlocked();
  assert.equal(env.outcome.status, 'CANCELLED');
  assert.deepEqual(kindsOf(env).at(-1), ['3:acknowledge_cancellation:']);
  assert.equal((await env.replay()).verdict, 'MATCH');
});

test('continue-as-new waits for open activities and carries its input and state', async () => {
  const flow: TestWorkflow = async (ctx, input) => {
    void ctx.activity('audit.write', input);
    ctx.continueAsNew({ cursor: input.cursor + 1 }, { state: { seen: input.cursor } });
  };
  let release!: () => void;
  const written = new Promise<void>((resolve) => { release = resolve; });
  const env = new WorkflowTestEnvironment({
    bundle: bundleOf({ flow }), workflowType: 'flow', stateMaxBytes: 1024,
    activities: { 'audit.write': async () => { await written; return 'ok'; } },
  }).start({ cursor: 7 });
  const pending = env.runUntilBlocked();
  release();
  await pending;
  assert.equal(env.outcome.status, 'CONTINUED_AS_NEW');
  assert.deepEqual(env.outcome.continuedAs, { input: { cursor: 8 }, version: 'same', state: { seen: 7 } });
  assert.equal(env.completions[0]!.answer, 'ACCEPTED_DEFERRED', 'deferred while the activity is open');
  assert.equal(env.completions.at(-1)!.completion.deferredCommand?.kind, 'continue_as_new');
});

test('carried state over the current version\'s maxBytes throws at continueAsNew', async () => {
  const flow: TestWorkflow = async (ctx) => {
    try {
      ctx.continueAsNew({}, { state: { blob: 'x'.repeat(2048) } });
    } catch (error) {
      if (error instanceof InlineValuesExceeded) return 'refused';
      throw error;
    }
  };
  const env = await run({ bundle: bundleOf({ flow }), workflowType: 'flow', stateMaxBytes: 1024 }, null);
  assert.equal(env.outcome.output, 'refused');
});

test('the 257th command of an activation throws CommandBatchLimitExceeded at the same call live and in replay', async () => {
  const flow: TestWorkflow = async (ctx) => {
    let made = 0;
    try {
      for (let i = 0; i < 300; i++) { void ctx.activity('fan.out', i); made++; }
    } catch (error) {
      if (!(error instanceof CommandBatchLimitExceeded)) throw error;
    }
    return made;
  };
  const env = await run({ bundle: bundleOf({ flow }), workflowType: 'flow', activities: {} }, null);
  assert.equal(env.outcome.output, 256);
  // The function's 256 activities, then the terminal command, which the limit does not refuse.
  assert.equal(env.completions[0]!.completion.commands.length, 257);
  const env2 = await run({ bundle: bundleOf({ flow: async (ctx) => { let made = 0; try { for (let i = 0; i < 300; i++) { await ctx.sideEffect(() => i); made++; } } catch (error) { if (!(error instanceof CommandBatchLimitExceeded)) throw error; } return made; } }), workflowType: 'flow', coldReplay: true }, null);
  assert.equal(env2.outcome.output, 256);
  assert.equal((await env2.replay()).verdict, 'MATCH');
});

test('a gated call crossing the inline budget less the state reserve throws', async () => {
  // 192 KiB of inline values, less the declared state's 64 KiB reserve, leaves 128 KiB for gated inputs.
  const flow: TestWorkflow = async (ctx, input) => {
    const made: string[] = [];
    try {
      for (let i = 0; i < 4; i++) { void ctx.activity('payments.charge', { pad: input.pad }); made.push(`gated-${i}`); }
    } catch (error) {
      if (!(error instanceof InlineValuesExceeded)) throw error;
      made.push('refused');
    }
    return made;
  };
  const options = { bundle: bundleOf({ flow }), workflowType: 'flow', stateMaxBytes: 64 * 1024, gatedEntries: ['payments.charge'], coldReplay: true };
  const env = await run(options, { pad: kilobytes(50) });
  // Two inputs of a little over 50 KiB fit 128 KiB; the third would cross it.
  assert.deepEqual(env.outcome.output, ['gated-0', 'gated-1', 'refused']);
  assert.deepEqual(env.completions[0]!.completion.commands.map((command) => command.kind), ['activity', 'activity', 'complete']);
  assert.equal((await env.replay()).verdict, 'MATCH');
  const ungated = await run({ ...options, gatedEntries: [] }, { pad: kilobytes(50) });
  assert.equal(ungated.completions[0]!.completion.commands.length, 5, 'ungated inputs may travel as blobs');
});

test('a gated call with a long consumption list throws at the same call live and in replay', async () => {
  // The reserve counts the consumption list of every signal that could be consumed in the activation, known from the
  // history at its start (Task 044-37): with many pending signals, the gated call that fit before now throws.
  const flow: TestWorkflow = async (ctx, input) => {
    ctx.onSignal('tick', () => undefined);
    try {
      void ctx.activity('payments.charge', { pad: input.pad });
      return 'fit';
    } catch (error) {
      if (error instanceof InlineValuesExceeded) return 'refused';
      throw error;
    }
  };
  // The input takes all but about 1 KiB of the 128 KiB left after the state's reserve; 400 pending signals' list does not fit.
  const pad = 'x'.repeat(MAX_INLINE_BYTES - 64 * 1024 - 1000);
  const options = { bundle: bundleOf({ flow }), workflowType: 'flow', stateMaxBytes: 64 * 1024, gatedEntries: ['payments.charge'] };
  const quiet = await run(options, { pad });
  assert.equal(quiet.outcome.output, 'fit');
  const busy = await run(options, { pad }, (env) => { for (let i = 0; i < 400; i++) env.signal('tick', i); });
  assert.equal(busy.outcome.output, 'refused');
  assert.equal((await busy.replay()).verdict, 'MATCH', 'the same call throws in replay');
});

test('inline values exceeded is thrown at the same call live and in replay', async () => {
  // Values of a little over 20 KiB: 192 KiB holds nine inline and sixteen more travel as blobs; the 26th call throws.
  const flow: TestWorkflow = async (ctx, input) => {
    let made = 0;
    try {
      for (let i = 0; i < 40; i++) { void ctx.activity('store.put', { i, pad: input.pad }); made++; }
    } catch (error) {
      if (!(error instanceof InlineValuesExceeded)) throw error;
    }
    return made;
  };
  const env = await run({ bundle: bundleOf({ flow }), workflowType: 'flow', coldReplay: true, activities: { 'store.put': () => 1 } }, { pad: kilobytes(20) });
  assert.equal(env.outcome.output, 25);
  assert.equal((await env.replay()).verdict, 'MATCH');
  // The declared state's 64 KiB reserve leaves 128 KiB: six values inline, sixteen as blobs, the 23rd call throws.
  const reserved = await run({ bundle: bundleOf({ flow }), workflowType: 'flow', stateMaxBytes: 64 * 1024, activities: { 'store.put': () => 1 } }, { pad: kilobytes(20) });
  assert.equal(reserved.outcome.output, 22);
});

test('a completion over its message budget moves its largest values to blobs', () => {
  const command = (n: number, kind: WorkflowCommand['kind'], body: Record<string, unknown>, alwaysInline = false): WorkflowCommand => ({
    commandNumber: n, kind, target: kind === 'activity' ? 'store.put' : '', body, issuedBySdk: false, alwaysInline,
  });
  const prepared = prepare([
    command(1, 'activity', { type: 'activity', activity: 'store.put', input: kilobytes(100) }),
    command(2, 'activity', { type: 'activity', activity: 'store.put', input: kilobytes(60) }),
    command(3, 'activity', { type: 'activity', activity: 'store.put', input: kilobytes(40) }),
    command(4, 'set_state', { state: kilobytes(30) }, true),
  ]);
  // 230 KiB do not fit 192 KiB: the largest value (100 KiB) moves, and 130 KiB stay inline.
  assert.deepEqual(prepared.map((item) => item.valueAsBlob), [true, false, false, false]);
});

test('a gated activity input is never a blob', () => {
  const prepared = prepare([
    { commandNumber: 1, kind: 'activity', target: 'payments.charge', body: { type: 'activity', activity: 'payments.charge', input: kilobytes(120) }, issuedBySdk: false, alwaysInline: true },
    { commandNumber: 2, kind: 'activity', target: 'store.put', body: { type: 'activity', activity: 'store.put', input: kilobytes(90) }, issuedBySdk: false, alwaysInline: false },
  ]);
  assert.deepEqual(prepared.map((item) => item.valueAsBlob), [false, true]);
});

test('every command kind has the body of the corpus table', async () => {
  // The table of specs/044-workflow-as-code/contracts/goldens/README.md (Decision 044-232): the body the chain hashes,
  // here with each value field still the value (the chain puts its descriptor there).
  const flow: TestWorkflow = async (ctx) => {
    ctx.onUpdate('ok', () => undefined, async () => 'done');
    ctx.onUpdate('no', () => { throw new Error('refused'); }, async () => null);
    ctx.onSignal('poke', () => undefined);
    ctx.patched('v2');
    ctx.sideEffect(() => 7);
    const nap = ctx.sleep(5);
    ctx.requestCancellation(nap);
    void ctx.script(`sha256:${'a'.repeat(64)}`, { s: 1 }, { retry: { maxAttempts: 2 } });
    void ctx.waitForEvent('payment.confirmed', 'order-1', { timeoutMs: 1000, lookbackMs: 10 });
    void ctx.child('shipping', { order: 1 }, { cancelPolicy: 'detach', businessId: 'b-1' });
    void ctx.approval('large-payments', { amount: 9 });
    const step = ctx.activity('payments.charge', { amount: 9 }, { retry: { maxAttempts: 3 }, timeoutMs: 100 });
    ctx.requestCancellation(step);
    await ctx.saga({ cancelPolicy: 'abort_no_compensation' }, async (scope) => { await scope.activity('inventory.reserve', 1); });
    ctx.setState({ n: 1 });
    try { await ctx.saga({}, async (scope) => { scope.abort(); }); } catch { /* the scope compensated */ }
    throw new WorkflowFailure('DONE', 'all kinds made');
  };
  const env = new WorkflowTestEnvironment({
    bundle: bundleOf({ flow }), workflowType: 'flow', stateMaxBytes: 1024, activities: { 'inventory.reserve': () => 'r' },
  }).start(null);
  env.update('ok', {});
  env.update('no', {});
  env.signal('poke', 1);
  await env.runUntilBlocked();
  const bodies = new Map<string, unknown>();
  for (const { completion } of env.completions) {
    for (const command of [...completion.commands, ...(completion.deferredCommand ? [completion.deferredCommand] : [])]) {
      if (!bodies.has(command.kind)) bodies.set(command.kind, command.body);
    }
  }
  const digest = `sha256:${'a'.repeat(64)}`;
  assert.deepEqual(Object.fromEntries(bodies), {
    update_accept: { admittedSeq: 2, accepted: true },
    update_reject: { admittedSeq: 3, accepted: false, reason: 'VALIDATION_FAILED' },
    marker_patch: { markerId: 'v2' },
    marker_side_effect: { markerId: 'side-effect', value: 7 },
    timer_start: { type: 'timer', durationMs: 5 },
    timer_cancel: { target: 'c-3' },
    script: { type: 'script', artifactDigest: digest, retry: { maxAttempts: 2 }, input: { s: 1 } },
    wait_event: { type: 'waitEvent', eventType: 'payment.confirmed', correlationKey: 'order-1', timeoutMs: 1000, lookbackMs: 10 },
    child: { type: 'child', workflowId: 'shipping', input: { order: 1 }, cancelPolicy: 'detach', businessId: 'b-1' },
    approval: { type: 'approval', policyName: 'large-payments', subject: { amount: 9 } },
    activity: { type: 'activity', activity: 'payments.charge', input: { amount: 9 }, retry: { maxAttempts: 3 }, timeoutMs: 100 },
    step_cancel: { target: 'c-9' },
    saga_open: { scope: 's-11', parentScope: null, onCompensationFailure: 'manual_review', cancelPolicy: 'abort_no_compensation' },
    update_complete: { admittedSeq: 2, result: 'done' },
    consume_signals: { signals: [{ name: 'poke', seqs: [4] }] },
    saga_close: { scope: 's-11' },
    set_state: { state: { n: 1 } },
    saga_abort: { scope: 's-18' },
    fail: { code: 'DONE', message: 'all kinds made' },
  });
});
