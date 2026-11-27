// Step-log capture once the process's secret set is full (sdks/SPEC.md §6.7, §6.9): a file of its own, since a full
// set stops capture for the whole process and every later test of the process would see it.
import { strict as assert } from 'node:assert';
import { dirname, join } from 'node:path';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import * as grpc from '@grpc/grpc-js';
import * as protoLoader from '@grpc/proto-loader';
import { WorkerHost } from '../src/index.js';

const here = dirname(fileURLToPath(import.meta.url));
const PROTO = join(here, '..', '..', 'proto', 'worker.proto');
const calls: Array<{ method: string; request: any }> = [];
let leases: any[] = [];
let secrets: Array<{ name: string; value: Buffer; version: number }> = [];
const server = new grpc.Server();
let port = 0;

before(async () => {
  const definition = protoLoader.loadSync(PROTO, { keepCase: true, longs: Number, enums: String, defaults: true, oneofs: true });
  const loaded = grpc.loadPackageDefinition(definition) as any;
  const record = (method: string) => (call: any, callback: any, answer: unknown) => {
    calls.push({ method, request: call.request });
    callback(null, answer);
  };
  server.addService(loaded.sagat_octopus_orchestrator.v1.WorkerService.service, {
    registerWorker: (c: any, cb: any) => record('registerWorker')(c, cb, { worker_id: 'w', session_token: 's', accepted_protocol_version: '1.7', step_logs: { enabled: true } }),
    pollTasks: (c: any, cb: any) => cb(null, { tasks: leases.splice(0, c.request.max_tasks), retry_after_ms: 20 }),
    heartbeat: (_c: any, cb: any) => cb(null, { lease_expires_at_epoch_ms: Date.now() + 60_000, cancel_requested: false }),
    completeStep: (c: any, cb: any) => record('completeStep')(c, cb, { command_id: 'c', already_applied: false, step_status: 'SUCCEEDED', run_status: 'RUNNING' }),
    failStep: (c: any, cb: any) => record('failStep')(c, cb, { command_id: 'c', already_applied: false, step_status: 'FAILED', run_status: 'RUNNING' }),
    resolveSecrets: (_c: any, cb: any) => cb(null, { secrets }),
    appendStepLogs: (c: any, cb: any) => record('appendStepLogs')(c, cb, { verdict: 'STEP_LOG_VERDICT_CONTINUE', stop_reason: 'STEP_LOG_STOP_REASON_UNSPECIFIED', stored_through_sequence: 0, retry_after_ms: 0 }),
  });
  port = await new Promise<number>((resolve, reject) =>
    server.bindAsync('127.0.0.1:0', grpc.ServerCredentials.createInsecure(), (error, bound) => (error ? reject(error) : resolve(bound))));
});
after(() => server.forceShutdown());

async function until(condition: () => boolean, what: string): Promise<void> {
  const end = Date.now() + 8000;
  while (!condition()) {
    if (Date.now() > end) assert.fail(`timed out waiting for ${what}`);
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

test('once the secret set is full nothing more is sent: the buffer is dropped and the report carries the final mark alone', async () => {
  const warnings: string[] = [];
  const host = new WorkerHost({
    grpcUrl: `http://127.0.0.1:${port}`, namespace: 'payments', token: 't', workerName: 'full', transport: 'poll', maxInflight: 1,
    logger: { debug() {}, info() {}, warn: (m: string) => warnings.push(m), error() {} },
  });
  let filled: () => void = () => undefined;
  const full = new Promise<void>((resolve) => { filled = resolve; });
  host.registerActivity('work', async (_input, context) => {
    for (let n = 1; n <= 10; n++) context.log.info(`before ${n}`);
    await full;
    await context.secrets(); // the 1,025th value does not fit
    for (let n = 1; n <= 300; n++) context.log.info(`after ${n}`);
    // A tick of a second would find the records buffered before the set filled: it sends nothing now.
    await new Promise((resolve) => setTimeout(resolve, 1200));
    return { ok: true };
  });
  await host.start();
  secrets = Array.from({ length: 1025 }, (_, n) => ({ name: `s${n}`, value: Buffer.from(`secret-${String(n).padStart(6, '0')}`), version: 1 }));
  leases = [{
    task_id: 'run/work/1', run_id: 'run', step_id: 'step', attempt: 1, generation: 1, lease_token: 'lt', expires_at_epoch_ms: Date.now() + 60_000,
    activity_name: 'work', input_json: '{"input":{}}', input_blob_ref: '', traceparent: '', tenant_alias: '', idempotency_key: 'k', task_kind: '',
    target_node: '', target_idempotency_key: '', instance: { instance_path: [], definition_node_id: 'work' },
  }];
  await until(() => calls.some((c) => c.method === 'registerWorker'), 'registration');
  filled();
  await until(() => calls.some((c) => c.method === 'completeStep'), 'the report');
  await host.stop(1000);
  assert.equal(calls.filter((c) => c.method === 'appendStepLogs').length, 0, 'no batch, no tick, no flush');
  const carry = calls.find((c) => c.method === 'completeStep')!.request['step_logs'];
  assert.deepEqual(carry.records, []);
  assert.equal(carry.highest_sequence, 310);
  assert.equal(warnings.filter((w) => /secret set/.test(w)).length, 1);
});
