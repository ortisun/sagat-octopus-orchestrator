import { strict as assert } from 'node:assert';
import { createHash } from 'node:crypto';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { WorkerHost } from '../../src/index.js';
import type { WorkflowTaskEvent } from '../../src/worker/workflows.js';
import type { WireHistoryEvent } from '../../src/worker/grpc.js';
import { MalformedHistory, blobRefsOf, commandToWire, eventFromProto3Json } from '../../src/workflows/protocol.js';
import { checkActivationAnswer, type PreparedCommand } from '../../src/workflows/run.js';
import { WorkflowThread } from '../../src/workflows/threaded.js';
import { SEED, bundleOf } from './support.js';
import { golden, leaseOf, order, stubRpc, tasksOf, type Calls } from './worker-support.js';

// The findings of the security review of the worker side (research Decision 044-324), each held by a test.

test('a cached run is not reused when the lease carries other gated entries or another state bound', async () => {
  for (const change of [{ gated_entries: ['inventory.reserve'] }, { state_max_bytes: 4096 }]) {
    const calls: Calls[] = [];
    const events: WorkflowTaskEvent[] = [];
    const tasks = tasksOf(stubRpc(calls), events);
    try {
      await tasks.load();
      await tasks.serve(leaseOf(1));
      await tasks.serve(leaseOf(2, change));
      assert.deepEqual(events.map(({ outcome, cacheHit }) => [outcome, cacheHit]), [['ACCEPTED', false], ['ACCEPTED', false]], JSON.stringify(change));
      // The same lease fields are a cache hit.
      await tasks.serve(leaseOf(3, change));
      assert.equal(events[2]!.cacheHit, true);
    } finally {
      await tasks.close();
    }
  }
});

/** The golden history with c-2's output carried as a blob of this size. */
function withBlobOutput(size: number): WireHistoryEvent[] {
  return golden.history.map((event) => (event.kind === 'HISTORY_STEP_OUTCOME' && event['step_outcome'].node === 'c-2'
    ? { ...event, step_outcome: { ...event['step_outcome'], output: { value: 'blob', blob: { blob_id: 'big-1', size, sha256: 'a'.repeat(64), content_type: 'application/json' }, placeholder: '' } } }
    : event)) as WireHistoryEvent[];
}

test('a history blob over its bound is never read: the workflow task fails with HistoryValuesExceeded', async () => {
  const calls: Calls[] = [];
  const events: WorkflowTaskEvent[] = [];
  const tasks = tasksOf(stubRpc(calls), events);
  try {
    await tasks.load();
    await tasks.serve(leaseOf(2, {}, withBlobOutput(65 * 1024 * 1024)));
    assert.deepEqual(calls.map(({ method }) => method), ['FailWorkflowTask']);
    assert.equal(calls[0]!.request['cause'], 'CAUSE_WORKFLOW_TASK_FAILED');
    assert.equal(calls[0]!.request['exception_type'], 'HistoryValuesExceeded');
  } finally {
    await tasks.close();
  }
});

test('the blobs of a history are bounded in all, before any is read', () => {
  // Five distinct blobs of 60 MiB: each under the per-value bound, together over the lease's 256 MiB.
  const events = [1, 2, 3, 4, 5].map((n) => ({
    seq: n, kind: 'HISTORY_SIGNAL_BUFFERED', body: 'signal_buffered',
    signal_buffered: { signal_name: 's', payload: { value: 'blob', blob: { blob_id: `b-${n}`, size: 60 * 1024 * 1024, sha256: 'b'.repeat(64) }, placeholder: '' } },
  })) as WireHistoryEvent[];
  assert.throws(() => blobRefsOf(events), (error: Error) => error.name === 'HistoryValuesExceeded');
});

test('a history over the largest a function replays is malformed, and read no further', async () => {
  const calls: Calls[] = [];
  const events: WorkflowTaskEvent[] = [];
  const tasks = tasksOf(stubRpc(calls), events);
  try {
    await tasks.load();
    await tasks.serve(leaseOf(2, { first_page: [], visible_count: 58_369 }));
    assert.deepEqual(calls.map(({ method, request }) => [method, request['cause']]), [['FailWorkflowTask', 'CAUSE_NONDETERMINISM_RISK']]);
  } finally {
    await tasks.close();
  }
});

test('the wire takes a marker identifier and every target from the checked target, and the host checks typed fields', () => {
  const prepared = (kind: string, target: string, body: Record<string, unknown>): PreparedCommand =>
    ({ kind, target, body, issuedBySdk: false, alwaysInline: false, commandNumber: 1, bodyDigest: new Uint8Array(32), valueAsBlob: false }) as PreparedCommand;
  assert.deepEqual(commandToWire(prepared('marker_patch', 'v2', { markerId: 'free text' }), 1, undefined)['marker'], { marker_id: 'v2' });
  assert.deepEqual(commandToWire(prepared('step_cancel', 'c-3', { target: 'c-9 or anything' }), 1, undefined)['target'], { target: 'c-3' });
  assert.deepEqual(commandToWire(prepared('saga_close', 's-4', { scope: 's-9' }), 1, undefined)['saga_close'], { target: 's-4' });
  const answer = (command: Record<string, unknown>): Record<string, unknown> => ({ commands: [{ issuedBySdk: true, alwaysInline: false, commandNumber: 1, ...command }], logs: [], open: true });
  const refused: Record<string, unknown>[] = [
    { kind: 'fail', target: '', body: { code: 'lower case', message: 'm' } },
    { kind: 'fail', target: '', body: { code: 'DONE', message: 'x'.repeat(4097) } },
    { kind: 'update_reject', target: '3', body: { admittedSeq: 3, accepted: false, reason: 'BECAUSE I SAID SO' } },
    { kind: 'update_reject', target: '3', body: { admittedSeq: 4, accepted: false, reason: 'VALIDATION_FAILED' } },
    { kind: 'update_complete', target: '3', body: { admittedSeq: 3, failureCode: 'free text' } },
    { kind: 'saga_open', target: 's-1', body: { scope: 's-1', parentScope: 'free', onCompensationFailure: 'manual_review', cancelPolicy: 'compensate_completed' } },
    { kind: 'saga_open', target: 's-1', body: { scope: 's-1', parentScope: null, onCompensationFailure: 'whatever', cancelPolicy: 'compensate_completed' } },
    { kind: 'marker_side_effect', target: 'fx', body: { markerId: 'fx', value: 1, extra: 'x' } },
  ];
  for (const command of refused) assert.throws(() => checkActivationAnswer(answer(command)), /the context answered/, JSON.stringify(command));
  assert.doesNotThrow(() => checkActivationAnswer(answer({ kind: 'update_reject', target: '3', body: { admittedSeq: 3, accepted: false, reason: 'VALIDATION_FAILED' } })));
});

test('a lease without its start time is dropped, and a history stream without its last page is never answered', async () => {
  const calls: Calls[] = [];
  const events: WorkflowTaskEvent[] = [];
  const tasks = tasksOf(stubRpc(calls, { pages: [golden.history.filter((event) => event.seq <= 2) as WireHistoryEvent[]], lastPage: false }), events);
  try {
    await tasks.load();
    await tasks.serve(leaseOf(1, { started_at_us: 0 }));
    await tasks.serve(leaseOf(1, { first_page: [] }));
    assert.deepEqual(events.map(({ outcome }) => outcome), ['DROPPED', 'UNREPORTED']);
    // The stream is read again from the next sequence, at most five times (Decision 044-325), and never answered.
    assert.deepEqual(calls.map(({ method }) => method), Array(5).fill('ReadHistory'), 'no report was made');
  } finally {
    await tasks.close();
  }
});

test('a lease whose inner fence differs from its own is dropped', async () => {
  const calls: Calls[] = [];
  const events: WorkflowTaskEvent[] = [];
  const tasks = tasksOf(stubRpc(calls), events);
  try {
    await tasks.load();
    await tasks.serve(leaseOf(1, { attempt: 2 }));
    await tasks.serve(leaseOf(1, { generation: 99 }));
    assert.deepEqual(events.map(({ outcome }) => outcome), ['DROPPED', 'DROPPED']);
    assert.deepEqual(calls, []);
  } finally {
    await tasks.close();
  }
});

test('a blob named twice with different claims is a malformed history', () => {
  const event = (seq: number, size: number): WireHistoryEvent => ({
    seq, kind: 'HISTORY_SIGNAL_BUFFERED', body: 'signal_buffered',
    signal_buffered: { signal_name: 's', payload: { value: 'blob', blob: { blob_id: 'same', size, sha256: 'c'.repeat(64) }, placeholder: '' } },
  }) as WireHistoryEvent;
  assert.deepEqual(blobRefsOf([event(1, 10), event(2, 10)]).length, 1);
  assert.throws(() => blobRefsOf([event(1, 10), event(2, 11)]), MalformedHistory);
});

test('a blob whose stream header differs from its reference is refused', async () => {
  const value = Buffer.from('{"authorizationId":"auth-77"}');
  const sha256 = createHash('sha256').update(value).digest('hex');
  const history = golden.history.map((event) => (event.kind === 'HISTORY_STEP_OUTCOME' && event['step_outcome'].node === 'c-2'
    ? { ...event, step_outcome: { ...event['step_outcome'], output: { value: 'blob', blob: { blob_id: 'out', size: value.length, sha256 }, placeholder: '' } } }
    : event)) as WireHistoryEvent[];
  const calls: Calls[] = [];
  const events: WorkflowTaskEvent[] = [];
  const tasks = tasksOf(stubRpc(calls, { blobs: new Map([['out', value]]), header: { blob_id: 'other', size: value.length, sha256, content_type: 'application/json' } }), events);
  try {
    await tasks.load();
    await tasks.serve(leaseOf(2, {}, history));
    assert.deepEqual(calls.map(({ method, request }) => [method, request['cause'] ?? '']), [['ReadBlob', ''], ['FailWorkflowTask', 'CAUSE_NONDETERMINISM_RISK']]);
  } finally {
    await tasks.close();
  }
});

test('an export line cannot set a prototype or nest without bound', () => {
  const line = '{"seq":"1","kind":"HISTORY_RUN_STARTED","runStarted":{"input":{"json":"bnVsbA=="}},"__proto__":{"stepOutcome":{"node":"c-1","outcome":"STEP_OUTCOME_COMPLETED"}}}';
  const event = eventFromProto3Json(JSON.parse(line));
  assert.equal(event.kind, 'run.started');
  let deep = '{"x":1}';
  for (let i = 0; i < 20_000; i++) deep = `{"x":${deep}}`;
  assert.throws(() => eventFromProto3Json(JSON.parse(`{"seq":"1","kind":"HISTORY_RUN_STARTED","runStarted":{},"unknown":${deep}}`)), MalformedHistory);
});

test('a malformed manifest is refused as a validation failure', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'sagat-manifest-'));
  writeFileSync(join(directory, 'bundle.js'), bundleOf({ order }).code);
  for (const manifest of ['null', '{"bundle":"bundle.js","workflows":7}']) {
    writeFileSync(join(directory, 'manifest.json'), manifest);
    const host = new WorkerHost({
      grpcUrl: 'http://127.0.0.1:1', namespace: 'default', token: 't', workerName: 'w',
      workflowBundlePath: join(directory, 'bundle.js'), workflowManifestPath: join(directory, 'manifest.json'),
    });
    await assert.rejects(host.start(), (error: { code?: string }) => error.code === 'VALIDATION_FAILED', manifest);
  }
});

test('a run of a thread that ended is never taken from the cache', async () => {
  const thread = new WorkflowThread();
  const bundle = await thread.loadBundle(bundleOf({ order }).code);
  const run = await thread.createRun(bundle, { workflowType: 'order', seed: SEED, stateMaxBytes: 0, gatedEntries: [] });
  assert.equal(run.broken, false);
  await thread.terminate();
  assert.equal(run.broken, true);
});
