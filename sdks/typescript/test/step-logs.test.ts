// Step-log capture (design 041, sdks/SPEC.md §6.9): numbering, redaction, ceilings, batches, the answers of
// AppendStepLogs, the carry and the wait before a report, against an in-memory worker service.
import { strict as assert } from 'node:assert';
import { dirname, join } from 'node:path';
import { after, afterEach, before, test } from 'node:test';
import { inspect } from 'node:util';
import { fileURLToPath } from 'node:url';
import * as grpc from '@grpc/grpc-js';
import * as protoLoader from '@grpc/proto-loader';
import protobuf from 'protobufjs';
import { ActivityFailure, WorkerHost, type WorkerConfig, type WorkerEvent } from '../src/index.js';
import type { TaskLease } from '../src/worker/grpc.js';
import {
  LIMITS, SecretSet, buildFields, buildRecord, carrySize, cutUtf8, recordSize, requestOverhead, wellFormed,
  type StepLogRecord,
} from '../src/worker/step-logs.js';

const here = dirname(fileURLToPath(import.meta.url));
const PROTO = join(here, '..', '..', 'proto', 'worker.proto');

// ---- the encoded sizes are protobuf's own ---------------------------------------------------------------------

const root = protobuf.loadSync(PROTO);
const RecordType = root.lookupType('sagat_octopus_orchestrator.v1.StepLogRecord');
const CarryType = root.lookupType('sagat_octopus_orchestrator.v1.StepLogCarry');
const RequestType = root.lookupType('sagat_octopus_orchestrator.v1.AppendStepLogsRequest');
const encodedLength = (type: protobuf.Type, value: object): number => type.encode(type.fromObject(value)).finish().length;
// What the SDK hands the channel, in protobufjs's names; an absent value stays absent, as on the channel.
const defined = (value: Record<string, unknown>): Record<string, unknown> =>
  Object.fromEntries(Object.entries(value).filter(([, v]) => v !== undefined));
const camel = (record: StepLogRecord): Record<string, unknown> => defined({
  sequence: record.sequence, workerTimeEpochMs: record.worker_time_epoch_ms, level: record.level, message: record.message,
  exception: record.exception, truncated: record.truncated,
  fields: record.fields?.map((f) => defined({ key: f.key, stringValue: f.string_value, intValue: f.int_value, doubleValue: f.double_value, boolValue: f.bool_value })),
});

test('the sizes the SDK counts are the bytes protobuf writes, defaults and oneof values included', () => {
  const records: StepLogRecord[] = [
    { sequence: 1, worker_time_epoch_ms: 1_795_779_247_123, level: 'STEP_LOG_LEVEL_INFO', message: 'é' },
    { sequence: 2 ** 40, worker_time_epoch_ms: 0, level: 'STEP_LOG_LEVEL_TRACE', fields: [
      { key: 'zero', int_value: 0 }, { key: 'neg', int_value: -5 }, { key: 'big', int_value: '9223372036854775807' },
      { key: 'no', bool_value: false }, { key: 'empty', string_value: '' }, { key: 'd', double_value: 0.25 }, { key: '', string_value: 'x' },
    ], exception: 'Error: x\n    at y', truncated: true },
    { sequence: 300, worker_time_epoch_ms: 5, level: 'STEP_LOG_LEVEL_ERROR', message: 'm'.repeat(8192) },
  ];
  for (const record of records) assert.equal(recordSize(record), encodedLength(RecordType, camel(record)), String(record.sequence));
  const sizes = records.map(recordSize);
  assert.equal(carrySize(sizes, 2 ** 40), encodedLength(CarryType, { records: records.map(camel), highestSequence: 2 ** 40 }));
  assert.equal(carrySize([], 300), encodedLength(CarryType, { highestSequence: 300 }));
  const identity = {
    task_id: 'run-1/charge/1', run_id: '0195b2c0-0000-7000-8000-000000000001', step_id: '0195b2c0-0000-7000-8000-000000000002',
    attempt: 3, generation: 77, lease_token: 'lt-123',
    instance: { instance_path: [{ scope_node_id: 'items', iteration_index: 0 }, { scope_node_id: 'inner', iteration_index: 129 }], definition_node_id: 'each' },
  };
  const overhead = requestOverhead(identity, 'worker-1', 'session-token');
  const full = encodedLength(RequestType, {
    workerId: 'worker-1', sessionToken: 'session-token', taskId: identity.task_id, generation: 77, leaseToken: 'lt-123',
    runId: identity.run_id, stepId: identity.step_id, attempt: 3,
    instance: { instancePath: identity.instance.instance_path.map((s) => ({ scopeNodeId: s.scope_node_id, iterationIndex: s.iteration_index })), definitionNodeId: 'each' },
    records: records.map(camel),
  });
  assert.equal(overhead + sizes.reduce((sum, size) => sum + 1 + (size < 128 ? 1 : size < 16384 ? 2 : 3) + size, 0), full);
});

test('a message is cut at the last character boundary under its ceiling, and text is made well formed', () => {
  const [cut, wasCut] = cutUtf8(`a${'é'.repeat(4500)}`, 8192);
  assert.equal(wasCut, true);
  assert.equal(Buffer.byteLength(cut), 8191);
  assert.equal(cut, `a${'é'.repeat(4095)}`);
  assert.deepEqual(cutUtf8('short', 8192), ['short', false]);
  assert.equal(wellFormed('a\u0000b\ud800c\udc00d😀'), 'a�b�c�d😀');
});

const none = { redact: (text: string) => text };

test('fields keep their types, drop a null, write anything else as compact JSON and name the non-finite doubles', () => {
  const { fields, truncated } = buildFields({
    text: 'plain', count: 42, ratio: 0.25, flag: true, nothing: null, gone: undefined, list: [1, 'two', true],
    nan: Number.NaN, inf: Number.POSITIVE_INFINITY, minf: Number.NEGATIVE_INFINITY, nul: 'a\u0000b', big: 2n ** 63n - 1n, huge: 2n ** 64n,
  }, none);
  assert.equal(truncated, false);
  assert.deepEqual(fields, [
    { key: 'big', int_value: '9223372036854775807' }, { key: 'count', int_value: 42 }, { key: 'flag', bool_value: true },
    { key: 'huge', string_value: '18446744073709551616' }, { key: 'inf', string_value: 'Infinity' },
    { key: 'list', string_value: '[1,"two",true]' }, { key: 'minf', string_value: '-Infinity' }, { key: 'nan', string_value: 'NaN' },
    { key: 'nul', string_value: 'a�b' }, { key: 'ratio', double_value: 0.25 }, { key: 'text', string_value: 'plain' },
  ]);
});

test('fields over 32 keys keep the first 32 in key order, and over 2,048 bytes stop before the one that passes', () => {
  const many: Record<string, number> = {};
  for (let n = 1; n <= 40; n++) many[`k${String(n).padStart(2, '0')}`] = n;
  const keys = buildFields(many, none);
  assert.equal(keys.truncated, true);
  assert.deepEqual(keys.fields.map((f) => f.key), Object.keys(many).slice(0, 32));
  const big = buildFields({ a: 'x'.repeat(1100), b: 'y'.repeat(1100), c: true }, none);
  assert.equal(big.truncated, true);
  assert.deepEqual(big.fields, [{ key: 'a', string_value: 'x'.repeat(1100) }]);
  // Exactly 2,048 bytes is kept: {"a":"x…"} is 8 bytes of frame and 2,040 of value.
  assert.equal(buildFields({ a: 'x'.repeat(2040) }, none).truncated, false);
  assert.equal(buildFields({ a: 'x'.repeat(2041) }, none).truncated, true);
});

test('keys colliding after redaction take ~2 in the order of their original bytes, and a number holding a secret is hidden', () => {
  const secret = { redact: (text: string) => text.split('canary-key-1').join('[REDACTED]').split('12345678').join('[REDACTED]') };
  const { fields } = buildFields({ 'canary-key-1': 'second', '[REDACTED]': 'first', n: 12345678, d: 1e15, ok: 7 }, secret);
  // 1e15 is a safe integer in JavaScript, so it travels as an integer; its text holds no secret here.
  assert.deepEqual(fields, [
    { key: '[REDACTED]', string_value: 'first' }, { key: '[REDACTED]~2', string_value: 'second' },
    { key: 'd', int_value: 1e15 }, { key: 'n', string_value: '[REDACTED]' }, { key: 'ok', int_value: 7 },
  ]);
});

// ---- the process's secret set ------------------------------------------------------------------------------------

test('the secret set redacts by the rule of redact: longest first, values under 6 bytes ignored, in the text left', () => {
  const secrets = new SecretSet();
  secrets.add([Buffer.from('xxxxxx'), Buffer.from('D]yyyyy'), Buffer.from('short'), Buffer.from('abcdefgh'), Buffer.from('abcdef')]);
  // "D]yyyyy" (7 bytes) goes before "xxxxxx" (6): the match the second replacement creates is not looked for again.
  assert.equal(secrets.redact('xxxxxxyyyyy'), '[REDACTED]yyyyy');
  assert.equal(secrets.redact('abcdefgh and abcdef and short'), '[REDACTED] and [REDACTED] and short');
  assert.equal(secrets.redact('nothing here'), 'nothing here');
  assert.doesNotMatch(inspect(secrets, { depth: 5 }), /abcdef|xxxxxx/);
  assert.doesNotMatch(JSON.stringify(secrets), /abcdef|xxxxxx/);
  assert.doesNotMatch(String(secrets), /abcdef|xxxxxx/);
});

test('the process secret set is reachable from the module by no reader and no redaction, only by an add', async () => {
  const module = await import('../src/worker/step-logs.js');
  for (const [name, value] of Object.entries(module)) {
    assert.ok(!(value instanceof SecretSet), `${name} exports a secret set`);
    if (value && typeof value === 'object') {
      for (const inner of Object.values(value)) assert.ok(!(inner instanceof SecretSet), `${name} holds a secret set`);
    }
  }
  assert.equal('processCapture' in module, false);
});

test('redaction runs on the text as formatted: a secret holding a NUL is found before the NUL is replaced', () => {
  const secrets = new SecretSet();
  secrets.add([Buffer.from('pa\u0000ssword123')]);
  const record = buildRecord(1, 1, 'info', 'x pa\u0000ssword123 y', { 'k\u0000pa\u0000ssword123': 'pa\u0000ssword123' }, new Error('pa\u0000ssword123'), secrets);
  assert.equal(record.message, 'x [REDACTED] y');
  assert.deepEqual(record.fields, [{ key: 'k\ufffd[REDACTED]', string_value: '[REDACTED]' }]);
  assert.match(record.exception!, /^Error: \[REDACTED\]/);
});

test('a secret inside an object, an array or a byte buffer is redacted, whatever the encoder escapes', () => {
  const secret = 'line"one\\two\nthree';
  const secrets = new SecretSet();
  secrets.add([Buffer.from(secret), Buffer.from('bytes-secret-77')]);
  const { fields } = buildFields({
    nested: { inner: [secret, { [secret]: 1 }], plain: 'ok' },
    list: ['a', secret],
    raw: Buffer.from('bytes-secret-77'),
    view: new Uint8Array(Buffer.from('bytes-secret-77')),
  }, secrets);
  const text = JSON.stringify(fields);
  assert.doesNotMatch(text, /line|three|bytes-secret-77/, text);
  assert.deepEqual(fields, [
    { key: 'list', string_value: '["a","[REDACTED]"]' },
    { key: 'nested', string_value: '{"inner":["[REDACTED]",{"[REDACTED]":1}],"plain":"ok"}' },
    { key: 'raw', string_value: '[REDACTED]' },
    { key: 'view', string_value: '[REDACTED]' },
  ]);
});

test('a secret that is not valid UTF-8 joins the set as the text the accessor hands out', () => {
  const secrets = new SecretSet();
  const bytes = Buffer.concat([Buffer.from([0xff]), Buffer.from('s3cr3t-token-42')]);
  secrets.add([bytes]);
  assert.equal(secrets.redact(`logged ${bytes.toString('utf8')}`), 'logged [REDACTED]');
});

test('the secret set holds 1,024 distinct values or 256 KiB, and the one that does not fit stops capture once', () => {
  const secrets = new SecretSet();
  const values = Array.from({ length: 1024 }, (_, n) => Buffer.from(`secret-${String(n).padStart(6, '0')}`));
  assert.equal(secrets.add(values), false);
  assert.equal(secrets.add([values[0]!]), false, 'a value already held takes no room');
  assert.equal(secrets.full, false);
  assert.equal(secrets.add([Buffer.from('one-value-too-many')]), true);
  assert.equal(secrets.full, true);
  assert.equal(secrets.add([Buffer.from('and-another-one')]), false, 'stopped once');
  const bytes = new SecretSet();
  assert.equal(bytes.add([Buffer.from('x'.repeat(LIMITS.secretBytes))]), false);
  assert.equal(bytes.add([Buffer.from('y'.repeat(6))]), true, 'past 256 KiB');
});

// ---- the host, against an in-memory worker service --------------------------------------------------------------

type Call = { method: string; request: Record<string, any>; at: number };
type Scripted =
  | { verdict: string; stop_reason?: string; retry_after_ms?: number }
  | { code: number; retryAfterMs?: number }
  | { hold: Promise<void>; then?: { verdict: string; stop_reason?: string } };

class Fake {
  readonly calls: Call[] = [];
  leases: TaskLease[] = [];
  accepted = '1.7';
  enabled: boolean | undefined = true;
  appendAnswers: Scripted[] = [];
  completeFailures: number[] = [];
  secrets: Array<{ name: string; value: Buffer; version: number }> = [];
  registrations = 0;
  private server = new grpc.Server();
  port = 0;

  async start(): Promise<void> {
    const definition = protoLoader.loadSync(PROTO, { keepCase: true, longs: Number, enums: String, defaults: true, oneofs: true });
    const loaded = grpc.loadPackageDefinition(definition) as unknown as {
      sagat_octopus_orchestrator: { v1: { WorkerService: grpc.ServiceClientConstructor } };
    };
    const record = (method: string, request: any): void => {
      this.calls.push({ method, request, at: performance.now() });
    };
    const fail = (callback: grpc.sendUnaryData<any>, code: number, retryAfterMs?: number): void => {
      const metadata = new grpc.Metadata();
      if (retryAfterMs !== undefined) metadata.set('retry-after-ms', String(retryAfterMs));
      callback({ code, details: 'scripted', metadata } as grpc.ServiceError);
    };
    const impl: Record<string, any> = {
      registerWorker: (call: any, callback: any) => {
        record('registerWorker', call.request);
        this.registrations += 1;
        callback(null, {
          worker_id: `w-${this.registrations}`, session_token: `session-${this.registrations}`, accepted_protocol_version: this.accepted,
          ...(this.enabled === undefined ? {} : { step_logs: { enabled: this.enabled } }),
        });
      },
      pollTasks: (call: any, callback: any) => {
        // A poll of a host a previous test stopped (its session is not the newest) takes nothing.
        if (call.request.session_token !== `session-${this.registrations}`) return callback(null, { tasks: [], retry_after_ms: 50 });
        record('pollTasks', call.request);
        callback(null, { tasks: this.leases.splice(0, call.request.max_tasks), retry_after_ms: 50 });
      },
      heartbeat: (call: any, callback: any) => callback(null, { lease_expires_at_epoch_ms: Date.now() + 60_000, cancel_requested: false }),
      completeStep: (call: any, callback: any) => {
        record('completeStep', call.request);
        const code = this.completeFailures.shift();
        if (code !== undefined) return fail(callback, code);
        callback(null, { command_id: 'c', already_applied: false, step_status: 'SUCCEEDED', run_status: 'RUNNING' });
      },
      failStep: (call: any, callback: any) => {
        record('failStep', call.request);
        callback(null, { command_id: 'c', already_applied: false, step_status: 'FAILED', run_status: 'RUNNING' });
      },
      resolveSecrets: (call: any, callback: any) => callback(null, { secrets: this.secrets }),
      appendStepLogs: (call: any, callback: any) => {
        record('appendStepLogs', call.request);
        const answer = this.appendAnswers.shift();
        const ok = (verdict = 'STEP_LOG_VERDICT_CONTINUE', stop_reason = 'STEP_LOG_STOP_REASON_UNSPECIFIED', retry_after_ms = 0) =>
          callback(null, { verdict, stop_reason, stored_through_sequence: 0, retry_after_ms });
        if (!answer) return ok();
        if ('hold' in answer) return void answer.hold.then(() => ok(answer.then?.verdict, answer.then?.stop_reason));
        if ('code' in answer) return fail(callback, answer.code, answer.retryAfterMs);
        return ok(answer.verdict, answer.stop_reason, answer.retry_after_ms);
      },
    };
    this.server.addService(loaded.sagat_octopus_orchestrator.v1.WorkerService.service, impl);
    this.port = await new Promise<number>((resolve, reject) =>
      this.server.bindAsync('127.0.0.1:0', grpc.ServerCredentials.createInsecure(), (error, port) => (error ? reject(error) : resolve(port))));
  }

  of(method: string): Call[] {
    return this.calls.filter((call) => call.method === method);
  }

  reset(): void {
    this.calls.length = 0;
    this.leases = [];
    this.accepted = '1.7';
    this.enabled = true;
    this.appendAnswers = [];
    this.completeFailures = [];
    this.secrets = [];
  }

  stop(): void {
    this.server.forceShutdown();
  }
}

let fake: Fake;
before(async () => {
  fake = new Fake();
  await fake.start();
});
after(() => fake.stop());
const hosts: WorkerHost[] = [];
afterEach(async () => {
  while (hosts.length) await hosts.pop()?.stop(1000);
  fake.reset();
});

let leaseCount = 0;
// Leases are queued once the test's host has registered: a poll still on its way from a host a previous test stopped
// is answered empty then, since its session is no longer the newest.
function lease(overrides: Partial<TaskLease> = {}): TaskLease {
  leaseCount += 1;
  return {
    task_id: `run-${leaseCount}/work/1`, run_id: `run-${leaseCount}`, step_id: `step-${leaseCount}`, attempt: 1, generation: 1, lease_token: 'lt',
    expires_at_epoch_ms: Date.now() + 60_000, activity_name: 'work', input_json: JSON.stringify({ input: {}, steps: {} }),
    input_blob_ref: '', traceparent: '', tenant_alias: '', idempotency_key: 'k', task_kind: '', target_node: '', target_idempotency_key: '',
    instance: { instance_path: [], definition_node_id: 'work' }, ...overrides,
  };
}

function host(extra: Partial<WorkerConfig> = {}): WorkerHost {
  const created = new WorkerHost({
    grpcUrl: `http://127.0.0.1:${fake.port}`, namespace: 'payments', token: 'tok', workerName: 'logs-worker', transport: 'poll',
    maxInflight: 1, retry: { initialDelayMs: 5, maxDelayMs: 20 }, ...extra,
  });
  hosts.push(created);
  return created;
}

async function until(condition: () => boolean, what: string, ms = 8000): Promise<void> {
  const end = Date.now() + ms;
  while (!condition()) {
    if (Date.now() > end) assert.fail(`timed out waiting for ${what}; calls: ${fake.calls.map((c) => c.method).join(', ')}`);
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

const messages = (records: any[]): string[] => records.map((r) => r.message);
/** A field as the fake decoded it, without the name of its oneof member. */
const plain = (fields: any[]): any[] => fields.map(({ value: _member, ...field }) => field);
const sequences = (records: any[]): number[] => records.map((r) => r.sequence);
const range = (from: number, to: number): number[] => Array.from({ length: to - from + 1 }, (_, i) => from + i);

/** Runs one activity that logs `count` lines named `prefix n` and returns; resolves with the report's request. */
async function runOne(count: number, extra: Partial<WorkerConfig> = {}, end?: (log: any) => unknown): Promise<Record<string, any>> {
  const h = host(extra);
  h.registerActivity('work', (_input, context) => {
    for (let n = 1; n <= count; n++) context.log.info(`line ${n}`);
    return end ? end(context.log) : { ok: true };
  });
  await h.start();
  fake.leases = [lease()];
  await until(() => fake.of('completeStep').length + fake.of('failStep').length > 0, 'the report');
  return (fake.of('completeStep')[0] ?? fake.of('failStep')[0])!.request;
}

test('a short step at 1.7 carries its lines on its report and makes no AppendStepLogs call', async () => {
  const report = await runOne(3);
  assert.equal(fake.of('appendStepLogs').length, 0);
  assert.deepEqual(messages(report['step_logs'].records), ['line 1', 'line 2', 'line 3']);
  assert.deepEqual(sequences(report['step_logs'].records), [1, 2, 3]);
  assert.equal(report['step_logs'].highest_sequence, 3);
  assert.equal(report['step_logs'].records[0].level, 'STEP_LOG_LEVEL_INFO');
});

test('nothing is captured below 1.7, with the policy off, without a policy, or with the option off', async () => {
  for (const [setup, extra] of [
    [() => { fake.accepted = '1.5'; }, {}],
    [() => { fake.enabled = false; }, {}],
    [() => { fake.enabled = undefined; }, {}],
    [() => undefined, { stepLogs: { capture: false } }],
  ] as Array<[() => void, Partial<WorkerConfig>]>) {
    setup();
    const report = await runOne(300, extra);
    assert.equal(report['step_logs'], null, JSON.stringify(extra));
    assert.equal(fake.of('appendStepLogs').length, 0);
    while (hosts.length) await hosts.pop()?.stop(1000);
    fake.reset();
  }
});

test('a long step sends the batch its 256th call cut, waits for its answer, and carries the rest', async () => {
  let release: () => void = () => undefined;
  fake.appendAnswers = [{ hold: new Promise<void>((resolve) => { release = resolve; }) }];
  const reported = runOne(260);
  await until(() => fake.of('appendStepLogs').length === 1, 'the batch');
  const [batch] = fake.of('appendStepLogs');
  assert.deepEqual(sequences(batch!.request['records']), range(1, 256));
  assert.equal(batch!.request['session_token'], `session-${fake.registrations}`);
  assert.equal(batch!.request['step_id'], `step-${leaseCount}`);
  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.equal(fake.of('completeStep').length, 0, 'the report waits for the batch in flight');
  release();
  const report = await reported;
  assert.deepEqual(sequences(report['step_logs'].records), range(257, 260));
  assert.equal(report['step_logs'].highest_sequence, 260);
  assert.ok(fake.of('completeStep')[0]!.at > batch!.at);
});

test('an answer held past 2 seconds is given up: the report goes with the newest records that fit', async () => {
  let release: () => void = () => undefined;
  const drops: Array<[string, number]> = [];
  fake.appendAnswers = [{ hold: new Promise<void>((resolve) => { release = resolve; }) }];
  let ended = 0;
  const report = await runOne(260, { onEvent: (e) => { if (e.type === 'steplogs.dropped') drops.push([e.reason, e.records]); } }, () => {
    ended = performance.now();
    return { ok: true };
  });
  // The wait is the SDK's own: from the handler's end, not from anything before it.
  const waited = fake.of('completeStep')[0]!.at - ended;
  release();
  assert.ok(waited >= LIMITS.reportWaitMs - 1, `reported ${waited} ms after the handler ended`);
  assert.deepEqual(sequences(report['step_logs'].records), range(257, 260));
  assert.deepEqual(drops, [['report_deadline', 256]], 'the abandoned batch is counted');
});

test('stopping the host leaves an attempt in its report wait to its report, and flushes one still running as its batch in flight', async () => {
  let release: () => void = () => undefined;
  fake.appendAnswers = [{ hold: new Promise<void>((resolve) => { release = resolve; }) }];
  const h = host({ maxInflight: 2 });
  let handlerEnded = false;
  let running: () => void = () => undefined;
  const stillRunning = new Promise<void>((resolve) => { running = resolve; });
  h.registerActivity('work', async (_input, context) => {
    if (context.attempt === 1) {
      for (let n = 1; n <= 260; n++) context.log.info(`waiting ${n}`);
      handlerEnded = true;
      return { ok: true };
    }
    for (let n = 1; n <= 10; n++) context.log.info(`running ${n}`);
    running();
    await new Promise<void>((resolve) => context.signal.addEventListener('abort', () => resolve()));
    return { stopped: true };
  });
  await h.start();
  fake.leases = [lease(), lease({ attempt: 2 })];
  await until(() => handlerEnded && fake.of('appendStepLogs').length === 1, 'the held batch of the attempt in its report wait');
  await stillRunning;
  const stopping = h.stop(200);
  await until(() => fake.of('appendStepLogs').length === 2, 'the shutdown flush of the running attempt');
  release();
  await stopping;
  const flushed = fake.of('appendStepLogs')[1]!.request['records'];
  assert.deepEqual(messages(flushed), range(1, 10).map((n) => `running ${n}`));
  const waiting = fake.of('completeStep').find((c) => c.request['attempt'] === 1);
  assert.ok(waiting, 'the attempt in its report wait still reported');
  assert.deepEqual(sequences(waiting.request['step_logs'].records), range(257, 260));
});

for (const [answer, expected] of [
  [undefined, []],
  [{ code: grpc.status.UNAVAILABLE }, [['send_failed', 10]]],
] as Array<[Scripted | undefined, Array<[string, number]>]>) {
  test(`a shutdown batch of a handler that then honours the cancel is counted only when it fails (${answer ? 'UNAVAILABLE' : 'CONTINUE'})`, async () => {
    const drops: Array<[string, number]> = [];
    let release: () => void = () => undefined;
    const held = new Promise<void>((resolve) => { release = resolve; });
    fake.appendAnswers = [answer ? { hold: held, then: undefined } : { hold: held }];
    if (answer) fake.appendAnswers = [{ code: grpc.status.UNAVAILABLE }];
    const h = host({ onEvent: (e) => { if (e.type === 'steplogs.dropped') drops.push([e.reason, e.records]); } });
    let logged: () => void = () => undefined;
    const ready = new Promise<void>((resolve) => { logged = resolve; });
    h.registerActivity('work', async (_input, context) => {
      for (let n = 1; n <= 10; n++) context.log.info(`running ${n}`);
      logged();
      await new Promise<void>((resolve) => context.signal.addEventListener('abort', () => resolve()));
      throw Object.assign(new Error('cancelled'), { name: 'AbortError' });
    });
    await h.start();
    fake.leases = [lease()];
    await ready;
    const stopping = h.stop(50);
    await until(() => fake.of('appendStepLogs').length === 1, 'the shutdown batch');
    release();
    await stopping;
    await until(() => !answer || drops.length > 0, 'the count of the failed batch');
    assert.equal(fake.of('appendStepLogs').length, 1, 'never sent again');
    assert.deepEqual(drops, expected);
  });
}

test('an UNAUTHENTICATED answer counts its batch as one the attempt gave up on', async () => {
  const drops: Array<[string, number]> = [];
  fake.appendAnswers = [{ code: grpc.status.UNAUTHENTICATED }];
  const report = await runOne(260, { onEvent: (e) => { if (e.type === 'steplogs.dropped') drops.push([e.reason, e.records]); } });
  assert.equal(report['step_logs'], null);
  assert.deepEqual(drops.sort(), [['send_failed', 260]]);
});

test('a batch holds at most 500 records when more are buffered as its turn comes', async () => {
  let release: () => void = () => undefined;
  fake.appendAnswers = [{ hold: new Promise<void>((resolve) => { release = resolve; }) }];
  const h = host();
  h.registerActivity('work', async (_input, context) => {
    for (let n = 1; n <= 256; n++) context.log.info(`a ${n}`);
    for (let n = 257; n <= 956; n++) context.log.info(`b ${n}`);
    release();
    await until(() => fake.of('appendStepLogs').length >= 2, 'the second batch');
    return { ok: true };
  });
  await h.start();
  fake.leases = [lease()];
  await until(() => fake.of('completeStep').length === 1, 'the report');
  const appends = fake.of('appendStepLogs');
  assert.deepEqual(sequences(appends[1]!.request['records']), range(257, 756));
});

test('FINAL_ONLY drops what is buffered and later calls, and the report carries the final mark with no record', async () => {
  fake.appendAnswers = [{ verdict: 'STEP_LOG_VERDICT_FINAL_ONLY' }];
  const report = await runOne(300);
  assert.equal(fake.of('appendStepLogs').length, 1);
  assert.deepEqual(report['step_logs'].records, []);
  assert.equal(report['step_logs'].highest_sequence, 300);
});

test('STOP_ATTEMPT sends no carry, and STOP_CAPTURE stops the next attempt of the session', async () => {
  fake.appendAnswers = [{ verdict: 'STEP_LOG_VERDICT_STOP_ATTEMPT', stop_reason: 'STEP_LOG_STOP_REASON_LEASE' }];
  assert.equal((await runOne(300))['step_logs'], null);
  while (hosts.length) await hosts.pop()?.stop(1000);
  fake.reset();
  fake.appendAnswers = [{ verdict: 'STEP_LOG_VERDICT_STOP_CAPTURE', stop_reason: 'STEP_LOG_STOP_REASON_CAPTURE_OFF' }];
  const h = host();
  h.registerActivity('work', (_input, context) => {
    for (let n = 1; n <= 300; n++) context.log.info(`line ${n}`);
    return { ok: true };
  });
  await h.start();
  fake.leases = [lease(), lease()];
  await until(() => fake.of('completeStep').length === 2, 'both reports');
  assert.equal(fake.of('appendStepLogs').length, 1, 'the second attempt of the session sends nothing');
  for (const report of fake.of('completeStep')) assert.equal(report.request['step_logs'], null);
});

test('STOP_CAPTURE for the session is answered by sending once more with the token the host now holds', async () => {
  // The batch's answer comes once the host has registered again: the session it sent is not the one it holds.
  let answer: () => void = () => undefined;
  fake.appendAnswers = [{
    hold: new Promise<void>((resolve) => { answer = resolve; }),
    then: { verdict: 'STEP_LOG_VERDICT_STOP_CAPTURE', stop_reason: 'STEP_LOG_STOP_REASON_SESSION' },
  }];
  const h = host();
  h.registerActivity('work', (_input, context) => {
    for (let n = 1; n <= 260; n++) context.log.info(`line ${n}`);
    return { ok: true };
  });
  await h.start();
  fake.leases = [lease()];
  await until(() => fake.of('appendStepLogs').length === 1, 'the batch');
  const sent = fake.of('appendStepLogs')[0]!.request['session_token'];
  await (h as unknown as { register(): Promise<void> }).register();
  answer();
  await until(() => fake.of('completeStep').length === 1, 'the report');
  const appends = fake.of('appendStepLogs');
  assert.equal(appends.length, 2);
  assert.notEqual(appends[1]!.request['session_token'], sent);
  assert.equal(appends[1]!.request['session_token'], `session-${fake.registrations}`);
  assert.deepEqual(sequences(appends[1]!.request['records']), range(1, 256));
  assert.deepEqual(sequences(fake.of('completeStep')[0]!.request['step_logs'].records), range(257, 260));
});

test('STOP_CAPTURE for the session with the token the host still holds stops capture and sends no carry', async () => {
  fake.appendAnswers = [{ verdict: 'STEP_LOG_VERDICT_STOP_CAPTURE', stop_reason: 'STEP_LOG_STOP_REASON_SESSION' }];
  const report = await runOne(260);
  assert.equal(fake.of('appendStepLogs').length, 1);
  assert.equal(report['step_logs'], null);
});

test('RETRY_LATER and RESOURCE_EXHAUSTED send the same batch again after their delay, without counting a retry', async () => {
  fake.appendAnswers = [
    { verdict: 'STEP_LOG_VERDICT_RETRY_LATER', retry_after_ms: 60 },
    { code: grpc.status.RESOURCE_EXHAUSTED, retryAfterMs: 60 },
    { code: grpc.status.UNAVAILABLE },
    { code: grpc.status.UNAVAILABLE },
  ];
  const report = await runOne(260);
  const appends = fake.of('appendStepLogs');
  assert.equal(appends.length, 5, 'two waits, two retries, then the answer');
  for (const append of appends) assert.deepEqual(append.request['records'], appends[0]!.request['records']);
  assert.ok(appends[1]!.at - appends[0]!.at >= 55);
  assert.deepEqual(sequences(report['step_logs'].records), range(257, 260));
});

test('UNAVAILABLE three times drops the batch after its two retries, INVALID_ARGUMENT at once, and the carry still goes', async () => {
  const drops: WorkerEvent[] = [];
  fake.appendAnswers = [{ code: grpc.status.UNAVAILABLE }, { code: grpc.status.UNAVAILABLE }, { code: grpc.status.UNAVAILABLE }];
  const report = await runOne(260, { onEvent: (event) => { if (event.type === 'steplogs.dropped') drops.push(event); } });
  assert.equal(fake.of('appendStepLogs').length, 3);
  assert.deepEqual(sequences(report['step_logs'].records), range(257, 260));
  assert.deepEqual(drops.map((d) => d.type === 'steplogs.dropped' && [d.reason, d.records]), [['send_failed', 256]]);
  while (hosts.length) await hosts.pop()?.stop(1000);
  fake.reset();
  fake.appendAnswers = [{ code: grpc.status.INVALID_ARGUMENT }];
  const invalid = await runOne(260);
  assert.equal(fake.of('appendStepLogs').length, 1);
  assert.deepEqual(sequences(invalid['step_logs'].records), range(257, 260));
});

test('UNIMPLEMENTED stops the batches of the attempt, and the report carries the newest 64 records', async () => {
  const drops: Array<[string, number]> = [];
  fake.appendAnswers = [{ code: grpc.status.UNIMPLEMENTED }];
  const report = await runOne(400, { onEvent: (event) => { if (event.type === 'steplogs.dropped') drops.push([event.reason, event.records]); } });
  assert.equal(fake.of('appendStepLogs').length, 1);
  assert.deepEqual(sequences(report['step_logs'].records), range(337, 400));
  assert.equal(report['step_logs'].highest_sequence, 400);
  assert.deepEqual(drops.sort(), [['report_deadline', 80], ['send_failed', 256]]);
});

test('after UNIMPLEMENTED the attempt keeps the newest records that fit one carry, whatever it logs after', async () => {
  fake.appendAnswers = [{ code: grpc.status.UNIMPLEMENTED }];
  const drops: Array<[string, number]> = [];
  const h = host({ onEvent: (e) => { if (e.type === 'steplogs.dropped') drops.push([e.reason, e.records]); } });
  h.registerActivity('work', async (_input, context) => {
    for (let n = 1; n <= 256; n++) context.log.info(`line ${n}`);
    await until(() => fake.of('appendStepLogs').length === 1, 'the refused batch');
    await new Promise((resolve) => setImmediate(resolve));
    for (let n = 257; n <= 2256; n++) context.log.info(`line ${n}`);
    return { ok: true };
  });
  await h.start();
  fake.leases = [lease()];
  await until(() => fake.of('completeStep').length === 1, 'the report');
  assert.equal(fake.of('appendStepLogs').length, 1);
  const carry = fake.of('completeStep')[0]!.request['step_logs'];
  assert.equal(carry.highest_sequence, 2256);
  assert.deepEqual(sequences(carry.records), range(2193, 2256));
  assert.deepEqual(drops.sort(), [['report_deadline', 2000 - 64], ['send_failed', 256]]);
});

test('a batch in its back-off is not sent again once the session stopped capture, nor a real CANCELLED dropped at once', async () => {
  // Attempt 1 is in its 1-second back-off after two UNAVAILABLE when attempt 2's batch is answered STOP_CAPTURE.
  fake.appendAnswers = [
    { code: grpc.status.UNAVAILABLE }, { code: grpc.status.UNAVAILABLE },
    { verdict: 'STEP_LOG_VERDICT_STOP_CAPTURE', stop_reason: 'STEP_LOG_STOP_REASON_CAPTURE_OFF' },
  ];
  const h = host({ maxInflight: 2 });
  let second: () => void = () => undefined;
  const secondMayLog = new Promise<void>((resolve) => { second = resolve; });
  h.registerActivity('work', async (_input, context) => {
    if (context.attempt === 2) await secondMayLog;
    for (let n = 1; n <= 256; n++) context.log.info(`line ${n}`);
    if (context.attempt === 1) {
      await until(() => fake.of('appendStepLogs').length === 2, 'the first retry');
      second();
      await until(() => fake.of('appendStepLogs').length === 3, 'the stopping answer');
    }
    return { ok: true };
  });
  await h.start();
  fake.leases = [lease(), lease({ attempt: 2 })];
  await until(() => fake.of('completeStep').length === 2, 'both reports');
  assert.equal(fake.of('appendStepLogs').length, 3, 'no second retry after the session stopped');
  while (hosts.length) await hosts.pop()?.stop(1000);
  fake.reset();
  fake.appendAnswers = [{ code: grpc.status.CANCELLED }];
  const report = await runOne(260);
  assert.equal(fake.of('appendStepLogs').length, 2, 'a CANCELLED status is retried');
  assert.deepEqual(sequences(report['step_logs'].records), range(257, 260));
});

test('a lost answer to a carrying report sends the same carry again, and a failure carries too', async () => {
  fake.completeFailures = [grpc.status.UNAVAILABLE];
  await runOne(3);
  await until(() => fake.of('completeStep').length === 2, 'the report sent again');
  const [first, second] = fake.of('completeStep');
  assert.ok(first && second);
  assert.deepEqual(second.request['step_logs'], first.request['step_logs']);
  while (hosts.length) await hosts.pop()?.stop(1000);
  fake.reset();
  const failed = await runOne(2, {}, () => { throw ActivityFailure.permanent('BROKEN', 'broken'); });
  assert.deepEqual(messages(failed['step_logs'].records), ['line 1', 'line 2']);
});

test('the capture level is the option, below it a call takes no sequence, and the host logger level does not matter', async () => {
  const lines: string[] = [];
  const logger = { debug: () => undefined, info: () => undefined, warn: () => undefined, error: (m: string) => lines.push(m) };
  const report = await runOne(0, { stepLogs: { level: 'debug' }, logger }, (log) => {
    log.trace('trace line');
    log.debug('debug line');
    log.info('info line', { k: 1 });
    log.warn('warn line');
    log.error('error line', undefined, new Error('the payment provider refused'));
    return { ok: true };
  });
  assert.deepEqual(messages(report['step_logs'].records), ['debug line', 'info line', 'warn line', 'error line']);
  assert.deepEqual(sequences(report['step_logs'].records), [1, 2, 3, 4]);
  assert.match(report['step_logs'].records[3].exception, /^Error: the payment provider refused/);
  assert.deepEqual(plain(report['step_logs'].records[1].fields), [{ key: 'k', int_value: 1 }]);
  assert.deepEqual(lines, ['error line'], 'the host still logs what it logged');
});

test('a line is redacted with every secret the process resolved, by any attempt, also after the host started again', async () => {
  fake.secrets = [{ name: 'key', value: Buffer.from('canary-value-9Kx2'), version: 1 }];
  const h = host();
  h.registerActivity('work', async (_input, context) => {
    if (context.attempt === 1) {
      await context.secrets();
      return { first: true };
    }
    context.log.info('canary-value-9Kx2', { 'canary-value-9Kx2': 'canary-value-9Kx2' }, new Error('failure canary-value-9Kx2'));
    return { second: true };
  });
  await h.start();
  fake.leases = [lease(), lease({ attempt: 2 })];
  await until(() => fake.of('completeStep').length === 2, 'both reports');
  const [record] = fake.of('completeStep')[1]!.request['step_logs'].records;
  assert.equal(record.message, '[REDACTED]');
  assert.deepEqual(plain(record.fields), [{ key: '[REDACTED]', string_value: '[REDACTED]' }]);
  assert.match(record.exception, /^Error: failure \[REDACTED\]/);
  assert.doesNotMatch(JSON.stringify(fake.of('completeStep')), /canary-value-9Kx2/);
  // The same host, stopped and started again in the process, still redacts what an earlier attempt resolved.
  await h.stop(1000);
  fake.calls.length = 0;
  await h.start();
  fake.leases = [lease({ attempt: 2 })];
  await until(() => fake.of('completeStep').length === 1, 'the report after the restart');
  assert.equal(fake.of('completeStep')[0]!.request['step_logs'].records[0].message, '[REDACTED]');
});

test('a logger call never throws into the activity, whatever it is handed', async () => {
  const circular: Record<string, unknown> = {};
  circular['self'] = circular;
  const hostile = { get boom(): never { throw new Error('getter'); } };
  const report = await runOne(0, { logger: { debug() {}, info() {}, warn() {}, error() {} } }, (log) => {
    log.info('circular', { circular, hostile: Object.defineProperty({}, 'toJSON', { value: () => { throw new Error('x'); } }) });
    log.info({ toString() { throw new Error('message'); } } as unknown as string);
    log.info('getter', hostile as unknown as Record<string, unknown>);
    return { ok: true };
  });
  assert.equal(report['step_logs'].highest_sequence, 3);
  assert.equal(fake.of('completeStep').length, 1, 'the activity completed: no call threw into it');
  assert.equal(fake.of('failStep').length, 0);
});

test('a batch holds at most 500 records and 192 KiB, and the attempt buffer stops at its bound', async () => {
  const drops: Array<[string, number]> = [];
  const big = 'm'.repeat(8000);
  let release: () => void = () => undefined;
  fake.appendAnswers = [{ hold: new Promise<void>((resolve) => { release = resolve; }) }];
  const h = host({ onEvent: (event) => { if (event.type === 'steplogs.dropped') drops.push([event.reason, event.records]); } });
  h.registerActivity('work', async (_input, context) => {
    // 8 KB lines: the attempt buffer (256 KiB) is full long before 256 records, so the tick sends.
    for (let n = 1; n <= 100; n++) context.log.info(`${n} ${big}`);
    await new Promise((resolve) => setTimeout(resolve, 1300));
    release();
    return { ok: true };
  });
  await h.start();
  fake.leases = [lease()];
  await until(() => fake.of('completeStep').length === 1, 'the report');
  const [batch] = fake.of('appendStepLogs');
  const size = encodedLength(RequestType, {
    workerId: batch!.request['worker_id'], sessionToken: batch!.request['session_token'], taskId: batch!.request['task_id'],
    generation: batch!.request['generation'], leaseToken: batch!.request['lease_token'], runId: batch!.request['run_id'],
    stepId: batch!.request['step_id'], attempt: batch!.request['attempt'],
    instance: { instancePath: [], definitionNodeId: 'work' },
    records: batch!.request['records'].map((r: any) => ({ sequence: r.sequence, workerTimeEpochMs: r.worker_time_epoch_ms, level: r.level, message: r.message })),
  });
  assert.ok(size <= LIMITS.batchBytes, `${size} bytes`);
  assert.ok(size > LIMITS.batchBytes - 8100, 'the batch is filled up to its bound');
  assert.ok(drops.some(([reason]) => reason === 'buffer_full'));
});
