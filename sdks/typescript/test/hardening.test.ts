// What a hostile or broken server, and a careless caller, must not be able to do (SPEC §1, §6.2, §6.7, §10).
// Each test here failed before the change it covers; they come from an independent security review.
import { strict as assert } from 'node:assert';
import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { dirname, join } from 'node:path';
import { inspect } from 'node:util';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import * as grpc from '@grpc/grpc-js';
import * as protoLoader from '@grpc/proto-loader';
import { SagatClient, SagatError, WorkerHost, canonicalJson, workflow } from '../src/index.js';

const here = dirname(fileURLToPath(import.meta.url));
const PROTO = join(here, '..', '..', 'proto', 'worker.proto');
const RUN_ID = '0190aaaa-0000-7000-8000-000000000001';
const CANARY = 'canary-bearer-token-8f3a';
const CANARY_KEY = '-----BEGIN PRIVATE KEY-----canary-key';

// ----------------------------------------------------------------------------- a worker service to misuse

type Handler = (call: { request: Record<string, unknown> }, callback: grpc.sendUnaryData<unknown>) => void;

class Service {
  readonly counts: Record<string, number> = {};
  tasks: Array<Record<string, unknown>> = [];
  pollAnswer: 'tasks' | 'unauthenticated' = 'tasks';
  retryAfterMs = 50;
  secrets: Array<{ name: string; value: Buffer; version: number }> = [{ name: 'api', value: Buffer.from('sk_live_SECRET'), version: 1 }];
  failSecretsOnce = false;
  private server = new grpc.Server();
  port = 0;

  async start(): Promise<void> {
    const definition = protoLoader.loadSync(PROTO, { keepCase: true, longs: Number, enums: String, defaults: true, oneofs: true });
    const loaded = grpc.loadPackageDefinition(definition) as unknown as { sagat_octopus_orchestrator: { v1: { WorkerService: grpc.ServiceClientConstructor } } };
    const count = (name: string): void => void (this.counts[name] = (this.counts[name] ?? 0) + 1);
    const impl: Record<string, Handler> = {
      registerWorker: (_call, callback) => {
        count('registerWorker');
        callback(null, { worker_id: 'w-1', session_token: 'session-canary-77', accepted_protocol_version: '1.2' });
      },
      pollTasks: (call, callback) => {
        count('pollTasks');
        if (this.pollAnswer === 'unauthenticated') {
          callback({ code: grpc.status.UNAUTHENTICATED, details: 'gone' } as grpc.ServiceError);
          return;
        }
        const asked = Number(call.request['max_tasks']);
        void asked;
        callback(null, { tasks: this.tasks.splice(0, this.tasks.length), retry_after_ms: this.retryAfterMs });
      },
      heartbeat: (_call, callback) => callback(null, { lease_expires_at_epoch_ms: Date.now() + 60_000, cancel_requested: false }),
      completeStep: (_call, callback) => {
        count('completeStep');
        callback(null, { command_id: 'c', already_applied: false, step_status: 'SUCCEEDED', run_status: 'RUNNING' });
      },
      failStep: (_call, callback) => {
        count('failStep');
        callback(null, { command_id: 'c', already_applied: false, step_status: 'FAILED', run_status: 'RUNNING' });
      },
      resolveSecrets: (_call, callback) => {
        count('resolveSecrets');
        if (this.failSecretsOnce) {
          this.failSecretsOnce = false;
          callback({ code: grpc.status.UNAVAILABLE, details: 'blip' } as grpc.ServiceError);
          return;
        }
        callback(null, { secrets: this.secrets });
      },
    };
    this.server.addService(loaded.sagat_octopus_orchestrator.v1.WorkerService.service, impl as unknown as grpc.UntypedServiceImplementation);
    this.port = await new Promise<number>((resolve, reject) => this.server.bindAsync('127.0.0.1:0', grpc.ServerCredentials.createInsecure(), (e, p) => (e ? reject(e) : resolve(p))));
  }

  stop(): void {
    this.server.forceShutdown();
  }
}

const lease = (n: number, document: string = JSON.stringify({ run: { id: RUN_ID }, node: 'charge', input: { n }, steps: {} })): Record<string, unknown> => ({
  task_id: `${RUN_ID}/charge/${n}`, run_id: RUN_ID, step_id: '0190aaaa-0000-7000-8000-0000000000aa', attempt: 1, generation: n,
  lease_token: 'lease-canary-55', expires_at_epoch_ms: Date.now() + 60_000, activity_name: 'charge', input_json: document,
  input_blob_ref: '', traceparent: '', tenant_alias: '', idempotency_key: `${RUN_ID}:charge`, task_kind: '', target_node: '', target_idempotency_key: '',
});

let service: Service;
before(async () => {
  service = new Service();
  await service.start();
});
after(() => service.stop());

const worker = (extra: Record<string, unknown> = {}): WorkerHost =>
  new WorkerHost({ grpcUrl: `http://127.0.0.1:${service.port}`, namespace: 'payments', token: CANARY, workerName: 'hardening', retry: { initialDelayMs: 5, maxDelayMs: 20 }, ...extra });

async function until(condition: () => boolean, what: string, ms = 5000): Promise<void> {
  const end = Date.now() + ms;
  while (!condition()) {
    if (Date.now() > end) assert.fail(`timed out waiting for ${what}`);
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

// ------------------------------------------------------------------------------------------------ tests

test('a session that is refused again after registering is not hammered: the registrations back off', async () => {
  const flooded = new Service();
  await flooded.start();
  flooded.pollAnswer = 'unauthenticated';
  const host = new WorkerHost({ grpcUrl: `http://127.0.0.1:${flooded.port}`, namespace: 'payments', token: 't', workerName: 'w', retry: { initialDelayMs: 50, maxDelayMs: 200 } });
  host.registerActivity('charge', () => null);
  await host.start();
  await new Promise((resolve) => setTimeout(resolve, 1500));
  await host.stop(200);
  flooded.stop();
  assert.ok((flooded.counts['registerWorker'] ?? 0) <= 30, `${flooded.counts['registerWorker']} registrations in 1.5 s`);
});

test('an integer the platform sent beyond 2^53 - 1 is refused, not rounded', async () => {
  service.tasks = [lease(1, '{"run":{"id":"r"},"node":"charge","input":{"customerId":9007199254740993},"steps":{}}')];
  const host = worker();
  const seen: unknown[] = [];
  host.registerActivity('charge', (input) => void seen.push(input));
  await host.start();
  await until(() => (service.counts['failStep'] ?? 0) >= 1, 'the step to be refused');
  await host.stop(200);
  assert.equal(seen.length, 0, 'the handler never saw a rounded id');
  // An integer that fits, and decimals/exponents (floats by nature), are untouched.
  service.tasks = [lease(2, '{"run":{"id":"r"},"node":"charge","input":{"a":9007199254740991,"b":1.5e300,"c":"9007199254740993"},"steps":{}}')];
  const ok = worker({ workerName: 'hardening-2' });
  ok.registerActivity('charge', (input) => void seen.push(input));
  await ok.start();
  await until(() => seen.length === 1, 'the safe document to be served');
  await ok.stop(200);
  assert.deepEqual(seen[0], { a: 9007199254740991, b: 1.5e300, c: '9007199254740993' });
});

test('no printable form of a client or a host shows a credential', async () => {
  const client = new SagatClient({ apiUrl: 'https://octopus.example.com/v1', namespace: 'payments', token: CANARY, tls: { key: CANARY_KEY } });
  const host = worker({ tls: undefined });
  host.registerActivity('charge', () => null);
  await host.start();
  for (const thing of [client, host]) {
    for (const text of [JSON.stringify(thing), inspect(thing, { depth: 10, showHidden: true }), String(thing), `${inspect(thing)}`]) {
      assert.ok(!text.includes(CANARY) && !text.includes('canary-key') && !text.includes('session-canary') && !text.includes('lease-canary'), text.slice(0, 200));
    }
  }
  await host.stop(200);
});

test('a gRPC URL of any scheme but http and https is refused instead of sent in plaintext', () => {
  for (const url of ['grpcs://127.0.0.1:50051', 'grpc://127.0.0.1:50051', 'dns:///octopus:443', '127.0.0.1:50051', 'ftp://x']) {
    assert.throws(
      () => new WorkerHost({ grpcUrl: url, namespace: 'payments', token: 't', workerName: 'w' }),
      (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED',
      url,
    );
  }
});

test('an event hook that throws costs the host nothing: the task is reported and the process lives', async () => {
  service.tasks = [lease(3)];
  const host = worker({ onEvent: () => { throw new Error('the metrics hook broke'); } });
  host.registerActivity('charge', () => ({ ok: true }));
  const before = service.counts['completeStep'] ?? 0;
  await host.start();
  await until(() => (service.counts['completeStep'] ?? 0) > before, 'the report');
  await host.stop(200);
});

test('secrets are copied out, wiped when the handler returns, refused afterwards, and a failed first resolution is not remembered', async () => {
  service.tasks = [lease(4)];
  service.failSecretsOnce = true;
  const host = worker({ workerName: 'hardening-secrets' });
  let leaked: { secrets: { text(n: string): string; get(n: string): Buffer }; copy: Buffer } | undefined;
  const outcomes: string[] = [];
  host.registerActivity('charge', async (_input, ctx) => {
    const secrets = await ctx.secrets().then((resolved: unknown) => resolved, () => undefined);
    outcomes.push(secrets ? 'first resolved' : 'first failed');
    const again = await ctx.secrets();
    const copy = again.get('api');
    assert.equal(copy.toString(), 'sk_live_SECRET');
    copy.fill(1); // the caller's copy is theirs: it does not touch the host's
    assert.equal(again.text('api'), 'sk_live_SECRET', 'the host kept its own');
    leaked = { secrets: again, copy: again.get('api') };
  });
  await host.start();
  await until(() => leaked !== undefined && (service.counts['completeStep'] ?? 0) >= 1, 'the handler to finish');
  await host.stop(200);
  assert.deepEqual(outcomes, ['first failed'], 'the failure was not cached for the rest of the attempt');
  assert.throws(() => leaked?.secrets.text('api'), (error: unknown) => error instanceof SagatError, 'refused after the handler');
  assert.throws(() => leaked?.secrets.get('api'), (error: unknown) => error instanceof SagatError);
});

test('a run id that is not an id, and a name that is a path segment of its own, are refused before a request', async () => {
  const requests: string[] = [];
  const server = createServer((req, res) => {
    requests.push(req.url ?? '');
    res.writeHead(200, { 'content-type': 'application/json' }).end('{}');
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  try {
    const client = new SagatClient({ apiUrl: `http://127.0.0.1:${(server.address() as AddressInfo).port}/v1`, namespace: 'payments', token: 't', retry: { maxAttempts: 1 } });
    for (const bad of ['..', '.', '', 'x', `${RUN_ID}/operations`, `${RUN_ID}\n`]) {
      await assert.rejects(client.get(bad), (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED', JSON.stringify(bad));
      await assert.rejects(client.cancel(bad, 'a good reason'), (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED');
      await assert.rejects(client.events(bad), (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED');
    }
    for (const name of ['..', '.', '']) {
      await assert.rejects(client.signal(RUN_ID, name, {}), (error: unknown) => error instanceof SagatError && error.code === 'VALIDATION_FAILED', JSON.stringify(name));
    }
    assert.equal(requests.length, 0, 'nothing was sent');
  } finally {
    server.close();
  }
});

test('an answer of more than 32 MiB is not read to the end', async () => {
  let sent = 0;
  const server = createServer((_req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' });
    const chunk = Buffer.alloc(1024 * 1024, 0x20);
    const push = (): void => {
      while (sent < 400 && !res.destroyed) {
        sent++;
        if (!res.write(chunk)) {
          res.once('drain', push);
          return;
        }
      }
      res.end();
    };
    push();
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  try {
    const client = new SagatClient({ apiUrl: `http://127.0.0.1:${(server.address() as AddressInfo).port}/v1`, namespace: 'payments', token: 't', retry: { maxAttempts: 1 }, timeoutMs: 20_000 });
    await assert.rejects(client.get(RUN_ID), (error: unknown) => error instanceof SagatError && error.code === 'PROTOCOL' && /larger/.test(error.message));
    // The client stopped reading at the cap: the server could not push the other 360 MiB through.
    await new Promise((resolve) => setTimeout(resolve, 200));
    assert.ok(sent < 200, `${sent} MiB were pushed`);
  } finally {
    server.close();
  }
});

test('a poll answer with more tasks than were asked for runs only as many as there is room for', async () => {
  service.tasks = [1, 2, 3, 4, 5, 6].map((n) => lease(100 + n));
  const host = worker({ workerName: 'hardening-room', maxInflight: 2 });
  let running = 0;
  let peak = 0;
  host.registerActivity('charge', async () => {
    running++;
    peak = Math.max(peak, running);
    await new Promise((resolve) => setTimeout(resolve, 100));
    running--;
  });
  await host.start();
  await new Promise((resolve) => setTimeout(resolve, 400));
  await host.stop(500);
  assert.ok(peak >= 1 && peak <= 2, `${peak} handlers at once`);
});

test('what the server says is bounded: a very long wait is capped, a long message cut, a trace id that is not hexadecimal not repeated', async () => {
  const server = createServer((_req, res) => {
    res.writeHead(409, { 'content-type': 'application/json', 'retry-after': '99999999' });
    res.end(JSON.stringify({ code: 'CONFLICT', message: 'x'.repeat(5000), retryable: false, traceId: '<script>not-a-trace</script>' }));
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  try {
    const client = new SagatClient({ apiUrl: `http://127.0.0.1:${(server.address() as AddressInfo).port}/v1`, namespace: 'payments', token: 't', retry: { maxAttempts: 1 } });
    await assert.rejects(client.get(RUN_ID), (error: unknown) => {
      assert.ok(error instanceof SagatError);
      assert.ok(error.message.length <= 512, `message of ${error.message.length}`);
      assert.equal(error.traceId, '');
      assert.ok((error.retryAfterMs ?? 0) <= 60_000, `retryAfterMs ${error.retryAfterMs}`);
      return true;
    });
  } finally {
    server.close();
  }
});

test('a key called __proto__ is kept in the canonical text, as in the other SDKs', () => {
  const schema = JSON.parse('{"properties":{"__proto__":{"type":"string"},"a":{"type":"string"}}}') as Record<string, never>;
  assert.equal(canonicalJson(schema), '{"properties":{"__proto__":{"type":"string"},"a":{"type":"string"}}}');
  const document = workflow('proto-check').step('a', 'x.y').inputSchema(schema).toJson();
  assert.ok(document.includes('"__proto__"'));
});

// ------------------------------------------------------------- from the follow-up security review

test('a logger that throws costs the host nothing: the heartbeat, the poll loop and the report go on', async () => {
  const exploding = {
    debug() { throw new Error('the logger broke'); },
    info() { throw new Error('the logger broke'); },
    warn() { throw new Error('the logger broke'); },
    error() { throw new Error('the logger broke'); },
  };
  service.tasks = [lease(7)];
  const host = worker({ workerName: 'hardening-logger', logger: exploding, heartbeatIntervalMs: 30 });
  let ran = 0;
  host.registerActivity('charge', async () => {
    ran++;
    await new Promise((resolve) => setTimeout(resolve, 200));
  });
  const before = service.counts['completeStep'] ?? 0;
  await host.start();
  await until(() => (service.counts['completeStep'] ?? 0) > before, 'the report with a broken logger');
  assert.equal(ran, 1);
  await host.stop(300);
  // The client logs too (a retry): a broken logger does not turn a retry into a failure.
  let attempts = 0;
  const server = createServer((_req, res) => {
    attempts++;
    if (attempts === 1) res.writeHead(503, { 'content-type': 'application/json' }).end(JSON.stringify({ code: 'UNAVAILABLE', message: 'busy', retryable: true, traceId: 'a'.repeat(32) }));
    else res.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({ runId: RUN_ID, workflowId: RUN_ID, versionId: RUN_ID, status: 'RUNNING', version: 1, paused: false, startedBy: 'a', createdAt: '2026-10-10T10:00:00Z', updatedAt: '2026-10-10T10:00:00Z', finishedAt: null, deadline: null, traceId: 'a'.repeat(32), correlationId: null, depth: 0, parent: null }));
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  try {
    const client = new SagatClient({ apiUrl: `http://127.0.0.1:${(server.address() as AddressInfo).port}/v1`, namespace: 'payments', token: 't', logger: exploding, retry: { initialDelayMs: 1, maxDelayMs: 2 } });
    assert.equal((await client.get(RUN_ID)).runId, RUN_ID);
  } finally {
    server.close();
  }
});

test('the secrets of an attempt are refused from the moment its handler returned, also to a late first call', async () => {
  service.tasks = [lease(8)];
  const host = worker({ workerName: 'hardening-late-secrets' });
  let late: Promise<unknown> | undefined;
  let kept: { text(n: string): string } | undefined;
  host.registerActivity('charge', async (_input, ctx) => {
    kept = await ctx.secrets();
    // Asked after the handler is done: this call is made by code the handler left behind.
    setTimeout(() => { late = ctx.secrets().then(() => 'resolved', (error: unknown) => (error instanceof SagatError ? error.code : 'other')); }, 150);
  });
  const resolvesBefore = service.counts['resolveSecrets'] ?? 0;
  await host.start();
  await until(() => late !== undefined, 'the late call');
  assert.equal(await late, 'VALIDATION_FAILED', 'refused');
  assert.equal((service.counts['resolveSecrets'] ?? 0) - resolvesBefore, 1, 'no second ResolveSecrets was made for a finished attempt');
  assert.throws(() => kept?.text('api'), (error: unknown) => error instanceof SagatError, 'what the handler kept is wiped as soon as it returned');
  await host.stop(300);
});

test('the 32 MiB cap holds with a tls option too: the body is streamed, not buffered first', async () => {
  let sent = 0;
  const server = createServer((_req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' });
    const chunk = Buffer.alloc(1024 * 1024, 0x20);
    const push = (): void => {
      while (sent < 400 && !res.destroyed) {
        sent++;
        if (!res.write(chunk)) {
          res.once('drain', push);
          return;
        }
      }
      res.end();
    };
    push();
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  try {
    const client = new SagatClient({ apiUrl: `http://127.0.0.1:${(server.address() as AddressInfo).port}/v1`, namespace: 'payments', token: 't', retry: { maxAttempts: 1 }, timeoutMs: 20_000, tls: {} });
    await assert.rejects(client.get(RUN_ID), (error: unknown) => error instanceof SagatError && error.code === 'PROTOCOL' && /larger/.test(error.message));
    await new Promise((resolve) => setTimeout(resolve, 200));
    assert.ok(sent < 200, `${sent} MiB were pushed through the tls transport`);
  } finally {
    server.close();
  }
});

test('a token provider that fails once is repeated: a computed result is not lost to a hiccup of the identity provider', async () => {
  service.tasks = [lease(9)];
  let failNext = false;
  let failures = 0;
  const host = worker({
    workerName: 'hardening-provider',
    token: () => {
      if (failNext) {
        failNext = false;
        failures++;
        throw new Error('the identity provider said: client_secret=hunter2');
      }
      return CANARY;
    },
  });
  host.registerActivity('charge', () => {
    failNext = true; // the next call of the provider, which is the report, fails once
    return { ok: true };
  });
  const before = service.counts['completeStep'] ?? 0;
  await host.start();
  await until(() => (service.counts['completeStep'] ?? 0) > before, 'the report after the provider failed once');
  assert.equal(failures, 1);
  await host.stop(300);

  // A REST call: the same, and a SERVER that sends the reason in an answer does not make a 401 repeatable.
  let calls = 0;
  const server = createServer((_req, res) => {
    calls++;
    res.writeHead(401, { 'content-type': 'application/json' }).end(JSON.stringify({ code: 'UNAUTHENTICATED', message: 'no', retryable: true, traceId: 'a'.repeat(32), details: { reason: 'TOKEN_PROVIDER_FAILED' } }));
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  try {
    const client = new SagatClient({ apiUrl: `http://127.0.0.1:${(server.address() as AddressInfo).port}/v1`, namespace: 'payments', token: 't', retry: { initialDelayMs: 1, maxDelayMs: 2 } });
    await assert.rejects(client.get(RUN_ID), (error: unknown) => error instanceof SagatError && error.code === 'UNAUTHENTICATED');
    assert.equal(calls, 1, 'a 401 whose body claims the reason is not repeated');
  } finally {
    server.close();
  }
  let provided = 0;
  const flaky = createServer((_req, res) => res.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({ runId: RUN_ID, workflowId: RUN_ID, versionId: RUN_ID, status: 'RUNNING', version: 1, paused: false, startedBy: 'a', createdAt: '2026-10-10T10:00:00Z', updatedAt: '2026-10-10T10:00:00Z', finishedAt: null, deadline: null, traceId: 'a'.repeat(32), correlationId: null, depth: 0, parent: null })));
  await new Promise<void>((resolve) => flaky.listen(0, '127.0.0.1', resolve));
  try {
    const client = new SagatClient({ apiUrl: `http://127.0.0.1:${(flaky.address() as AddressInfo).port}/v1`, namespace: 'payments', retry: { initialDelayMs: 1, maxDelayMs: 2 }, token: () => { if (provided++ === 0) throw new Error('blip'); return 't'; } });
    assert.equal((await client.get(RUN_ID)).runId, RUN_ID);
    assert.equal(provided, 2);
  } finally {
    flaky.close();
  }
});
