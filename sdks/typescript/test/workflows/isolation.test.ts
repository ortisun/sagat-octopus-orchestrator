import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { WorkflowTestEnvironment, WorkflowThread, type HistoryEvent } from '../../src/workflows/index.js';
import { WorkflowSandbox } from '../../src/workflows/sandbox.js';
import { SEED, bundleOf, localBundle } from './support.js';

// The findings of the security review of the workflow context (pull request 290), each held by a test.

async function outputOf(flow: string, options: { stateMaxBytes?: number } = {}): Promise<unknown> {
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow', ...options }).start(null);
  const outcome = await env.runUntilBlocked();
  assert.equal(outcome.status, 'COMPLETED', JSON.stringify(outcome));
  return outcome.output;
}

test('a dynamic import in workflow code reaches no object of the host', async () => {
  // A bundle made by hand (the bundler turns import() into a failure inside the context): Node's refusal would be an
  // error of the host's realm, from which the host's Function and process are reachable; the context gets a string.
  const prelude = `import('node:fs').catch((error) => {
    let reach;
    try { reach = typeof error.constructor.constructor('return process')(); } catch (refused) { reach = 'refused ' + refused.name; }
    globalThis.__reach = typeof error + ':' + reach;
  });`;
  const sandbox = new WorkflowSandbox(localBundle({ noop: 'async () => null' }, prelude), 2000);
  await new Promise((resolve) => setImmediate(resolve));
  sandbox.evaluate('0');
  assert.equal(sandbox.evaluate('globalThis.__reach'), 'string:refused EvalError');
});

test('the bridge input cannot be turned into a setter that runs outside the deadline', async () => {
  const thread = new WorkflowThread({ requestTimeoutMs: 10_000 });
  try {
    const hostile = `Object.defineProperty(globalThis, '__sagat_in', { set() { for (;;) {} }, configurable: false });`;
    await assert.rejects(thread.loadBundle(bundleOf({ noop: 'async () => null' }, hostile).code), (error: unknown) => (error as Error).name === 'InvalidWorkflowBundle');
  } finally {
    await thread.terminate();
  }
});

test('a request the thread never answers ends the thread instead of hanging', async () => {
  const thread = new WorkflowThread({ requestTimeoutMs: 200 });
  try {
    const bundle = await thread.loadBundle(bundleOf({ busy: 'async () => { const end = Date.now(); let i = 0; while (i < 3e9) i++; return end; }' }).code);
    const run = await thread.createRun(bundle, { workflowType: 'busy', seed: SEED, stateMaxBytes: 0, gatedEntries: [] }, { activationDeadlineMs: 60_000 });
    const started: Extract<HistoryEvent, { kind: 'workflow_task.started' }> = { seq: 2, kind: 'workflow_task.started', atUs: 1 };
    await assert.rejects(run.activate([{ seq: 1, kind: 'run.started', input: null }, started], started), (error: unknown) => (error as Error).name === 'WorkflowThreadStuck');
    assert.equal(run.broken, true);
  } finally {
    await thread.terminate();
  }
});

test('a value that cannot be hashed fails the workflow task inside the context, and its text stays there', async () => {
  // 256 levels of nesting fit the value alone but not the body that carries it.
  const flow = `async (ctx) => { let v = 0; for (let i = 0; i < 256; i++) v = { customer_x: v }; ctx.setState(v); return 'set'; }`;
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow', stateMaxBytes: 65536 }).start(null);
  const outcome = await env.runUntilBlocked();
  assert.equal(outcome.status, 'UNDER_REVIEW');
  assert.deepEqual(outcome.review, { cause: 'WORKFLOW_TASK_FAILED', failure: { cause: 'WORKFLOW_TASK_FAILED', exceptionType: 'TypeError' } });
  assert.doesNotMatch(JSON.stringify(outcome), /customer_x/);
});

test('an answer the context tampered with fails the workflow task as a runtime risk and drops the run', async () => {
  // The workflow replaces JSON.stringify, which the bridge uses: its answer no longer holds a well-formed command list.
  const flow = `async (ctx) => { const original = JSON.stringify; JSON.stringify = (value) => original(value).replace('"kind":"complete"', '"kind":"launch"'); return 'done'; }`;
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  const outcome = await env.runUntilBlocked();
  assert.equal(outcome.status, 'UNDER_REVIEW');
  assert.equal(outcome.review?.cause, 'NONDETERMINISM_RISK');
});

test('free text of workflow code never crosses back in an error', async () => {
  await assert.rejects(WorkflowThread.shared().loadBundle(bundleOf({ noop: 'async () => null' }, `throw new Error('4111 1111 1111 1111');`).code),
    (error: unknown) => (error as Error).name === 'InvalidWorkflowBundle' && !/4111/.test((error as Error).message));
});

test('a thrown object that looks like a timeout is not an activation deadline', async () => {
  await assert.rejects(WorkflowThread.shared().loadBundle(bundleOf({ noop: 'async () => null' }, `throw { code: 'ERR_SCRIPT_EXECUTION_TIMEOUT' };`).code),
    (error: unknown) => (error as Error).name === 'InvalidWorkflowBundle');
});

test('an exception type is checked by the host, whatever the context did to its own checks', async () => {
  const flow = `async () => { RegExp.prototype.test = () => true; class Smuggled extends Error {} Object.defineProperty(Smuggled, 'name', { value: 'card 4111 1111' }); throw new Smuggled('x'); }`;
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow }), workflowType: 'flow' }).start(null);
  const outcome = await env.runUntilBlocked();
  assert.equal(outcome.status, 'UNDER_REVIEW');
  assert.deepEqual(outcome.review?.failure, { cause: 'WORKFLOW_TASK_FAILED' });
});

test('a value changed after its call is not the value the call made', async () => {
  const output = await outputOf(`async (ctx) => { const state = { n: 1 }; ctx.setState(state); state.pad = 'x'.repeat(100000); return 'kept'; }`, { stateMaxBytes: 1024 });
  assert.equal(output, 'kept');
  const env = new WorkflowTestEnvironment({ bundle: bundleOf({ flow: `async (ctx) => { const state = { n: 1 }; ctx.setState(state); state.pad = 'x'.repeat(100000); return 'kept'; }` }), workflowType: 'flow', stateMaxBytes: 1024 }).start(null);
  await env.runUntilBlocked();
  assert.deepEqual(env.outcome.state, { n: 1 });
});

test('the clock cannot be read through Intl or Temporal', async () => {
  const output = await outputOf(`async (ctx) => [
    new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', year: 'numeric', month: 'numeric', day: 'numeric', hour: 'numeric', minute: 'numeric', second: 'numeric', fractionalSecondDigits: 3 }).format()
      === new Intl.DateTimeFormat('en-US', { timeZone: 'UTC', year: 'numeric', month: 'numeric', day: 'numeric', hour: 'numeric', minute: 'numeric', second: 'numeric', fractionalSecondDigits: 3 }).format(ctx.now()),
    typeof Temporal === 'undefined' || Temporal.Now.instant().epochMilliseconds === ctx.now().getTime(),
  ]`);
  assert.deepEqual(output, [true, true]);
});

test('stack traces in workflow code carry no frame of the host', async () => {
  const output = await outputOf(`async () => new Error('where').stack`) as string;
  assert.match(output, /^Error: where/);
  assert.doesNotMatch(output, /file:|node:|\/Users\/|sagat-bridge/);
});

test('WebAssembly and shared memory are absent', async () => {
  assert.deepEqual(await outputOf(`async () => [typeof WebAssembly, typeof SharedArrayBuffer, typeof Atomics]`), ['undefined', 'undefined', 'undefined']);
});

test('a run that exhausts its thread\'s heap ends that thread only', async () => {
  const thread = new WorkflowThread({ maxOldGenerationSizeMb: 96 });
  try {
    const bundle = await thread.loadBundle(bundleOf({ hog: 'async () => { const kept = []; for (;;) kept.push(new Array(1e5).fill({ x: 1 })); }' }).code);
    const run = await thread.createRun(bundle, { workflowType: 'hog', seed: SEED, stateMaxBytes: 0, gatedEntries: [] }, { activationDeadlineMs: 30_000 });
    const started: Extract<HistoryEvent, { kind: 'workflow_task.started' }> = { seq: 2, kind: 'workflow_task.started', atUs: 1 };
    await assert.rejects(run.activate([{ seq: 1, kind: 'run.started', input: null }, started], started), (error: unknown) => (error as Error).name === 'WorkflowThreadExited');
    assert.equal(run.broken, true);
    assert.equal(thread.dead, true);
  } finally {
    await thread.terminate();
  }
  // This process and the shared thread go on.
  assert.equal(await outputOf('async () => "alive"'), 'alive');
});
