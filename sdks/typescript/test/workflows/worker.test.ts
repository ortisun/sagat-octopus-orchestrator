import { strict as assert } from 'node:assert';
import { createHash } from 'node:crypto';
import { test } from 'node:test';
import { SagatError } from '../../src/errors.js';
import type { WorkflowTaskEvent } from '../../src/worker/workflows.js';
import type { WireHistoryEvent } from '../../src/worker/grpc.js';
import { WorkflowTestEnvironment } from '../../src/workflows/index.js';
import { bundleOf, type TestWorkflow } from './support.js';
import { golden, leaseOf, stubRpc, tasksOf, type Calls } from './worker-support.js';

// The findings of the correctness review of the worker side (research Decision 044-325), each held by a test.

const history = golden.history as WireHistoryEvent[];
const upTo = (seq: number, from = 1): WireHistoryEvent[] => history.filter((event) => event.seq >= from && event.seq <= seq);

test('a cold worker replays each page\'s completed activations before it asks for the next page', async () => {
  const calls: Calls[] = [];
  const events: WorkflowTaskEvent[] = [];
  const replayedAt: number[] = [];
  let replayed = 0;
  const log = { debug: (message: string) => { if (message === 'replayed an activation') replayed++; }, info() {}, warn() {}, error() {} };
  // Activation 4's lease; the completion of activation 2 (sequence 11) closes activation 1 within the first page.
  const rpc = stubRpc(calls, { pages: [upTo(11), upTo(16, 12)], onPage: () => replayedAt.push(replayed) });
  const tasks = tasksOf(rpc, events, log);
  try {
    await tasks.load();
    await tasks.serve(leaseOf(4, { first_page: [] }));
    assert.deepEqual(events.map(({ outcome }) => outcome), ['ACCEPTED']);
    assert.deepEqual(replayedAt, [0, 1], 'activation 1 was replayed before the second page was asked for');
  } finally {
    await tasks.close();
  }
});

test('a retriable error of ReadHistory resumes from the next sequence, and one of ReadBlob reads again', async () => {
  const value = Buffer.from('{"authorizationId":"auth-77"}');
  const blobbed = history.map((event) => (event.kind === 'HISTORY_STEP_OUTCOME' && event['step_outcome'].node === 'c-2'
    ? { ...event, step_outcome: { ...event['step_outcome'], output: { value: 'blob', blob: { blob_id: 'out', size: value.length, sha256: createHash('sha256').update(value).digest('hex') }, placeholder: '' } } }
    : event)) as WireHistoryEvent[];
  const calls: Calls[] = [];
  const events: WorkflowTaskEvent[] = [];
  const rpc = stubRpc(calls, {
    pages: [blobbed.filter((event) => event.seq <= 5), blobbed.filter((event) => event.seq > 5 && event.seq <= 9)],
    historyErrors: new Map([[1, new SagatError({ code: 'RATE_LIMITED', message: 'too many streams', retryAfterMs: 1 })]]),
    blobErrors: new Map([[1, new SagatError({ code: 'TRANSPORT', message: 'no answer' })]]),
    blobs: new Map([['out', value]]),
  });
  const tasks = tasksOf(rpc, events);
  try {
    await tasks.load();
    await tasks.serve(leaseOf(2, { first_page: [] }, blobbed));
    assert.deepEqual(events.map(({ outcome }) => outcome), ['ACCEPTED']);
    assert.deepEqual(calls.filter(({ method }) => method === 'ReadHistory').map(({ request }) => request['from_seq']), [1, 6]);
    assert.equal(calls.filter(({ method }) => method === 'ReadBlob').length, 2);
  } finally {
    await tasks.close();
  }
});

test('a report is repeated on a retriable error with the same key and body', async () => {
  const calls: Calls[] = [];
  const events: WorkflowTaskEvent[] = [];
  const tasks = tasksOf(stubRpc(calls, { completeAnswers: [new SagatError({ code: 'UNAVAILABLE', message: 'semaphore full', retryAfterMs: 1 })] }), events);
  try {
    await tasks.load();
    await tasks.serve(leaseOf(1));
    const reports = calls.filter(({ method }) => method === 'CompleteWorkflowTask');
    assert.equal(reports.length, 2);
    assert.deepEqual(reports[1]!.request, reports[0]!.request);
    assert.equal(reports[0]!.request['idempotency_key'], 'workflow-complete-wt-1-1-1');
    assert.deepEqual(events.map(({ outcome }) => outcome), ['ACCEPTED']);
  } finally {
    await tasks.close();
  }
});

test('the cache keeps a run after ACCEPTED_DEFERRED and drops it after REVIEW or a stale lease', async () => {
  const cases: Array<[Record<string, unknown> | SagatError, boolean]> = [
    [{ outcome: 'WORKFLOW_TASK_ACCEPTED_DEFERRED', cause: 'CAUSE_UNHANDLED_EVENTS', already_applied: false }, true],
    [{ outcome: 'WORKFLOW_TASK_REVIEW', cause: 'CAUSE_CHAIN_MISMATCH', already_applied: false }, false],
    [new SagatError({ code: 'STALE_ATTEMPT', message: 'not current' }), false],
  ];
  for (const [answer, kept] of cases) {
    const calls: Calls[] = [];
    const events: WorkflowTaskEvent[] = [];
    const tasks = tasksOf(stubRpc(calls, { completeAnswers: [answer] }), events);
    try {
      await tasks.load();
      await tasks.serve(leaseOf(1));
      await tasks.serve(leaseOf(2));
      assert.equal(events[1]!.cacheHit, kept, JSON.stringify(answer));
    } finally {
      await tasks.close();
    }
  }
});

test('events committed after the lease\'s start are read but never applied', async () => {
  // A signal journaled after S(1) = 2: activation 1 must not see it.
  const trailing = { seq: 3, kind: 'HISTORY_SIGNAL_BUFFERED', origin: 'HISTORY_ORIGIN_EXTERNAL', body: 'signal_buffered', at_us: 1767225600000300,
    signal_buffered: { signal_name: 'late', payload: { value: 'json', json: Buffer.from('1'), placeholder: '' } } } as unknown as WireHistoryEvent;
  const calls: Calls[] = [];
  const events: WorkflowTaskEvent[] = [];
  const tasks = tasksOf(stubRpc(calls, { pages: [[...upTo(2), trailing]] }), events);
  try {
    await tasks.load();
    await tasks.serve(leaseOf(1, { first_page: [], visible_count: 3 }));
    const report = calls.find(({ method }) => method === 'CompleteWorkflowTask')!;
    assert.equal(report.request['applied_through_seq'], 2);
    assert.deepEqual(report.request['commands'].map((command: Record<string, unknown>) => command['kind']), ['COMMAND_ACTIVITY', 'COMMAND_ACTIVITY']);
  } finally {
    await tasks.close();
  }
});

test('a history out of order is malformed and reported as a risk', async () => {
  const misplaced = history.filter((event) => event.seq <= 2).concat() as WireHistoryEvent[];
  // A command record before any completion.
  const record = { ...history.find((event) => event.seq === 4)!, seq: 2 } as WireHistoryEvent;
  const calls: Calls[] = [];
  const events: WorkflowTaskEvent[] = [];
  const tasks = tasksOf(stubRpc(calls), events);
  try {
    await tasks.load();
    const started = { ...history.find((event) => event.seq === 2)!, seq: 3 } as WireHistoryEvent;
    await tasks.serve(leaseOf(1, { first_page: [misplaced[0]!, record, started], visible_count: 3 }));
    assert.deepEqual(calls.map(({ method, request }) => [method, request['cause']]), [['FailWorkflowTask', 'CAUSE_NONDETERMINISM_RISK']]);
  } finally {
    await tasks.close();
  }
});

const cancelOne = `async (ctx) => {
  const work = ctx.activity('payments.capture', { amount: 5 });
  ctx.requestCancellation(work);
  try { return { output: await work }; } catch (error) { return { error: error.name }; }
}`;

test('in the test environment a stubbed step that its own completion cancels ends cancelled', async () => {
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow: cancelOne }), workflowType: 'flow', activities: { 'payments.capture': () => 'ok' } }).start(null);
  assert.deepEqual((await env.runUntilBlocked()).output, { error: 'CommandCancelled' });
});

test('in the test environment a child, a wait and an approval are settled by their own events', async () => {
  const flow = `async (ctx) => {
    const shipping = ctx.child('shipping', {});
    const paid = ctx.waitForEvent('paid', 'k', { timeoutMs: 60000 });
    const review = ctx.approval('large', {});
    await ctx.sleep(1);
    const out = [];
    for (const promise of [shipping, paid, review]) { try { out.push(await promise); } catch (error) { out.push(error.name + ':' + error.status); } }
    return out;
  }`;
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  // Settled while the second workflow task runs (after the sleep fired), before the wait's own timeout could fire.
  env.onTaskStarted = (task) => {
    if (task !== 2) return;
    env.beginStep(1);
    env.completeStep(1, { shipped: true });
    env.completeStep(2, { amount: 3 });
    env.decideApproval(3, 'approved');
  };
  assert.deepEqual((await env.runUntilBlocked()).output, [{ shipped: true }, { amount: 3 }, 'approved']);
  const kinds = env.history.filter((event) => ['child.finished', 'wait.outcome', 'approval.decided'].includes(event.kind)).map((event) => [event.kind, event.origin]);
  assert.deepEqual(kinds, [['child.finished', 'engine'], ['wait.outcome', 'external'], ['approval.decided', 'engine']]);
  const failing = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  await failing.runUntilBlocked();
  failing.failStep(1, 'CANCELLED', 'cancelled');
  assert.throws(() => failing.failStep(3, 'X'), /an approval is decided/);
});

test('a re-emitted deferral keeps the carrier its deferral held, cold or cached', async () => {
  const flow: TestWorkflow = async (ctx, input) => {
    ctx.onSignal('poke', () => undefined);
    ctx.onUpdate('fill', () => undefined, async () => 'u'.repeat(input.update));
    return 'o'.repeat(input.output);
  };
  for (const coldReplay of [false, true]) {
    // Held inline: a 150 KiB output deferred alone; re-emitted beside a 100 KiB update result, which moves instead.
    const inline = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow', coldReplay }).start({ output: 150 * 1024, update: 100 * 1024 });
    inline.onTaskStarted = (task) => { if (task === 1) { inline.signal('poke', null); inline.update('fill', null); } };
    await inline.runUntilBlocked();
    assert.equal(inline.completions[0]!.answer, 'ACCEPTED_DEFERRED');
    const second = inline.completions[1]!.completion;
    assert.equal(second.deferredCommand?.valueAsBlob, false, `coldReplay ${coldReplay}: held inline`);
    assert.equal(second.commands.find((command) => command.kind === 'update_complete')?.valueAsBlob, true);
    // Held as a blob: a 250 KiB output moved at its first emission is re-emitted as a blob alone.
    const blob = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow', coldReplay }).start({ output: 250 * 1024, update: 1 });
    blob.onTaskStarted = (task) => { if (task === 1) blob.signal('poke', null); };
    await blob.runUntilBlocked();
    assert.equal(blob.completions[0]!.completion.commands.at(-1)?.valueAsBlob, true);
    assert.equal(blob.completions[1]!.completion.deferredCommand?.valueAsBlob, true, `coldReplay ${coldReplay}: held as a blob`);
  }
});

test('a failure message counts in the packing as it counts in the check', async () => {
  // An input of 194,000 canonical bytes fits alone; with the message's 4,002 the input must move to a blob.
  const flow = `async (ctx) => { void ctx.activity('a.b', 'x'.repeat(193998)); throw new WorkflowFailure('DECLINED', 'm'.repeat(4000)); }`;
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  assert.equal((await env.runUntilBlocked()).status, 'FAILED');
  assert.equal(env.completions[0]!.completion.commands[0]!.valueAsBlob, true);
});

test('a wait or a script refuses a timeout outside the code profile at the call', async () => {
  const flow = `async (ctx) => {
    const out = [];
    for (const timeoutMs of [0, -5, 1.5, 366 * 86400000 + 1]) { try { void ctx.waitForEvent('e', 'k', { timeoutMs }); out.push('made'); } catch (error) { out.push(error.name); } }
    try { void ctx.waitForEvent('e', 'k', { timeoutMs: 1000, lookbackMs: 7 * 86400000 + 1 }); out.push('made'); } catch (error) { out.push(error.name); }
    try { void ctx.script('sha256:${'a'.repeat(64)}', {}, { timeoutMs: 0 }); out.push('made'); } catch (error) { out.push(error.name); }
    return out;
  }`;
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  assert.deepEqual((await env.runUntilBlocked()).output, ['RangeError', 'RangeError', 'RangeError', 'RangeError', 'RangeError', 'RangeError']);
});
