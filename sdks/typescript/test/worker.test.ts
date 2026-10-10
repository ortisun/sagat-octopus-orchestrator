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

const here = dirname(fileURLToPath(import.meta.url));
const PROTO = join(here, '..', '..', 'proto', 'worker.proto');

type Call = { method: string; request: Record<string, unknown>; metadata: grpc.Metadata };

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
  pollRetryAfterMs = 50;
  private server = new grpc.Server();
  port = 0;

  async start(): Promise<void> {
    const definition = protoLoader.loadSync(PROTO, { keepCase: true, longs: Number, enums: String, defaults: true, oneofs: true });
    const loaded = grpc.loadPackageDefinition(definition) as unknown as {
      sagat_octopus_orchestrator: { v1: { WorkerService: grpc.ServiceClientConstructor } };
    };
    const impl: Record<string, grpc.handleUnaryCall<any, any>> = {};
    const reply = (method: string, answer: (request: any) => unknown): void => {
      impl[method] = (call, callback) => {
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
          setTimeout(() => callback(null, answer(call.request)), this.pollDelayMs);
          return;
        }
        callback(null, answer(call.request));
      };
    };
    reply('registerWorker', () => ({ worker_id: 'w-1', session_token: 'session-1', accepted_protocol_version: this.acceptedVersion }));
    reply('pollTasks', (request: { max_tasks: number }) => {
      // Like the real service: no more than were asked for.
      const tasks = this.leases.splice(0, request.max_tasks);
      return { tasks, retry_after_ms: this.pollRetryAfterMs };
    });
    reply('heartbeat', () => ({ lease_expires_at_epoch_ms: this.heartbeatExpiresMs, cancel_requested: this.cancelRequested }));
    reply('completeStep', () => ({ command_id: 'c', already_applied: false, step_status: 'SUCCEEDED', run_status: 'RUNNING' }));
    reply('failStep', () => ({ command_id: 'c', already_applied: false, step_status: 'FAILED', run_status: 'RUNNING' }));
    reply('resolveSecrets', () => ({ secrets: this.secrets }));
    this.server.addService(loaded.sagat_octopus_orchestrator.v1.WorkerService.service, impl as unknown as grpc.UntypedServiceImplementation);
    this.port = await new Promise<number>((resolve, reject) =>
      this.server.bindAsync('127.0.0.1:0', grpc.ServerCredentials.createInsecure(), (error, port) => (error ? reject(error) : resolve(port))),
    );
  }

  stop(): void {
    this.server.forceShutdown();
  }

  of(method: string): Call[] {
    return this.calls.filter((call) => call.method === method);
  }

  reset(): void {
    this.calls.length = 0;
    this.leases = [];
    this.failures = {};
    this.acceptedVersion = '1.2';
    this.cancelRequested = false;
    this.heartbeatExpiresMs = Date.now() + 60_000;
    this.secrets = [];
    this.pollDelayMs = 0;
    this.pollRetryAfterMs = 50;
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
  const end = Date.now() + ms;
  while (!condition()) {
    if (Date.now() > end) assert.fail(`timed out waiting for ${what}`);
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

test('a worker registers its activities and the newest protocol it speaks, with the token as metadata', async () => {
  const h = host({ languages: ['typescript'], maxInflight: 7 });
  h.registerActivity('b.second', () => 1).registerCompensation('a.undo', () => 1).registerReconciler('c.lookup', () => reconcile.unknown());
  await h.start();
  const [register] = fake.of('registerWorker');
  assert.ok(register);
  assert.deepEqual(register.request, {
    worker_name: 'test-worker', namespace: 'payments',
    capabilities: { languages: ['typescript'], activities: ['a.undo', 'b.second', 'c.lookup'], pools: [], max_inflight: 7, protocol_version: '1.2' },
  });
  assert.equal(register.metadata.get('authorization')[0], 'Bearer tok');
  assert.equal(h.protocolVersion, '1.2');
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
    await new Promise((resolve) => setTimeout(resolve, 150));
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
  const worker = host({ heartbeatIntervalMs: 10, retry: { initialDelayMs: 30, maxDelayMs: 60 } });
  worker.registerActivity('charge', () => ({ ok: true }));
  await worker.start();
  await until(() => fake.of('completeStep').length >= 4, 'the report to go through after its retries');
  const order = fake.calls.map((call) => call.method).filter((method) => method === 'heartbeat' || method === 'completeStep');
  const first = order.indexOf('completeStep');
  const last = order.lastIndexOf('completeStep');
  assert.ok(order.slice(first, last).includes('heartbeat'), `a heartbeat between the first report and the last: ${order.join(',')}`);
  const lastBeat = order.lastIndexOf('heartbeat');
  assert.ok(lastBeat < last || order.slice(last).every((m) => m !== 'heartbeat'), 'none after the report that was made');
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
