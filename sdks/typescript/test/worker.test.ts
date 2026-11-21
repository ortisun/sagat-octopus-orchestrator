import { strict as assert } from 'node:assert';
import { dirname, join } from 'node:path';
import { after, afterEach, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import * as grpc from '@grpc/grpc-js';
import * as protoLoader from '@grpc/proto-loader';
import {
  ActivityFailure, SagatError, WorkerHost, reconcile,
  type ActivityContext, type WorkerConfig, type WorkerEvent,
} from '../src/index.js';
import type { TaskLease } from '../src/worker/grpc.js';
import { errorOfStatus } from '../src/worker/grpc.js';
import { FaultPlan } from '../conformance/faults.js';

const here = dirname(fileURLToPath(import.meta.url));
const PROTO = join(here, '..', '..', 'proto', 'worker.proto');

type Call = { method: string; request: Record<string, unknown>; metadata: grpc.Metadata };

test('a blob integrity failure is a protocol error without server text', () => {
  const metadata = new grpc.Metadata();
  metadata.set('x-sagat-error', 'BLOB_CORRUPT');
  const error = Object.assign(new Error('payload must not escape'), {
    code: grpc.status.DATA_LOSS, details: 'payload must not escape', metadata,
  }) as grpc.ServiceError;
  const mapped = errorOfStatus('ReadBlob', error);
  assert.equal(mapped.code, 'PROTOCOL');
  assert.equal(mapped.reason, 'BLOB_CORRUPT');
  assert.doesNotMatch(mapped.message, /payload must not escape/);
});

/** An in-memory worker service the tests script: queued leases, queued answers, a record of every call. */
class FakeServer {
  readonly calls: Call[] = [];
  leases: TaskLease[] = [];
  /** Per method, errors to answer with before succeeding (consumed in order). */
  failures: Record<string, Array<{ code: number; reason?: string; retryAfterMs?: number }>> = {};
  acceptedVersion = '1.2';
  cancelRequested = false;
  heartbeatExpiresMs = Date.now() + 60_000;
  secrets: Array<{ name: string; value: Buffer; version: number }> = [];
  /** How long a poll takes to answer, and the wait the answer asks for when it brings no task. */
  pollDelayMs = 0;
  completeDelayMs = 0;
  pollRetryAfterMs = 50;
  streamBatches: TaskLease[][] | undefined;
  streamFailureAfterFirst: number | undefined;
  holdStream = false;
  streamCancelled = 0;
  /** A misbehaving service: answers a poll with every task it has, whatever `max_tasks` says. */
  ignoreMax = false;
  /** When set, a FailStep waits for this promise before it is answered; a scripted failure does not wait. */
  failStepGate: Promise<void> | undefined;
  /** The FailStep calls that arrived, the ones on their way at the service now, and the most there were at once. */
  failStepsArrived = 0;
  failStepsOnTheirWay = 0;
  failStepsPeak = 0;
  /**
   * Bumped by `reset()`. A host keeps the session token it registered with, so a host a previous test left
   * running (still polling while it stops) cannot take the leases of the next test: its receives are
   * answered empty and are not recorded with this test's calls.
   */
  private generation = 1;
  private server = new grpc.Server();
  port = 0;

  async start(): Promise<void> {
    const definition = protoLoader.loadSync(PROTO, { keepCase: true, longs: Number, enums: String, defaults: true, oneofs: true });
    const loaded = grpc.loadPackageDefinition(definition) as unknown as {
      sagat_octopus_orchestrator: { v1: { WorkerService: grpc.ServiceClientConstructor } };
    };
    const impl: Record<string, any> = {};
    const reply = (method: string, answer: (request: any) => unknown): void => {
      impl[method] = (call: grpc.ServerUnaryCall<any, any>, callback: grpc.sendUnaryData<any>) => {
        if (method === 'pollTasks' && this.stale(call.request)) {
          callback(null, { tasks: [], retry_after_ms: this.pollRetryAfterMs });
          return;
        }
        this.calls.push({ method, request: call.request as Record<string, unknown>, metadata: call.metadata });
        const queued = this.failures[method]?.shift();
        if (queued) {
          const metadata = new grpc.Metadata();
          if (queued.reason) metadata.set('x-sagat-error', queued.reason);
          if (queued.retryAfterMs !== undefined) metadata.set('retry-after-ms', String(queued.retryAfterMs));
          callback({ code: queued.code, details: 'scripted', metadata } as grpc.ServiceError);
          return;
        }
        if (method === 'pollTasks' && this.pollDelayMs > 0) {
          const response = answer(call.request);
          const timer = setTimeout(() => callback(null, response), this.pollDelayMs);
          call.on('cancelled', () => clearTimeout(timer));
          return;
        }
        if (method === 'completeStep' && this.completeDelayMs > 0) {
          setTimeout(() => callback(null, answer(call.request)), this.completeDelayMs);
          return;
        }
        callback(null, answer(call.request));
      };
    };
    reply('registerWorker', () => ({ worker_id: 'w-1', session_token: `session-${this.generation}`, accepted_protocol_version: this.acceptedVersion }));
    reply('pollTasks', (request: { max_tasks: number }) => {
      // Like the real service: no more than were asked for.
      const tasks = this.leases.splice(0, this.ignoreMax ? this.leases.length : request.max_tasks);
      return { tasks, retry_after_ms: this.pollRetryAfterMs };
    });
    impl['streamTasks'] = (call: grpc.ServerWritableStream<{ max_tasks: number }, unknown>) => {
      if (this.stale(call.request as unknown as Record<string, unknown>)) {
        call.write({ tasks: [], retry_after_ms: this.pollRetryAfterMs });
        call.end();
        return;
      }
      this.calls.push({ method: 'streamTasks', request: call.request as unknown as Record<string, unknown>, metadata: call.metadata });
      let closed = false;
      const closedOnce = (): void => {
        if (!closed) this.streamCancelled += 1;
        closed = true;
      };
      call.on('cancelled', closedOnce);
      call.on('close', closedOnce);
      const queued = this.failures['streamTasks']?.shift();
      if (queued) {
        const metadata = new grpc.Metadata();
        if (queued.reason) metadata.set('x-sagat-error', queued.reason);
        if (queued.retryAfterMs !== undefined) metadata.set('retry-after-ms', String(queued.retryAfterMs));
        call.emit('error', { code: queued.code, details: 'scripted', metadata } as grpc.ServiceError);
        return;
      }
      const batches = this.streamBatches ?? [this.leases.splice(0, call.request.max_tasks)];
      let sent = 0;
      for (const tasks of batches) {
        if (tasks.length === 0) continue;
        call.write({ tasks, retry_after_ms: 0 });
        sent += tasks.length;
        if (this.streamFailureAfterFirst !== undefined) {
          const code = this.streamFailureAfterFirst;
          this.streamFailureAfterFirst = undefined;
          setTimeout(() => call.emit('error', { code, details: 'scripted', metadata: new grpc.Metadata() } as grpc.ServiceError), 20);
          return;
        }
      }
      this.streamBatches = undefined;
      if (this.holdStream) return;
      if (sent < call.request.max_tasks) call.write({ tasks: [], retry_after_ms: this.pollRetryAfterMs });
      call.end();
    };
    reply('heartbeat', () => ({ lease_expires_at_epoch_ms: this.heartbeatExpiresMs, cancel_requested: this.cancelRequested }));
    reply('completeStep', () => ({ command_id: 'c', already_applied: false, step_status: 'SUCCEEDED', run_status: 'RUNNING' }));
    reply('failStep', () => ({ command_id: 'c', already_applied: false, step_status: 'FAILED', run_status: 'RUNNING' }));
    const answerFailStep = impl['failStep'] as (call: grpc.ServerUnaryCall<any, any>, callback: grpc.sendUnaryData<any>) => void;
    impl['failStep'] = (call: grpc.ServerUnaryCall<any, any>, callback: grpc.sendUnaryData<any>) => {
      this.failStepsArrived += 1;
      this.failStepsOnTheirWay += 1;
      this.failStepsPeak = Math.max(this.failStepsPeak, this.failStepsOnTheirWay);
      const gate = (this.failures['failStep']?.length ?? 0) > 0 ? undefined : this.failStepGate;
      void (gate ?? Promise.resolve()).then(() => answerFailStep(call, (error, response) => {
        this.failStepsOnTheirWay -= 1;
        callback(error, response);
      }));
    };
    reply('resolveSecrets', () => ({ secrets: this.secrets }));
    this.server.addService(loaded.sagat_octopus_orchestrator.v1.WorkerService.service, impl as unknown as grpc.UntypedServiceImplementation);
    this.port = await new Promise<number>((resolve, reject) =>
      this.server.bindAsync('127.0.0.1:0', grpc.ServerCredentials.createInsecure(), (error, port) => (error ? reject(error) : resolve(port))),
    );
  }

  stop(): void {
    this.server.forceShutdown();
  }

  /** A receive by a host registered before the last `reset()`. */
  private stale(request: Record<string, unknown>): boolean {
    const token = request['session_token'];
    return typeof token === 'string' && token !== '' && token !== `session-${this.generation}`;
  }

  of(method: string): Call[] {
    return this.calls.filter((call) => call.method === method);
  }

  reset(): void {
    this.generation += 1;
    this.calls.length = 0;
    this.leases = [];
    this.failures = {};
    this.acceptedVersion = '1.2';
    this.cancelRequested = false;
    this.heartbeatExpiresMs = Date.now() + 60_000;
    this.secrets = [];
    this.pollDelayMs = 0;
    this.completeDelayMs = 0;
    this.pollRetryAfterMs = 50;
    this.streamBatches = undefined;
    this.streamFailureAfterFirst = undefined;
    this.holdStream = false;
    this.streamCancelled = 0;
    this.ignoreMax = false;
    this.failStepGate = undefined;
    this.failStepsArrived = 0;
    this.failStepsOnTheirWay = 0;
    this.failStepsPeak = 0;
  }
}

const lease = (overrides: Partial<TaskLease> = {}): TaskLease => ({
  task_id: 'run-1/charge/1', run_id: 'run-1', step_id: 'charge', attempt: 1, generation: 1, lease_token: 'lt', expires_at_epoch_ms: Date.now() + 60_000,
  activity_name: 'charge', input_json: JSON.stringify({ run: { id: 'run-1' }, node: 'charge', input: { amount: 5 }, steps: { earlier: { output: { ok: true } } } }),
  input_blob_ref: '', traceparent: '00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01', tenant_alias: '', idempotency_key: 'run-1:charge',
  task_kind: '', target_node: '', target_idempotency_key: '', ...overrides,
});

let fake: FakeServer;
before(async () => {
  fake = new FakeServer();
  await fake.start();
});
after(() => fake.stop());
const hosts: WorkerHost[] = [];
afterEach(async () => {
  while (hosts.length) await hosts.pop()?.stop(1000);
  fake.reset();
});

function host(extra: Partial<WorkerConfig> = {}): WorkerHost {
  const created = new WorkerHost({
    grpcUrl: `http://127.0.0.1:${fake.port}`, namespace: 'payments', token: 'tok', workerName: 'test-worker',
    retry: { initialDelayMs: 5, maxDelayMs: 20 }, ...extra,
  });
  hosts.push(created);
  return created;
}

async function until(condition: () => boolean, what: string, ms = 5000): Promise<void> {
  // A descheduled event loop must let later activity timers run before this waiter expires.
  const attempts = Math.ceil(ms / 10);
  let attempt = 0;
  while (!condition()) {
    if (attempt++ >= attempts) {
      // Which calls the fake answered tells where a slow or stuck host stopped.
      assert.fail(`timed out waiting for ${what}; calls: ${fake.calls.slice(0, 6).map((c) => `${c.method}(${String(c.request['worker_id'] ?? c.request['worker_name'] ?? '')})`).join(', ')}`);
    }
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

test('the asynchronous waiter does not spend its budget while the event loop cannot run', async () => {
  let ready = false;
  setTimeout(() => {
    const end = Date.now() + 100;
    while (Date.now() < end) { /* simulate a descheduled runner */ }
  }, 5);
  setTimeout(() => { ready = true; }, 20);
  await until(() => ready, 'the later timer', 50);
});

test('scoped instances that share a legacy task id are counted one by one against maxInflight', async () => {
  // The legacy task id is run/node/generation, so two iterations of one scope leased together carry the
  // same one: when the quick one ends, the slow one still holds its slot.
  const instanceOf = (index: number) => ({ instance_path: [{ scope_node_id: 'lines', iteration_index: index }], definition_node_id: 'charge' });
  const scoped = (index: number): TaskLease => lease({
    instance: instanceOf(index), input_json: JSON.stringify({ input: {}, item: { n: index }, index, steps: {} }),
  });
  fake.acceptedVersion = '1.5';
  fake.leases = [scoped(0), scoped(1)];
  let release: () => void = () => undefined;
  const slow = new Promise<void>((resolve) => { release = resolve; });
  const h = host({ maxInflight: 2, transport: 'poll' });
  h.registerActivity('charge', async (_input, context) => {
    if (context.index === 0) await slow;
    return { ok: true };
  });
  await h.start();
  await until(() => fake.of('completeStep').length === 1, 'the quick iteration to complete');
  const completedAt = fake.calls.findIndex((call) => call.method === 'completeStep');
  await until(() => fake.calls.slice(completedAt).some((call) => call.method === 'pollTasks' || call.method === 'streamTasks'), 'the next receive');
  const next = fake.calls.slice(completedAt).find((call) => call.method === 'pollTasks' || call.method === 'streamTasks');
  assert.equal(next?.request['max_tasks'], 1, 'one slot is still held by the slow iteration');
  release();
  await until(() => fake.of('completeStep').length === 2, 'the slow iteration to complete');
});

test('a worker registers its activities and the newest protocol it speaks, with the token as metadata', async () => {
  const h = host({ languages: ['typescript'], maxInflight: 7 });
  h.registerActivity('b.second', () => 1).registerCompensation('a.undo', () => 1).registerReconciler('c.lookup', () => reconcile.unknown());
  await h.start();
  const [register] = fake.of('registerWorker');
  assert.ok(register);
  assert.deepEqual(register.request, {
    worker_name: 'test-worker', namespace: 'payments',
    capabilities: { languages: ['typescript'], activities: ['a.undo', 'b.second', 'c.lookup'], pools: [], max_inflight: 7, protocol_version: '1.5' },
  });
  assert.equal(register.metadata.get('authorization')[0], 'Bearer tok');
  assert.equal(h.protocolVersion, '1.2');
});

test('protocol 1.5 echoes scoped identity and protocol 1.4 omits it', async () => {
  const instance = {
    instance_path: [{ scope_node_id: 'lines', iteration_index: 7 }],
    definition_node_id: 'charge',
  };
  fake.acceptedVersion = '1.5';
  fake.leases = [lease({ instance, input_json: JSON.stringify({ input: { order: 7 }, item: { sku: 'one' }, index: 7, steps: {} }) })];
  const current = host({ workerName: 'scoped-current' });
  let currentContext: ActivityContext | undefined;
  current.registerActivity('charge', (_input, context) => {
    currentContext = context;
    return { ok: true };
  });
  await current.start();
  await until(() => fake.of('completeStep').length === 1, 'the scoped completion');
  assert.deepEqual(fake.of('completeStep')[0]!.request['instance'], instance);
  assert.deepEqual(currentContext?.instance, {
    instancePath: [{ scopeNodeId: 'lines', iterationIndex: 7 }],
    definitionNodeId: 'charge',
  });
  assert.deepEqual(currentContext?.item, { sku: 'one' });
  assert.equal(currentContext?.index, 7);
  await current.stop();

  fake.reset();
  fake.acceptedVersion = '1.4';
  fake.leases = [lease({ instance, input_json: JSON.stringify({ input: { order: 7 }, item: { sku: 'one' }, index: 7, steps: {} }) })];
  const legacy = host({ workerName: 'scoped-legacy', protocolVersion: '1.4' });
  let legacyContext: ActivityContext | undefined;
  legacy.registerActivity('charge', (_input, context) => {
    legacyContext = context;
    return { ok: true };
  });
  await legacy.start();
  await until(() => fake.of('completeStep').length === 1, 'the legacy completion');
  assert.equal(fake.of('completeStep')[0]!.request['instance'], null);
  assert.equal(legacyContext?.instance, undefined);
  assert.equal(legacyContext?.item, undefined);
  assert.equal(legacyContext?.index, undefined);
});

test('a task is run with its context and completed once with a key that is the same on every retry', async () => {
  let seen: ActivityContext | undefined;
  let received: unknown;
  const events: WorkerEvent[] = [];
  const h = host({ onEvent: (e) => events.push(e) });
  h.registerActivity<{ amount: number }, { charged: number }>('charge', (input, ctx) => {
    seen = ctx;
    received = input;
    return { charged: input.amount * 2 };
  });
  fake.failures['completeStep'] = [{ code: grpc.status.UNAVAILABLE }, { code: grpc.status.RESOURCE_EXHAUSTED, reason: 'RATE_LIMITED', retryAfterMs: 1 }];
  fake.leases = [lease()];
  await h.start();
  await until(() => fake.of('completeStep').length === 3, 'three attempts to report');
  assert.deepEqual(received, { amount: 5 });
  assert.equal(seen?.runId, 'run-1');
  assert.equal(seen?.node, 'charge');
  assert.equal(seen?.stepId, 'charge');
  assert.equal(seen?.attempt, 1);
  assert.equal(seen?.idempotencyKey, 'run-1:charge');
  assert.equal(seen?.taskKind, 'FORWARD');
  assert.deepEqual(seen?.steps, { earlier: { ok: true } });
  assert.equal(seen?.traceparent, '00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01');
  const keys = new Set(fake.of('completeStep').map((c) => c.request['idempotency_key']));
  assert.deepEqual([...keys], ['done-run-1/charge/1']);
  const last = fake.of('completeStep')[2]!.request;
  assert.equal(last['output_json'], '{"charged":10}');
  assert.equal(last['lease_token'], 'lt');
  assert.equal(last['generation'], 1);
  await until(() => events.some((e) => e.type === 'task.completed'), 'the completed event');
});

test('a failure on purpose carries its code and whether it may be retried; any other error says only its type', async () => {
  const h = host();
  h.registerActivity('permanent', () => { throw ActivityFailure.permanent('INSUFFICIENT_FUNDS', 'not enough'); });
  h.registerActivity('unknown', () => { throw ActivityFailure.outcomeUnknown('the provider timed out'); });
  h.registerActivity('boom', () => { throw new TypeError('secret card 4111 leaked in the message'); });
  h.registerActivity('badcode', () => { throw new ActivityFailure('not a code!', 'x', false); });
  fake.leases = ['permanent', 'unknown', 'boom', 'badcode'].map((name, i) => lease({ task_id: `run-${i}/${name}/1`, activity_name: name, step_id: name }));
  await h.start();
  await until(() => fake.of('failStep').length === 4, 'four failures');
  const by = Object.fromEntries(fake.of('failStep').map((c) => [c.request['step_id'] as string, c.request]));
  assert.deepEqual([by['permanent']?.['error_code'], by['permanent']?.['retryable'], by['permanent']?.['safe_message']], ['INSUFFICIENT_FUNDS', false, 'not enough']);
  assert.deepEqual([by['unknown']?.['error_code'], by['unknown']?.['retryable']], ['OUTCOME_UNKNOWN', false]);
  assert.deepEqual([by['boom']?.['error_code'], by['boom']?.['retryable'], by['boom']?.['safe_message']], ['ACTIVITY_ERROR', true, 'TypeError']);
  assert.equal(by['badcode']?.['error_code'], 'WORKER_ERROR');
  assert.equal(by['permanent']?.['idempotency_key'], 'failed-run-0/permanent/1');
});

test('a stale report is dropped and never retried', async () => {
  const h = host();
  h.registerActivity('charge', () => ({ ok: true }));
  fake.failures['completeStep'] = [{ code: grpc.status.ABORTED, reason: 'STALE_ATTEMPT' }];
  const events: WorkerEvent[] = [];
  const h2 = host({ workerName: 'other', onEvent: (e) => events.push(e) });
  h2.registerActivity('charge', () => ({ ok: true }));
  fake.leases = [lease()];
  await h2.start();
  await until(() => events.some((e) => e.type === 'task.stale'), 'the stale event');
  assert.equal(fake.of('completeStep').length, 1);
  void h;
});

test('an input the handler cannot take, an activity nobody registered and an output too large are failures for good', async () => {
  const h = host();
  h.registerActivity('typed', () => 1, { parse: () => { throw new Error('not an order'); } });
  h.registerActivity('huge', () => 'x'.repeat(300 * 1024));
  h.registerActivity('bad-number', () => ({ n: Number.NaN }));
  fake.leases = [
    lease({ task_id: 'r/typed/1', activity_name: 'typed', step_id: 'typed' }),
    lease({ task_id: 'r/huge/1', activity_name: 'huge', step_id: 'huge' }),
    lease({ task_id: 'r/nobody/1', activity_name: 'nobody', step_id: 'nobody' }),
    lease({ task_id: 'r/bad-number/1', activity_name: 'bad-number', step_id: 'bad-number' }),
    lease({ task_id: 'r/junk/1', activity_name: 'typed', step_id: 'junk', input_json: '{nope' }),
  ];
  await h.start();
  await until(() => fake.of('failStep').length === 5, 'five failures');
  const code = (step: string): unknown => fake.of('failStep').find((c) => c.request['step_id'] === step)?.request['error_code'];
  assert.equal(code('typed'), 'INPUT_INVALID');
  assert.equal(code('huge'), 'PAYLOAD_TOO_LARGE');
  assert.equal(code('nobody'), 'NO_HANDLER');
  assert.equal(code('bad-number'), 'OUTPUT_INVALID');
  assert.equal(code('junk'), 'INPUT_INVALID');
  assert.ok(fake.of('failStep').every((c) => c.request['retryable'] === false));
});

test('the heartbeat starts before the handler ends, carries the step identity and moves the deadline', async () => {
  const h = host({ heartbeatIntervalMs: 20 });
  let deadlines: number[] = [];
  h.registerActivity('slow', async (_input, ctx) => {
    deadlines.push(ctx.deadline.getTime());
    await until(() => fake.of('heartbeat').length >= 3 && ctx.deadline.getTime() > deadlines[0]!, 'three heartbeats and the extended lease while the handler is active');
    deadlines.push(ctx.deadline.getTime());
    return { ok: true };
  });
  fake.heartbeatExpiresMs = Date.now() + 120_000;
  fake.leases = [lease({ activity_name: 'slow', step_id: 'slow', task_id: 'run-1/slow/1' })];
  await h.start();
  await until(() => fake.of('completeStep').length === 1, 'the completion');
  const beats = fake.of('heartbeat');
  assert.ok(beats.length >= 3, `${beats.length} beats`);
  assert.deepEqual(
    { run_id: beats[0]!.request['run_id'], step_id: beats[0]!.request['step_id'], attempt: beats[0]!.request['attempt'], lease_token: beats[0]!.request['lease_token'], worker_id: beats[0]!.request['worker_id'] },
    { run_id: 'run-1', step_id: 'slow', attempt: 1, lease_token: 'lt', worker_id: 'w-1' },
  );
  assert.ok((deadlines[1] ?? 0) > (deadlines[0] ?? 0), 'the lease was extended');
  assert.equal(fake.of('heartbeat').length, beats.length);
});

test('a cancellation reaches the activity through its signal and it ends however it likes', async () => {
  const h = host({ heartbeatIntervalMs: 20 });
  let reason: unknown;
  h.registerActivity('long', async (_input, ctx) => {
    await new Promise<void>((resolve) => ctx.signal.addEventListener('abort', () => resolve(), { once: true }));
    reason = ctx.signal.reason;
    return { stopped: true };
  });
  fake.cancelRequested = true;
  fake.leases = [lease({ activity_name: 'long', step_id: 'long', task_id: 'run-1/long/1' })];
  await h.start();
  await until(() => fake.of('completeStep').length === 1, 'the completion');
  assert.equal(reason, 'run-cancelled');
  assert.equal(fake.of('completeStep')[0]!.request['output_json'], '{"stopped":true}');
});

test('an activity that raises its cancellation is failed for good as CANCELLED', async () => {
  const h = host({ heartbeatIntervalMs: 20 });
  h.registerActivity('long', async (_input, ctx) => {
    await new Promise<void>((resolve) => ctx.signal.addEventListener('abort', () => resolve(), { once: true }));
    throw new SagatError({ code: 'CANCELLED', message: 'stopped' });
  });
  fake.cancelRequested = true;
  fake.leases = [lease({ activity_name: 'long', step_id: 'long', task_id: 'run-1/long/1' })];
  await h.start();
  await until(() => fake.of('failStep').length === 1, 'the failure');
  assert.deepEqual([fake.of('failStep')[0]!.request['error_code'], fake.of('failStep')[0]!.request['retryable']], ['CANCELLED', false]);
});

test('a lease lost during the activity stops it and its result is never reported', async () => {
  const h = host({ heartbeatIntervalMs: 20 });
  const events: WorkerEvent[] = [];
  const h2 = host({ workerName: 'lost', heartbeatIntervalMs: 20, onEvent: (e) => events.push(e) });
  h2.registerActivity('slow', async (_input, ctx) => {
    await new Promise<void>((resolve) => ctx.signal.addEventListener('abort', () => resolve(), { once: true }));
    return { late: true, reason: String(ctx.signal.reason) };
  });
  fake.failures['heartbeat'] = [{ code: grpc.status.ABORTED, reason: 'STALE_ATTEMPT' }];
  fake.leases = [lease({ activity_name: 'slow', step_id: 'slow', task_id: 'run-1/slow/1' })];
  await h2.start();
  await until(() => events.some((e) => e.type === 'task.stale'), 'the stale event');
  assert.equal(fake.of('completeStep').length, 0);
  assert.equal(fake.of('failStep').length, 0);
  void h;
});

test('a compensation sees its kind and the effect it undoes, and a reconciler answers exactly a verdict', async () => {
  const seen: Array<Record<string, unknown>> = [];
  const h = host();
  h.registerCompensation('limit.release', (_input, ctx) => {
    seen.push({ kind: ctx.taskKind, target: ctx.targetNode, targetKey: ctx.targetIdempotencyKey, key: ctx.idempotencyKey });
    return { released: true };
  });
  h.registerReconciler('limit.lookup', () => reconcile.applied({ id: 'r-9' }));
  h.registerReconciler('limit.none', () => reconcile.notApplied());
  h.registerReconciler('limit.bad', () => ({ yes: true }) as never);
  fake.leases = [
    lease({ task_id: 'run-1/reserve~c/2', step_id: 'reserve~c', activity_name: 'limit.release', task_kind: 'COMPENSATION', target_node: 'reserve', target_idempotency_key: 'run-1:reserve', idempotency_key: 'compensation:c1:run-1' }),
    lease({ task_id: 'run-1/a~r/1', step_id: 'a~r', activity_name: 'limit.lookup', task_kind: 'RECONCILIATION', target_node: 'a', target_idempotency_key: 'k' }),
    lease({ task_id: 'run-1/b~r/1', step_id: 'b~r', activity_name: 'limit.none', task_kind: 'RECONCILIATION', target_node: 'b' }),
    lease({ task_id: 'run-1/c~r/1', step_id: 'c~r', activity_name: 'limit.bad', task_kind: 'RECONCILIATION', target_node: 'c' }),
  ];
  await h.start();
  await until(() => fake.of('completeStep').length === 3 && fake.of('failStep').length === 1, 'every report');
  assert.deepEqual(seen, [{ kind: 'COMPENSATION', target: 'reserve', targetKey: 'run-1:reserve', key: 'compensation:c1:run-1' }]);
  const out = (step: string): unknown => fake.of('completeStep').find((c) => c.request['step_id'] === step)?.request['output_json'];
  assert.equal(out('a~r'), '{"effect":"applied","output":{"id":"r-9"}}');
  assert.equal(out('b~r'), '{"effect":"not_applied"}');
  assert.equal(fake.of('failStep')[0]!.request['error_code'], 'RECONCILIATION_ANSWER_INVALID');
});

test('the host runs at the version the server agreed to: against 1.0 it sends no step identity and cannot resolve secrets', async () => {
  fake.acceptedVersion = '1';
  const h = host({ heartbeatIntervalMs: 20, protocolVersion: '1.9' });
  let secretError: unknown;
  h.registerActivity('slow', async (_input, ctx) => {
    await new Promise((resolve) => setTimeout(resolve, 80));
    secretError = await ctx.secrets().catch((error: unknown) => error);
    return 1;
  });
  fake.leases = [lease({ activity_name: 'slow', step_id: 'slow', task_id: 'run-1/slow/1' })];
  await h.start();
  assert.equal(fake.of('registerWorker')[0]!.request['capabilities'] && (fake.of('registerWorker')[0]!.request['capabilities'] as Record<string, unknown>)['protocol_version'], '1.9');
  assert.equal(h.protocolVersion, '1');
  await until(() => fake.of('completeStep').length === 1, 'the completion');
  const beat = fake.of('heartbeat')[0]!.request;
  assert.deepEqual([beat['run_id'], beat['step_id'], beat['attempt']], ['', '', 0]);
  assert.ok(secretError instanceof SagatError && secretError.code === 'UNSUPPORTED_PROTOCOL');
  assert.equal(fake.of('resolveSecrets').length, 0);
});

test('secrets are resolved once per attempt, on first use, as a read-only map', async () => {
  const h = host();
  let text = '';
  h.registerActivity('pay', async (_input, ctx) => {
    const first = await ctx.secrets();
    const second = await ctx.secrets();
    assert.equal(first, second);
    text = first.text('stripe-key');
    return { names: first.names() };
  });
  fake.secrets = [{ name: 'stripe-key', value: Buffer.from('sk_live_x'), version: 3 }];
  fake.leases = [lease({ activity_name: 'pay', step_id: 'pay', task_id: 'run-1/pay/1' })];
  await h.start();
  await until(() => fake.of('completeStep').length === 1, 'the completion');
  assert.equal(text, 'sk_live_x');
  assert.equal(fake.of('resolveSecrets').length, 1);
  assert.ok(!fake.of('completeStep')[0]!.request['output_json']!.toString().includes('sk_live'));
});

test('a session that is gone is registered again and the worker goes on', async () => {
  const events: WorkerEvent[] = [];
  const h = host({ onEvent: (e) => events.push(e) });
  h.registerActivity('charge', () => ({ ok: true }));
  await h.start();
  fake.failures['pollTasks'] = [{ code: grpc.status.UNAUTHENTICATED }];
  await until(() => fake.of('registerWorker').length === 2, 'a second registration');
  fake.leases = [lease()];
  await until(() => fake.of('completeStep').length === 1, 'the task after it');
  assert.ok(events.some((e) => e.type === 'reconnect'));
});

test('protocol 1.4 streams two frames and dispatches them before the receive ends', async () => {
  fake.acceptedVersion = '1.4';
  fake.holdStream = true;
  fake.streamBatches = [
    [lease({ task_id: 'run-1/charge/1' })],
    [lease({ task_id: 'run-2/charge/1', run_id: 'run-2' })],
  ];
  const worker = host({ maxInflight: 3 });
  worker.registerActivity('charge', () => ({ ok: true }));
  await worker.start();
  await until(() => fake.of('completeStep').length === 2, 'both streamed tasks before EOF');
  assert.equal(fake.of('streamTasks').length, 1);
  assert.equal(fake.of('streamTasks')[0]!.request['max_tasks'], 3);
  assert.equal(fake.of('pollTasks').length, 0);
  const began = Date.now();
  await worker.stop(500);
  assert.ok(Date.now() - began < 1000, 'stop cancels the idle receive promptly');
});

test('a stream error after a frame preserves that handler and reconnects only receiving', async () => {
  fake.acceptedVersion = '1.4';
  fake.streamBatches = [[lease()]];
  fake.streamFailureAfterFirst = grpc.status.UNAVAILABLE;
  const worker = host({ maxInflight: 2 });
  worker.registerActivity('charge', () => ({ ok: true }));
  await worker.start();
  await until(() => fake.of('completeStep').length === 1, 'the partial delivery to complete');
  await until(() => fake.of('streamTasks').length >= 2, 'a receive reconnect');
  assert.equal(fake.of('pollTasks').length, 0);
});

test('a full-credit frame does not hide an unimplemented trailer from auto fallback', async () => {
  fake.acceptedVersion = '1.4';
  fake.streamBatches = [[lease()]];
  fake.streamFailureAfterFirst = grpc.status.UNIMPLEMENTED;
  const worker = host({ maxInflight: 1 });
  worker.registerActivity('charge', () => ({ ok: true }));
  await worker.start();
  await until(() => fake.of('completeStep').length === 1, 'the full-credit frame to complete');
  await until(() => fake.of('pollTasks').length >= 1, 'polling after the trailer was classified');
  assert.equal(fake.of('streamTasks').length, 1);
});

test('the conformance partial-stream fault forwards the first frame then fails receiving', async () => {
  fake.acceptedVersion = '1.4';
  fake.streamBatches = [[lease()]];
  const plan = new FaultPlan([{ channel: 'grpc', match: 'StreamTasks', action: 'dropStreamAfterFirst', times: 1 }]);
  const worker = host({ maxInflight: 2, interceptors: plan.interceptors() });
  worker.registerActivity('charge', () => ({ ok: true }));
  await worker.start();
  await until(() => fake.of('completeStep').length === 1, 'the frame forwarded by the fault');
  await until(() => fake.of('streamTasks').length >= 2, 'the receive after the injected disconnect');
  assert.deepEqual(plan.applied(), [1]);
});

test('auto falls back only when task streaming is unimplemented', async () => {
  fake.acceptedVersion = '1.4';
  fake.failures['streamTasks'] = [{ code: grpc.status.UNIMPLEMENTED }];
  fake.leases = [lease()];
  const worker = host();
  worker.registerActivity('charge', () => ({ ok: true }));
  await worker.start();
  await until(() => fake.of('completeStep').length === 1, 'the task through fallback polling');
  assert.equal(fake.of('streamTasks').length, 1);
  assert.ok(fake.of('pollTasks').length >= 1);
});

test('explicit streaming refuses an older negotiated server', async () => {
  const worker = host({ transport: 'stream' });
  worker.registerActivity('charge', () => ({ ok: true }));
  await assert.rejects(worker.start(), (error: unknown) =>
    error instanceof SagatError && error.code === 'UNSUPPORTED_PROTOCOL');
  assert.equal(fake.of('pollTasks').length, 0);
});

test('explicit streaming reports UNIMPLEMENTED as unsupported without polling', async () => {
  fake.acceptedVersion = '1.4';
  fake.failures['streamTasks'] = [{ code: grpc.status.UNIMPLEMENTED }];
  const events: WorkerEvent[] = [];
  const worker = host({ transport: 'stream', onEvent: (event) => events.push(event) });
  worker.registerActivity('charge', () => ({ ok: true }));
  await worker.start();
  await until(() => events.some((event) => event.type === 'receive.failed'), 'the safe receive failure event');
  const failed = events.find((event) => event.type === 'receive.failed');
  assert.deepEqual(failed, { type: 'receive.failed', code: 'UNSUPPORTED_PROTOCOL' });
  assert.equal(fake.of('pollTasks').length, 0);
});

test('explicit streaming does not call StreamTasks after re-registration negotiates 1.3', async () => {
  fake.acceptedVersion = '1.4';
  fake.streamBatches = [[lease()]];
  fake.streamFailureAfterFirst = grpc.status.UNAUTHENTICATED;
  const events: WorkerEvent[] = [];
  const worker = host({ transport: 'stream', maxInflight: 1, onEvent: (event) => events.push(event) });
  worker.registerActivity('charge', () => ({ ok: true }));
  await worker.start();
  await until(() => fake.of('streamTasks').length === 1, 'the first stream');
  fake.acceptedVersion = '1.3';
  await until(() => fake.of('registerWorker').length === 2, 'the downgraded registration');
  await until(
    () => events.some((event) => event.type === 'receive.failed' && event.code === 'UNSUPPORTED_PROTOCOL'),
    'the local unsupported-protocol receive failure',
  );
  assert.equal(fake.of('completeStep').length, 1, 'the frame delivered before re-registration finishes');
  assert.equal(fake.of('streamTasks').length, 1, 'protocol 1.3 is refused before another stream call');
  assert.equal(fake.of('pollTasks').length, 0);
});

test('a stream that exceeds capacity runs bounded work and is cancelled', async () => {
  fake.holdStream = true;
  fake.streamBatches = [[
    lease({ task_id: 'run-1/charge/1' }),
    lease({ task_id: 'run-2/charge/1', run_id: 'run-2' }),
  ]];
  let handled = 0;
  const worker = handBackHost({ maxInflight: 1 });
  worker.registerActivity('charge', async (_input, ctx) => {
    handled += 1;
    await new Promise<void>((resolve) => ctx.signal.addEventListener('abort', () => resolve(), { once: true }));
    return null;
  });
  await worker.start();
  await until(() => handled === 1, 'one bounded handler');
  await until(() => fake.of('failStep').length === 1, 'the lease beyond the credit to be handed back');
  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.equal(handled, 1);
  assert.equal(fake.of('failStep')[0]!.request['task_id'], 'run-2/charge/1');
  assert.equal(fake.of('failStep')[0]!.request['error_code'], 'TASK_NOT_STARTED');
  assert.equal(fake.of('streamTasks').length, 1, 'the malformed receive is not left open or reopened at once');
  assert.equal(fake.of('streamTasks')[0]!.request['max_tasks'], 1);
  await worker.stop(50);
});

test('a stream cannot make the host own more than the aggregate input budget: the lease over it is handed back', async () => {
  fake.streamBatches = [[lease({ input_json: 'x'.repeat(8 * 1024 * 1024 + 1) })]];
  let handled = false;
  const worker = handBackHost({ maxInflight: 1 });
  worker.registerActivity('charge', () => { handled = true; });
  await worker.start();
  await until(() => fake.of('failStep').length === 1, 'the lease over the budget to be handed back');
  await new Promise((resolve) => setTimeout(resolve, 150));
  assert.equal(handled, false);
  assert.equal(fake.of('completeStep').length, 0);
  assert.equal(fake.of('failStep').length, 1, 'handed back once');
  assert.equal(fake.of('failStep')[0]!.request['error_code'], 'TASK_NOT_STARTED');
});

test('an empty stream hint is followed by a wait, never by another receive at once', async () => {
  fake.acceptedVersion = '1.4';
  const worker = host();
  worker.registerActivity('charge', () => null);
  await worker.start();
  await new Promise((resolve) => setTimeout(resolve, 400));
  const receives = fake.of('streamTasks').length;
  assert.ok(receives >= 2 && receives <= 10, `${receives} empty receive windows in 400 ms`);
  assert.equal(fake.of('pollTasks').length, 0);
});

test('an empty poll is followed by a wait, never by another poll at once', async () => {
  const h = host();
  h.registerActivity('charge', () => ({ ok: true }));
  await h.start();
  await new Promise((resolve) => setTimeout(resolve, 400));
  const polls = fake.of('pollTasks').length;
  assert.ok(polls >= 2 && polls <= 10, `${polls} polls in 400 ms`);
});

test('no more tasks are taken than the host has room for, and stop waits for the ones in flight', async () => {
  const h = host({ maxInflight: 2 });
  let running = 0;
  let peak = 0;
  h.registerActivity('charge', async () => {
    peak = Math.max(peak, ++running);
    await new Promise((resolve) => setTimeout(resolve, 100));
    running--;
    return 1;
  });
  fake.leases = [1, 2, 3, 4].map((i) => lease({ task_id: `run-${i}/charge/1`, run_id: `run-${i}` }));
  await h.start();
  await until(() => fake.of('pollTasks').length > 0, 'a poll');
  assert.ok(Number(fake.of('pollTasks')[0]!.request['max_tasks']) <= 2);
  await h.stop(5000);
  assert.ok(peak <= 2);
  assert.equal(running, 0, 'nothing was left running');
});

test('stop while a poll is in flight does not wait out the pause an empty answer asks for', async () => {
  fake.pollDelayMs = 150;
  fake.pollRetryAfterMs = 10_000;
  const worker = host();
  worker.registerActivity('charge', () => 1);
  await worker.start();
  await until(() => fake.of('pollTasks').length === 1, 'the first poll');
  const began = Date.now();
  await worker.stop(1000);
  assert.ok(Date.now() - began < 1500, `stop took ${Date.now() - began} ms`);
});

test('registration is refused clearly: an unsupported protocol, an unbound identity', async () => {
  const h = host();
  h.registerActivity('charge', () => 1);
  fake.failures['registerWorker'] = [{ code: grpc.status.INVALID_ARGUMENT, reason: 'UNSUPPORTED_PROTOCOL' }];
  await assert.rejects(h.start(), (e: unknown) => e instanceof SagatError && e.code === 'UNSUPPORTED_PROTOCOL' && !e.retryable);
  assert.equal(fake.of('registerWorker').length, 1, 'not retried');
  const h2 = host({ workerName: 'second' });
  h2.registerActivity('charge', () => 1);
  fake.failures['registerWorker'] = [{ code: grpc.status.PERMISSION_DENIED, reason: 'NOT_BOUND' }];
  await assert.rejects(h2.start(), (e: unknown) => e instanceof SagatError && e.code === 'NOT_BOUND');
});

test('registering twice or after the start is a mistake and says so', async () => {
  const h = host();
  h.registerActivity('a', () => 1);
  assert.throws(() => h.registerActivity('a', () => 2), (e: unknown) => e instanceof SagatError && e.code === 'VALIDATION_FAILED');
  assert.throws(() => host({ maxInflight: 0 }), (e: unknown) => e instanceof SagatError);
  await assert.rejects(host().start(), (e: unknown) => e instanceof SagatError && /at least one activity/.test(e.message));
});

test('the heartbeat goes on while the result is being reported, until the report is made', async () => {
  fake.failures['completeStep'] = [{ code: 14 }, { code: 14 }, { code: 14 }];
  fake.leases = [lease()];
  // 300 ms of retries against a heartbeat every 10 ms: room enough for a loaded machine.
  const worker = host({ heartbeatIntervalMs: 10, retry: { initialDelayMs: 100, maxDelayMs: 100 } });
  worker.registerActivity('charge', () => ({ ok: true }));
  await worker.start();
  await until(() => fake.of('completeStep').length >= 4, 'the report to go through after its retries');
  const order = fake.calls.map((call) => call.method).filter((method) => method === 'heartbeat' || method === 'completeStep');
  const first = order.indexOf('completeStep');
  const last = order.lastIndexOf('completeStep');
  assert.ok(order.slice(first, last).includes('heartbeat'), `a heartbeat between the first report and the last: ${order.join(',')}`);
  // Once the report is made the heartbeat ends. One that was already on its way may still arrive; no more than that.
  await new Promise((resolve) => setTimeout(resolve, 200));
  const afterwards = fake.calls.map((call) => call.method).filter((method) => method === 'heartbeat' || method === 'completeStep');
  assert.ok(afterwards.slice(afterwards.lastIndexOf('completeStep') + 1).filter((m) => m === 'heartbeat').length <= 1, 'none after the report that was made, but the one in flight');
});

test('a handler that raises its cancellation after the host was stopped is not reported, and one cancelled by the run is', async () => {
  // Cancelled by the host: the lease lapses and the step is leased again.
  fake.leases = [lease()];
  const stopping = host();
  let entered = false;
  stopping.registerActivity('charge', (_input, ctx) => new Promise((_resolve, reject) => {
    entered = true;
    ctx.signal.addEventListener('abort', () => reject(new DOMException('cancelled', 'AbortError')));
  }));
  await stopping.start();
  await until(() => entered, 'the activity to start');
  await stopping.stop(50);
  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.equal(fake.of('failStep').length, 0, 'a deploy does not fail a step for good');
  assert.equal(fake.of('completeStep').length, 0);

  // Cancelled by the run: reported as CANCELLED, not retryable.
  fake.reset();
  fake.leases = [lease()];
  fake.cancelRequested = true;
  const cancelled = host({ heartbeatIntervalMs: 20 });
  cancelled.registerActivity('charge', (_input, ctx) => new Promise((_resolve, reject) => {
    ctx.signal.addEventListener('abort', () => reject(new DOMException('cancelled', 'AbortError')));
  }));
  await cancelled.start();
  await until(() => fake.of('failStep').length === 1, 'the cancellation to be reported');
  assert.equal(fake.of('failStep')[0]?.request['error_code'], 'CANCELLED');
  assert.equal(fake.of('failStep')[0]?.request['retryable'], false);
});

test('a poll that found nothing is followed by another as soon as a task has finished, not after the whole pause', async () => {
  fake.pollRetryAfterMs = 10_000;
  fake.leases = [lease()];
  const worker = host({ maxInflight: 2 });
  worker.registerActivity('charge', async () => {
    await new Promise((resolve) => setTimeout(resolve, 150));
    return { ok: true };
  });
  await worker.start();
  // The poll that found the room empty-handed, while the task is still running: the pause is now 10 s.
  await until(() => fake.of('pollTasks').length >= 2, 'the empty poll');
  const polls = fake.of('pollTasks').length;
  assert.equal(fake.of('completeStep').length, 0, 'the task is still running');
  await until(() => fake.of('pollTasks').length > polls, 'a poll after the task finished', 1500);
});

// ------------------------------------------------------- from the correctness review of the first three SDKs

test('a host is restartable: stopped, it can be started again and serves again (SPEC §6.8)', async () => {
  const worker = host();
  worker.registerActivity('charge', () => ({ ok: true }));
  fake.leases = [lease()];
  await worker.start();
  await until(() => fake.of('completeStep').length === 1, 'the first completion');
  await worker.stop(500);
  fake.leases = [lease({ task_id: 'run-1/charge/2', generation: 2 })];
  await worker.start();
  await until(() => fake.of('completeStep').length === 2, 'the completion after the restart');
  assert.equal(fake.of('registerWorker').length, 2, 'it registered again');
  await worker.stop(500);
});

test('two starts at once register once', async () => {
  const worker = host();
  worker.registerActivity('charge', () => null);
  await Promise.all([worker.start(), worker.start()]);
  assert.equal(fake.of('registerWorker').length, 1);
  await worker.stop(500);
});

test('stop does not wait for a poll the server holds: it ends within its timeout and the channel is closed', async () => {
  fake.pollDelayMs = 6000;
  const worker = host();
  worker.registerActivity('charge', () => null);
  await worker.start();
  await until(() => fake.of('pollTasks').length === 1, 'the poll');
  const began = Date.now();
  await worker.stop(200);
  assert.ok(Date.now() - began < 2500, `stop took ${Date.now() - began} ms`);
});

test('after a stop that timed out the heartbeats of a task that will not end stop too', async () => {
  const worker = host({ heartbeatIntervalMs: 30, transport: 'poll' });
  worker.registerActivity('charge', () => new Promise(() => undefined));
  await worker.start();
  await until(() => fake.of('pollTasks').length >= 1, 'the worker poll');
  fake.leases = [lease()];
  await until(() => fake.of('heartbeat').length >= 2, 'heartbeats');
  await worker.stop(100);
  // A heartbeat that was already on its way when the host stopped may still arrive; none is sent after that.
  await new Promise((resolve) => setTimeout(resolve, 150));
  const after = fake.of('heartbeat').length;
  await new Promise((resolve) => setTimeout(resolve, 400));
  assert.equal(fake.of('heartbeat').length, after, 'no heartbeat after the host stopped');
});

test('a failure while reporting is cut by code points, never inside a pair, and what is sent is well formed', async () => {
  fake.leases = [lease()];
  const worker = host();
  worker.registerActivity('charge', () => {
    throw ActivityFailure.permanent('TOO_LONG', '😀'.repeat(600) + '\ud800');
  });
  await worker.start();
  await until(() => fake.of('failStep').length === 1, 'the failure');
  const message = String(fake.of('failStep')[0]!.request['safe_message']);
  assert.equal(Array.from(message).length, 512, 'exactly 512 code points: cut by code points, not by UTF-16 units (which would leave 256 emoji)');
  assert.equal(message, message.replace(/[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/g, '\ufffd'), 'no lone surrogate');
  await worker.stop(500);
});

test('against 1.1 every task is a forward task: task_kind and the target are not read', async () => {
  fake.acceptedVersion = '1.1';
  fake.leases = [lease({ task_kind: 'COMPENSATION', target_node: 'charge', target_idempotency_key: 'run-1:charge' })];
  const worker = host();
  let seen: { kind: string; target: string; key: string } | undefined;
  worker.registerCompensation('charge', (_input, ctx) => void (seen = { kind: ctx.taskKind, target: ctx.targetNode, key: ctx.targetIdempotencyKey }));
  await worker.start();
  await until(() => seen !== undefined, 'the task');
  assert.deepEqual(seen, { kind: 'FORWARD', target: '', key: '' });
  await worker.stop(500);
});

test('the node of the context is the document\'s, and the lease\'s step id only when the document has none', async () => {
  const seen: string[] = [];
  const worker = host();
  worker.registerActivity('charge', (_input, ctx) => void seen.push(ctx.node));
  fake.leases = [
    lease({ input_json: JSON.stringify({ node: 'from-document', input: {}, steps: {} }) }),
    lease({ task_id: 'run-1/charge/2', generation: 2, step_id: 'the-step-id', input_json: JSON.stringify({ input: {}, steps: {} }) }),
  ];
  await worker.start();
  await until(() => seen.length === 2, 'both tasks');
  assert.deepEqual([...seen].sort(), ['from-document', 'the-step-id']);
  await worker.stop(500);
});

test('a failed poll is followed by a pause, a refusal that asks for a pace is obeyed, and a full host does not poll in a loop', async () => {
  // UNAVAILABLE: backoff, not a loop.
  fake.failures['pollTasks'] = Array.from({ length: 5000 }, () => ({ code: 14 }));
  const down = host();
  down.registerActivity('charge', () => null);
  await down.start();
  await new Promise((resolve) => setTimeout(resolve, 700));
  const unavailable = fake.of('pollTasks').length;
  await down.stop(300);
  assert.ok(unavailable >= 2 && unavailable <= 14, `${unavailable} polls in 0.7 s of failures`);

  // RESOURCE_EXHAUSTED with retry-after-ms: exactly the pause asked for.
  fake.reset();
  fake.failures['pollTasks'] = [{ code: 8, retryAfterMs: 600 }];
  const slow = host();
  slow.registerActivity('charge', () => null);
  await slow.start();
  await until(() => fake.of('pollTasks').length === 1, 'the refused poll');
  await new Promise((resolve) => setTimeout(resolve, 350));
  assert.equal(fake.of('pollTasks').length, 1, 'no second poll before the pause asked for');
  await slow.stop(300);

  // A host with every slot busy pauses between looks.
  fake.reset();
  fake.leases = [lease()];
  const full = host({ maxInflight: 1 });
  full.registerActivity('charge', () => new Promise((resolve) => setTimeout(resolve, 700)));
  await full.start();
  await until(() => fake.of('pollTasks').length >= 1, 'the first poll');
  await new Promise((resolve) => setTimeout(resolve, 500));
  assert.ok(fake.of('pollTasks').length <= 14, `${fake.of('pollTasks').length} polls while full`);
  await full.stop(1500);
});

test('a static token is not asked for again when the registration is refused', async () => {
  fake.failures['registerWorker'] = [{ code: 16 }, { code: 16 }];
  const worker = host({ token: 'static-token' });
  worker.registerActivity('charge', () => null);
  await assert.rejects(worker.start(), (error: unknown) => error instanceof SagatError && error.code === 'UNAUTHENTICATED');
  assert.equal(fake.of('registerWorker').length, 1, 'one attempt: a static token cannot be refreshed');
});

test('the heartbeat interval is never under 500 ms unless the caller sets it', async () => {
  fake.leases = [lease({ expires_at_epoch_ms: Date.now() + 600 })];
  const worker = host();
  worker.registerActivity('charge', () => new Promise((resolve) => setTimeout(resolve, 1600)));
  await worker.start();
  await until(() => fake.of('heartbeat').length >= 1, 'a heartbeat', 3000);
  const times = fake.of('heartbeat').length;
  assert.ok(times <= 4, `${times} heartbeats in 1.6 s with a lease of 0.6 s: a third of it is 200 ms, the floor makes it 500`);
  await worker.stop(2500);
});

test('stop and start back to back are in order: the host that was stopped is started again and serves', async () => {
  const worker = host();
  worker.registerActivity('charge', () => ({ ok: true }));
  await worker.start();
  const stopping = worker.stop(500); // not awaited: a deploy hook
  const starting = worker.start();
  await Promise.all([stopping, starting]);
  fake.leases = [lease({ task_id: 'run-1/charge/9', generation: 9 })];
  await until(() => fake.of('completeStep').length === 1, 'a task served by the restarted host');
  await worker.stop(500);
});

test('stop cancels the poll the server holds and nothing else: a report that is on its way arrives', async () => {
  fake.leases = [lease()];
  fake.completeDelayMs = 1500;
  const events: string[] = [];
  const worker = host({ onEvent: (event) => void events.push(event.type) });
  worker.registerActivity('charge', () => ({ ok: true }));
  await worker.start();
  await until(() => fake.of('completeStep').length === 1, 'the report to be sent');
  fake.pollDelayMs = 8000; // the next poll is held by the server
  await new Promise((resolve) => setTimeout(resolve, 100));
  await worker.stop(600); // the poll is cancelled at 600 ms; the report (answered at 1500 ms) must not be
  await until(() => events.includes('task.completed'), 'the report to be acknowledged', 3000);
});

// ------------------------------------------------------------------------------------------ hand-back

// The hand-back of SPEC §6.2: on a session of protocol 1.8 or later, a lease the host received and will not run is
// reported with FailStep TASK_NOT_STARTED, before any handler of it runs, once, and never for a task whose handler
// started; below 1.8 it lapses (mirrors handback_test.go of Go).

/** What the tests reach inside a host: the barrier of the receive loop and whether `stop` has begun. */
type HandBackSeam = { frameReceived?: (leases: number) => Promise<void> | void; stopping: boolean };
const seamOf = (h: WorkerHost): HandBackSeam => h as unknown as HandBackSeam;

function deferred(): { promise: Promise<void>; resolve: () => void } {
  let resolve: () => void = () => undefined;
  const promise = new Promise<void>((done) => { resolve = done; });
  return { promise, resolve };
}

const numbered = (n: number, overrides: Partial<TaskLease> = {}): TaskLease => lease({
  task_id: `run-${n}/charge/1`, run_id: `run-${n}`, lease_token: `LEASE-TOKEN-${n}`, idempotency_key: `run-${n}:charge`,
  input_json: JSON.stringify({ run: { id: `run-${n}` }, node: 'charge', input: {} }), ...overrides,
});

function capturingLogger(): { lines: Array<{ level: string; message: string; fields: Record<string, unknown> }>; logger: WorkerConfig['logger'] } {
  const lines: Array<{ level: string; message: string; fields: Record<string, unknown> }> = [];
  const at = (level: string) => (message: string, fields?: Record<string, unknown>) => { lines.push({ level, message, fields: fields ?? {} }); };
  return { lines, logger: { debug: at('debug'), info: at('info'), warn: at('warn'), error: at('error') } };
}

/** The warnings that hand a task back, as `task:reason`. */
const handBackWarnings = (lines: ReturnType<typeof capturingLogger>['lines']): string[] =>
  lines.filter((line) => line.level === 'warn' && line.message.includes('hands back'))
    .map((line) => `${String(line.fields['task'])}:${String(line.fields['reason'])}`).sort();

/**
 * The task ids of the FailStep calls, sorted, after checking that each is the whole hand-back: the code TASK_NOT_STARTED,
 * retryable, the fixed message, the failure's idempotency key and the lease's fence and identity.
 */
function handedBack(): string[] {
  const ids: string[] = [];
  for (const call of fake.of('failStep')) {
    const request = call.request;
    const n = String(request['run_id']).slice('run-'.length);
    assert.equal(request['error_code'], 'TASK_NOT_STARTED', `every failure here is a hand-back: ${JSON.stringify(request)}`);
    assert.equal(request['retryable'], true);
    assert.equal(request['safe_message'], 'the host did not start this task');
    assert.equal(request['idempotency_key'], `failed-${String(request['task_id'])}`);
    assert.equal(request['task_id'], `run-${n}/charge/1`);
    assert.equal(request['step_id'], 'charge');
    assert.equal(request['attempt'], 1);
    assert.equal(Number(request['generation']), 1);
    assert.equal(request['lease_token'], `LEASE-TOKEN-${n}`, 'the fence of the lease itself');
    assert.equal(request['traceparent'], lease().traceparent);
    ids.push(String(request['task_id']));
  }
  return ids.sort();
}

const beatsOf = (task: string): number => fake.of('heartbeat').filter((call) => call.request['task_id'] === task).length;

/** A host whose session negotiates protocol 1.8, the first minor on which a host hands a lease back (decision D006-080). */
function handBackHost(extra: Partial<WorkerConfig> = {}): WorkerHost {
  fake.acceptedVersion = '1.8';
  return host({ protocolVersion: '1.8', ...extra });
}

// Below protocol 1.8 the host drops the same leases and reports none of them: a server of 1.6 or 1.7 would record
// TASK_NOT_STARTED as an ordinary failure that spends the attempt, so the lease lapses as before (decision D006-080).
test('below protocol 1.8 a lease the host drops lapses without a hand-back', async () => {
  fake.acceptedVersion = '1.7';
  fake.holdStream = true;
  fake.streamBatches = [[numbered(1), numbered(2), numbered(3)]];
  const { lines, logger } = capturingLogger();
  const events: WorkerEvent[] = [];
  const h = host({
    maxInflight: 1, protocolVersion: '1.7', logger,
    onEvent: (event) => { if (event.type === 'task.failed' || event.type === 'task.dropped') events.push(event); },
  });
  h.registerActivity('charge', () => 1);
  await h.start();
  await until(() => fake.of('completeStep').length === 1, 'the kept task to complete');
  // Stop waits for every hand-back on its way, so a hand-back, had one been queued, has reached the fake.
  await h.stop(5000);
  assert.equal(fake.of('failStep').length, 0, 'a dropped lease lapses below protocol 1.8, it is never handed back');
  assert.deepEqual(events, [], 'a lease that lapses emits no failure event');
  assert.deepEqual(
    lines.filter((line) => line.level === 'warn' && line.message.includes('its lease lapses'))
      .map((line) => `${String(line.fields['task'])}:${String(line.fields['reason'])}`).sort(),
    ['run-2/charge/1:credit', 'run-3/charge/1:credit'],
    'a warning names each dropped task, the reason and that its lease lapses',
  );
});

// From protocol 1.8 the same leases are handed back (decision D006-080).
test('from protocol 1.8 a lease the host drops is handed back', async () => {
  fake.holdStream = true;
  fake.streamBatches = [[numbered(1), numbered(2), numbered(3)]];
  const h = handBackHost({ maxInflight: 1 });
  h.registerActivity('charge', () => 1);
  await h.start();
  await until(() => fake.of('completeStep').length === 1, 'the kept task to complete');
  await h.stop(5000);
  assert.deepEqual(handedBack(), ['run-2/charge/1', 'run-3/charge/1'], 'the two leases beyond the credit');
});

test('a poll answer beyond the credit is handed back', async () => {
  fake.ignoreMax = true;
  fake.leases = [numbered(1), numbered(2), numbered(3)];
  const { lines, logger } = capturingLogger();
  const ran: string[] = [];
  const h = handBackHost({ maxInflight: 1, transport: 'poll', logger });
  h.registerActivity('charge', (_input, ctx) => { ran.push(ctx.taskId); return 1; });
  await h.start();
  await until(() => fake.of('completeStep').length === 1, 'the kept task to complete');
  await h.stop(5000);
  assert.deepEqual(handedBack(), ['run-2/charge/1', 'run-3/charge/1'], 'the two leases beyond the credit');
  assert.deepEqual(ran, ['run-1/charge/1'], 'only the first lease runs');
  assert.equal(beatsOf('run-2/charge/1') + beatsOf('run-3/charge/1'), 0, 'a lease that is handed back is never beaten for');
  assert.deepEqual(handBackWarnings(lines), ['run-2/charge/1:credit', 'run-3/charge/1:credit'], 'a warning names the task and the reason');
  assert.ok(lines.every((line) => !JSON.stringify(line).includes('LEASE-TOKEN')), 'never the lease token in a log line');
});

test('a stream frame beyond the credit is handed back and the started task is not', async () => {
  fake.holdStream = true;
  fake.streamBatches = [[numbered(1), numbered(2), numbered(3)]];
  const release = deferred();
  const h = handBackHost({ maxInflight: 1, heartbeatIntervalMs: 20 });
  h.registerActivity('charge', () => release.promise.then(() => 1));
  await h.start();
  await until(() => fake.of('failStep').length === 2, 'the two hand-backs');
  await until(() => beatsOf('run-1/charge/1') > 0, 'the started task to beat');
  release.resolve();
  await until(() => fake.of('completeStep').length === 1, 'the kept task to complete');
  await h.stop(5000);
  assert.deepEqual(handedBack(), ['run-2/charge/1', 'run-3/charge/1']);
  assert.equal(fake.of('failStep').length, 2, 'the started task is never handed back');
  assert.equal(beatsOf('run-2/charge/1') + beatsOf('run-3/charge/1'), 0, 'a lease that is handed back is never beaten for');
});

test('the rest of a frame over the input budget is handed back', async () => {
  fake.holdStream = true;
  const heavy = { input_json: JSON.stringify({ run: { id: 'run-1' }, node: 'charge', input: 'x'.repeat(5 * 1024 * 1024) }) };
  fake.streamBatches = [[numbered(1, heavy), numbered(2, heavy), numbered(3)]];
  const { lines, logger } = capturingLogger();
  const h = handBackHost({ maxInflight: 3, logger });
  h.registerActivity('charge', () => 1);
  await h.start();
  await until(() => fake.of('completeStep').length === 1, 'the kept task to complete');
  await until(() => fake.of('failStep').length === 2, 'the hand-backs');
  await h.stop(5000);
  assert.deepEqual(handedBack(), ['run-2/charge/1', 'run-3/charge/1'], 'the lease over the budget and the rest of the frame');
  assert.deepEqual(handBackWarnings(lines), ['run-2/charge/1:malformed', 'run-3/charge/1:malformed']);
});

test('a stream frame read while stopping is handed back whole', async () => {
  fake.holdStream = true;
  fake.streamBatches = [[numbered(1), numbered(2)]];
  const { lines, logger } = capturingLogger();
  const h = handBackHost({ maxInflight: 4, logger });
  const read = deferred();
  const proceed = deferred();
  seamOf(h).frameReceived = async () => {
    read.resolve();
    await proceed.promise;
  };
  let ran = 0;
  h.registerActivity('charge', () => { ran += 1; return 1; });
  await h.start();
  await read.promise;
  const stopped = h.stop(5000);
  await until(() => seamOf(h).stopping, 'the stop to begin');
  proceed.resolve();
  await stopped;
  // Stop returned: the hand-backs were made before the channel closed.
  assert.deepEqual(handedBack(), ['run-1/charge/1', 'run-2/charge/1'], 'the whole frame');
  assert.equal(ran, 0, 'a host that is stopping starts nothing new');
  assert.deepEqual(handBackWarnings(lines), ['run-1/charge/1:stopping', 'run-2/charge/1:stopping']);
});

test('a poll answer that arrives while stopping is handed back and the running task is not', async () => {
  fake.leases = [numbered(1)];
  const h = handBackHost({ maxInflight: 4, transport: 'poll' });
  const started = deferred();
  const release = deferred();
  h.registerActivity('charge', async () => {
    started.resolve();
    await release.promise;
    return 1;
  });
  // The second poll answer that carries leases is held in the loop until the stop has begun.
  const holding = deferred();
  const hold = deferred();
  let answers = 0;
  seamOf(h).frameReceived = async (leases) => {
    if (leases > 0 && ++answers === 2) {
      holding.resolve();
      await hold.promise;
    }
  };
  await h.start();
  await started.promise;
  fake.leases.push(numbered(2), numbered(3));
  await holding.promise;
  const stopped = h.stop(5000);
  await until(() => seamOf(h).stopping, 'the stop to begin');
  hold.resolve();
  await until(() => fake.of('failStep').length === 2, 'the hand-backs');
  release.resolve();
  await stopped;
  assert.deepEqual(handedBack(), ['run-2/charge/1', 'run-3/charge/1']);
  assert.equal(fake.of('completeStep').length, 1, 'the running task reports its own result');
  assert.equal(fake.of('completeStep')[0]!.request['task_id'], 'run-1/charge/1');
});

test('a duplicate lease is ignored and never handed back', async () => {
  fake.holdStream = true;
  // The second is the running task's own lease again; the fourth repeats a lease handed back.
  fake.streamBatches = [[numbered(1), numbered(1), numbered(2), numbered(2)]];
  const { lines, logger } = capturingLogger();
  const release = deferred();
  const h = handBackHost({ maxInflight: 1, logger });
  h.registerActivity('charge', () => release.promise.then(() => 1));
  await h.start();
  await until(() => fake.of('failStep').length >= 1, 'the hand-back');
  release.resolve();
  await until(() => fake.of('completeStep').length === 1, 'the kept task to complete');
  await h.stop(5000);
  assert.deepEqual(handedBack(), ['run-2/charge/1'], 'a duplicate is never reported, a lease is handed back once');
  assert.equal(lines.filter((line) => line.level === 'warn' && line.message.includes('ignored')).length, 2, 'each duplicate is logged');
});

const sibling = (index: number): TaskLease => numbered(1, {
  instance: { instance_path: [{ scope_node_id: 'each', iteration_index: index }], definition_node_id: 'charge' },
});

test('sibling iterations that share a task id are not duplicates', async () => {
  fake.acceptedVersion = '1.5';
  fake.holdStream = true;
  fake.streamBatches = [[sibling(0), sibling(1)]];
  const h = host({ maxInflight: 2 });
  h.registerActivity('charge', () => 1);
  await h.start();
  await until(() => fake.of('completeStep').length === 2, 'both iterations to complete');
  await h.stop(5000);
  assert.equal(fake.of('failStep').length, 0, 'sibling iterations run, neither is handed back');
});

test('a sibling iteration beyond the credit is handed back with its instance, not ignored', async () => {
  fake.holdStream = true;
  fake.streamBatches = [[sibling(0), sibling(1)]];
  const release = deferred();
  const h = handBackHost({ maxInflight: 1 });
  h.registerActivity('charge', () => release.promise.then(() => 1));
  await h.start();
  await until(() => fake.of('failStep').length === 1, 'the hand-back of the second iteration');
  release.resolve();
  await until(() => fake.of('completeStep').length === 1, 'the first iteration to complete');
  await h.stop(5000);
  assert.deepEqual(handedBack(), ['run-1/charge/1']);
  assert.deepEqual(fake.of('failStep')[0]!.request['instance'], sibling(1).instance, 'the hand-back names the iteration it gives back');
  assert.deepEqual(fake.of('completeStep')[0]!.request['instance'], sibling(0).instance);
});

test('a hand-back is retried with the same key and ends quietly when stale', async () => {
  fake.ignoreMax = true;
  fake.leases = [numbered(1), numbered(2), numbered(3)];
  fake.failures['failStep'] = [{ code: grpc.status.UNAVAILABLE }, { code: grpc.status.ABORTED, reason: 'STALE_ATTEMPT' }];
  const events: WorkerEvent[] = [];
  const h = handBackHost({
    maxInflight: 1, transport: 'poll',
    onEvent: (event) => {
      if (event.type === 'task.failed' || event.type === 'task.stale' || event.type === 'task.dropped') events.push(event);
    },
  });
  h.registerActivity('charge', () => 1);
  await h.start();
  await until(() => events.length === 2, 'the outcome of both hand-backs');
  await h.stop(5000);
  const keys = new Map<string, number>();
  for (const call of fake.of('failStep')) {
    const key = String(call.request['idempotency_key']);
    keys.set(key, (keys.get(key) ?? 0) + 1);
  }
  // Three calls for two leases: one answered UNAVAILABLE and repeated with its key, one stale.
  assert.equal(fake.of('failStep').length, 3, `hand-back calls with keys ${JSON.stringify([...keys])}`);
  assert.equal(keys.size, 2, `hand-back calls with keys ${JSON.stringify([...keys])}`);
  handedBack();
  const failed = events.filter((event) => event.type === 'task.failed' && event.code === 'TASK_NOT_STARTED' && event.retryable);
  assert.equal(failed.length, 1, `one hand-back counted as a failure with its code: ${JSON.stringify(events)}`);
  assert.equal(events.filter((event) => event.type === 'task.stale').length, 1, `one stale: ${JSON.stringify(events)}`);
});

test('a flood of leases is handed back at most eight at a time', async () => {
  fake.ignoreMax = true;
  fake.leases = Array.from({ length: 10_000 }, (_, i) => numbered(i + 1));
  const gate = deferred();
  fake.failStepGate = gate.promise;
  const ran: string[] = [];
  const h = handBackHost({ maxInflight: 1, transport: 'poll' });
  h.registerActivity('charge', (_input, ctx) => { ran.push(ctx.taskId); return 1; });
  await h.start();
  try {
    await until(() => fake.failStepsArrived >= 8, 'eight hand-backs held at the service', 20_000);
    // While the eight are held, no ninth may arrive: this window only looks for an absence.
    await new Promise((resolve) => setTimeout(resolve, 300));
    assert.equal(fake.failStepsArrived, 8, 'at most eight hand-backs on their way at once');
  } finally {
    gate.resolve();
  }
  await until(() => fake.of('failStep').length === 9_999, 'every lease beyond the first to be handed back', 120_000);
  await h.stop(30_000);
  assert.ok(fake.failStepsPeak <= 8, `peak of ${fake.failStepsPeak} hand-backs at once`);
  const ids = handedBack();
  assert.equal(ids.length, 9_999);
  assert.equal(new Set(ids).size, 9_999, 'each lease is handed back once');
  assert.ok(!ids.includes('run-1/charge/1'), 'never the task that started');
  assert.deepEqual(ran, ['run-1/charge/1']);
});

test('a lease leased again while its hand-back is on its way is ignored', async () => {
  fake.ignoreMax = true;
  fake.leases = [numbered(1), numbered(2)];
  const gate = deferred();
  fake.failStepGate = gate.promise;
  const { lines, logger } = capturingLogger();
  const ran: string[] = [];
  const h = handBackHost({ maxInflight: 1, transport: 'poll', logger });
  h.registerActivity('charge', (_input, ctx) => { ran.push(ctx.taskId); return 1; });
  // The number of polls when the answer that leases run-2 again is read: the next poll begins after it was looked at.
  let answers = 0;
  let pollsAtRepeat = -1;
  seamOf(h).frameReceived = (leases) => {
    if (leases > 0 && ++answers === 2) pollsAtRepeat = fake.of('pollTasks').length;
  };
  await h.start();
  try {
    await until(() => fake.failStepsArrived === 1, 'the hand-back of run-2 held at the service');
    await until(() => fake.of('completeStep').length === 1, 'run-1 to complete');
    fake.leases.push(numbered(2));
    await until(() => pollsAtRepeat >= 0 && fake.of('pollTasks').length > pollsAtRepeat, 'the answer that leases run-2 again to be looked at');
    assert.deepEqual(ran, ['run-1/charge/1'], 'a lease whose hand-back is on its way is in flight: never run');
    assert.equal(lines.filter((line) => line.level === 'warn' && line.message.includes('ignored') && line.fields['task'] === 'run-2/charge/1').length, 1,
      'the duplicate is logged');
  } finally {
    gate.resolve();
  }
  await h.stop(5000);
  assert.deepEqual(handedBack(), ['run-2/charge/1'], 'exactly one hand-back for it');
  assert.deepEqual(ran, ['run-1/charge/1']);
});

test('hand-backs still on their way when the grace ends are dropped once, and stop is bounded', async () => {
  fake.ignoreMax = true;
  fake.leases = Array.from({ length: 13 }, (_, i) => numbered(i + 1));
  // The first hand-back is answered UNAVAILABLE and waits five to ten seconds before it repeats, past the stop; the next seven
  // are held at the service; the last four (run-10 to run-13) wait their turn at the host.
  fake.failures['failStep'] = [{ code: grpc.status.UNAVAILABLE }];
  const gate = deferred();
  fake.failStepGate = gate.promise;
  const { lines, logger } = capturingLogger();
  const events: WorkerEvent[] = [];
  const rejections: unknown[] = [];
  const onRejection = (reason: unknown): void => { rejections.push(reason); };
  process.on('unhandledRejection', onRejection);
  const h = handBackHost({
    maxInflight: 1, transport: 'poll', logger, retry: { initialDelayMs: 10_000, maxDelayMs: 10_000 },
    onEvent: (event) => events.push(event),
  });
  h.registerActivity('charge', () => 1);
  const dropped = (): Array<{ taskId: string; reason: string }> =>
    events.flatMap((event) => (event.type === 'task.dropped' ? [{ taskId: event.taskId, reason: event.reason }] : []));
  try {
    await h.start();
    await until(() => fake.failStepsArrived === 8, 'one hand-back refused and seven held at the service');
    await until(() => fake.of('completeStep').length === 1, 'the kept task to complete');
    const began = Date.now();
    await h.stop(200);
    const took = Date.now() - began;
    // The grace and the two seconds stop gives after it.
    assert.ok(took < 200 + 2000 + 1500, `stop took ${took} ms`);
    await until(() => dropped().length === 12, 'every hand-back to be dropped', 20_000);
  } finally {
    gate.resolve();
    process.off('unhandledRejection', onRejection);
  }
  // The calls held at the service end before this test does, so that none is recorded among the next test's calls.
  await until(() => fake.failStepsOnTheirWay === 0, 'the calls held at the service to end');
  const queued = ['run-10/charge/1', 'run-11/charge/1', 'run-12/charge/1', 'run-13/charge/1'];
  assert.deepEqual(dropped().filter((d) => queued.includes(d.taskId)).map((d) => `${d.taskId}:${d.reason}`).sort(),
    queued.map((id) => `${id}:CANCELLED`), 'what waited its turn when the grace ended is not made');
  assert.deepEqual(lines.filter((line) => line.message.includes('the host has stopped')).map((line) => String(line.fields['task'])).sort(),
    queued, 'and each is logged as not made');
  const ends = new Map<string, number>();
  for (const event of events) {
    if (event.type === 'task.dropped' || event.type === 'task.failed' || event.type === 'task.stale') {
      ends.set(event.taskId, (ends.get(event.taskId) ?? 0) + 1);
    }
  }
  assert.equal(ends.size, 12, 'every lease beyond the first ends');
  assert.ok([...ends.values()].every((n) => n === 1), `each ends once: ${JSON.stringify([...ends])}`);
  assert.ok(!events.some((event) => event.type === 'task.failed'), 'none is reported after the stop');
  assert.equal(fake.failStepsArrived, 8, 'nothing more reaches the service: no connection is opened again');
  assert.deepEqual(rejections, [], 'no unhandled rejection');
});

test('a frame after the credit is used up is read and handed back whole', async () => {
  fake.holdStream = true;
  // One receive window: the first frame uses up the credit of one, the second carries two more.
  fake.streamBatches = [[numbered(1)], [numbered(2), numbered(3)]];
  const release = deferred();
  const h = handBackHost({ maxInflight: 1 });
  h.registerActivity('charge', () => release.promise.then(() => 1));
  await h.start();
  await until(() => fake.of('failStep').length === 2, 'the two hand-backs');
  assert.equal(fake.of('completeStep').length, 0, 'run-1 is still held');
  release.resolve();
  await until(() => fake.of('completeStep').length === 1, 'the kept task to complete');
  await h.stop(5000);
  assert.deepEqual(handedBack(), ['run-2/charge/1', 'run-3/charge/1'], 'the frame after the credit is read and handed back');
  assert.equal(fake.of('completeStep')[0]!.request['task_id'], 'run-1/charge/1');
});

test('a handler cannot say TASK_NOT_STARTED: only the host can', async () => {
  fake.ignoreMax = true;
  // Two forward tasks, a compensation and a reconciliation fail with the code reserved for the host; the fifth lease is
  // beyond the credit of four and is really handed back.
  fake.leases = [
    numbered(1),
    numbered(2),
    numbered(3, { activity_name: 'refund', task_kind: 'COMPENSATION' }),
    numbered(4, { activity_name: 'check', task_kind: 'RECONCILIATION', target_node: 'charge' }),
    numbered(5),
  ];
  const h = handBackHost({ maxInflight: 4, transport: 'poll' });
  h.registerActivity('charge', (_input, ctx) => {
    throw ctx.taskId === 'run-1/charge/1'
      ? ActivityFailure.retryable('TASK_NOT_STARTED', 'custom')
      : ActivityFailure.permanent('TASK_NOT_STARTED', 'custom');
  });
  h.registerCompensation('refund', () => { throw ActivityFailure.retryable('TASK_NOT_STARTED', 'custom'); });
  h.registerReconciler('check', () => { throw ActivityFailure.permanent('TASK_NOT_STARTED', 'custom'); });
  await h.start();
  await until(() => fake.of('failStep').length === 5, 'the four failures and the hand-back');
  await h.stop(5000);
  const sent = Object.fromEntries(fake.of('failStep').map((call) => [
    String(call.request['task_id']), `${String(call.request['error_code'])}/${String(call.request['retryable'])}/${String(call.request['safe_message'])}`,
  ]));
  assert.deepEqual(sent, {
    'run-1/charge/1': 'WORKER_ERROR/true/custom',
    'run-2/charge/1': 'WORKER_ERROR/false/custom',
    'run-3/charge/1': 'WORKER_ERROR/true/custom',
    'run-4/charge/1': 'WORKER_ERROR/false/custom',
    'run-5/charge/1': 'TASK_NOT_STARTED/true/the host did not start this task',
  }, "a handler's TASK_NOT_STARTED is a WORKER_ERROR with its own message and retryable; the host's hand-back keeps it");
});
