import { strict as assert } from 'node:assert';
import { createHash } from 'node:crypto';
import { test } from 'node:test';
import * as grpc from '@grpc/grpc-js';
import * as protoLoader from '@grpc/proto-loader';
import { WorkerHost, type WorkerEvent } from '../../src/index.js';
import { serveWorkflowCode } from '../../src/worker/host.js';
import { LOADER_OPTIONS, PROTO_PATH, startFakeWorkflowServer, workerServiceClient, type FakeWorkflowServer, type FakeWorkflowServerOptions } from '../fake-workflow-server.js';
import { NONDETERMINISM_CHANGED, REFERENCE_RUNS, commandsOf, type ReferenceRun } from './goldens.js';
import { bundleOf, type TestWorkflow } from './support.js';

// The worker side of workflow tasks (Decision 044-319): a WorkerHost serves each scenario run of the golden corpus to
// the fake workflow server of Task 044-30, which compares every lease's answer with the goldens (the fence, S(k), the
// replayed chain, every command's number, kind, fragment bytes, value and carrier, saga scope and typed member, and
// every failure report). A worker switch of the corpus (`worker: "B"`) is a second host with an empty cache.

type WorkflowEvent = Extract<WorkerEvent, { type: 'workflow.task' }>;

interface Served {
  fake: FakeWorkflowServer;
  events: WorkflowEvent[];
  blobs: Map<string, Buffer>;
}

/**
 * A proxy in front of the fake (which does not serve WriteBlob): it answers WriteBlob itself, keeping the bytes and
 * answering their size and SHA-256 as the server does, and forwards every other call to the fake unchanged.
 */
async function blobProxy(fake: FakeWorkflowServer, blobs: Map<string, Buffer>): Promise<{ address: string; close(): void }> {
  const definition = protoLoader.loadSync(PROTO_PATH, LOADER_OPTIONS);
  const service = (grpc.loadPackageDefinition(definition) as any).sagat_octopus_orchestrator.v1.WorkerService.service as grpc.ServiceDefinition;
  const client = workerServiceClient(fake.address);
  const implementation: grpc.UntypedServiceImplementation = {};
  for (const [name, method] of Object.entries(service)) {
    const lower = name.charAt(0).toLowerCase() + name.slice(1);
    if (name === 'WriteBlob') {
      implementation[name] = (call: grpc.ServerReadableStream<any, any>, callback: grpc.sendUnaryData<any>) => {
        const chunks: Buffer[] = [];
        call.on('data', (part: { chunk?: Buffer }) => { if (part.chunk) chunks.push(Buffer.from(part.chunk)); });
        call.on('end', () => {
          const bytes = Buffer.concat(chunks);
          const id = `test-blob-${blobs.size + 1}`;
          blobs.set(id, bytes);
          callback(null, { blob_id: id, size: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex'), content_type: 'application/json' });
        });
      };
    } else if (method.responseStream) {
      implementation[name] = (call: grpc.ServerWritableStream<any, any>) => {
        const upstream = client[lower](call.request) as grpc.ClientReadableStream<any>;
        upstream.on('data', (frame: unknown) => call.write(frame));
        upstream.on('error', (error: grpc.ServiceError) => call.emit('error', error));
        upstream.on('end', () => call.end());
      };
    } else {
      implementation[name] = (call: grpc.ServerUnaryCall<any, any>, callback: grpc.sendUnaryData<any>) => {
        client[lower](call.request, (error: grpc.ServiceError | null, response: unknown) => callback(error, response));
      };
    }
  }
  const server = new grpc.Server();
  server.addService(service, implementation);
  const port = await new Promise<number>((resolve, reject) => server.bindAsync('127.0.0.1:0', grpc.ServerCredentials.createInsecure(), (error, bound) => (error ? reject(error) : resolve(bound))));
  return { address: `127.0.0.1:${port}`, close: () => { server.forceShutdown(); client.close(); } };
}

function hostFor(fake: FakeWorkflowServer, address: string, reference: ReferenceRun, name: string, served: Served,
  onEvent: (event: WorkflowEvent, host: WorkerHost) => void, sources: Record<string, TestWorkflow | string> = reference.versions): WorkerHost {
  const commands = fake.golden.commands;
  const host = new WorkerHost({
    grpcUrl: `http://${address}`, namespace: 'default', token: 'test-token', workerName: name, maxInflight: 1, activationDeadlineMs: 100,
    onEvent: (event) => { if (event.type === 'workflow.task') { served.events.push(event); onEvent(event, host); } },
  });
  serveWorkflowCode(host, Object.entries(sources).map(([version, source]) => ({
    workflowType: commands.workflowType, codeDigest: commands.codeVersions[version]!, bundleCode: bundleOf({ [commands.workflowType]: source }).code,
  })));
  return host;
}

/** The activations each worker of the corpus serves, in order: `A` unless the golden names another. */
function workersOf(reference: ReferenceRun): string[] {
  const commands = commandsOf(reference.scenario, reference.run);
  const workers: string[] = [];
  for (const activation of commands.activations) {
    for (const failed of activation.failedAttempts ?? []) workers.push(String(failed['worker'] ?? activation.worker ?? 'A'));
    if (activation.window.throughSeq !== null) workers.push(String(activation.worker ?? 'A'));
  }
  return workers;
}

async function serve(reference: ReferenceRun, options: Partial<FakeWorkflowServerOptions> = {}, changed?: Record<string, string>): Promise<Served> {
  const fake = await startFakeWorkflowServer({ scenario: reference.scenario, run: reference.run, ...options });
  const served: Served = { fake, events: [], blobs: new Map() };
  const proxy = await blobProxy(fake, served.blobs);
  const workers = workersOf(reference);
  try {
    let index = 0;
    while (index < workers.length && !fake.underReview) {
      const worker = workers[index]!;
      // The leases this worker serves in a row; it stops (its cache with it) before the next worker's first lease.
      let last = index;
      while (last + 1 < workers.length && workers[last + 1] === worker) last++;
      const count = last - index + 1;
      let seen = 0;
      await new Promise<void>((resolve, reject) => {
        const host = hostFor(fake, proxy.address, reference, `worker-${worker}`, served, (_event, self) => {
          seen++;
          if (seen === count || fake.underReview) {
            // Stopped from inside its last report: the host asks for no further lease.
            void self.stop().then(resolve, reject);
          }
        }, worker !== 'A' && changed ? changed : reference.versions);
        host.start().catch(reject);
      });
      index = last + 1;
    }
    return served;
  } finally {
    proxy.close();
    await fake.close();
  }
}

const served = REFERENCE_RUNS.filter((reference) => reference.corpusDefect === undefined);

for (const reference of served) {
  test(`a host serves ${reference.scenario}/${reference.run} as the corpus says`, { timeout: 120_000 }, async () => {
    const options: Partial<FakeWorkflowServerOptions> = reference.scenario === 'code-history-paging' ? { firstPageMaxEvents: 0 } : {};
    const changed = reference.scenario === 'code-nondeterminism' ? { v1: NONDETERMINISM_CHANGED } : undefined;
    const { fake, events, blobs } = await serve(reference, options, changed);
    assert.deepEqual(fake.mismatches, [], 'the fake found no difference from the goldens');
    assert.ok(fake.finished || fake.underReview, 'every lease of the golden was served');
    const commands = fake.golden.commands;
    // Every completed activation was answered with its golden outcome; every failed attempt with its cause.
    const completed = commands.activations.filter((activation) => activation.window.throughSeq !== null);
    assert.deepEqual(fake.completions.map(({ k, outcome }) => [k, outcome]), completed.map((activation) => [activation.k, activation.outcome ?? 'WORKFLOW_TASK_ACCEPTED']));
    const failed = commands.activations.flatMap((activation) => (activation.failedAttempts ?? []).map((attempt) => [activation.k, attempt.cause]));
    assert.deepEqual(fake.failures.map(({ k, cause }) => [k, cause]), failed);
    // The cache did what the golden says: a cold worker replays from sequence 1, a warm one reuses its run.
    for (const activation of commands.activations) {
      if (activation.cache === undefined) continue;
      const event = events.find((e) => e.taskId === `wt-${activation.k}-${(activation.failedAttempts?.length ?? 0) + 1}`);
      assert.equal(event?.cacheHit, activation.cache === 'hit', `activation ${activation.k} is a cache ${String(activation.cache)}`);
    }
    // The pages the host read are the golden's.
    for (const activation of commands.activations) {
      if (!activation.historyPages) continue;
      assert.deepEqual(fake.historyPages.filter((page) => page.k === activation.k).map(({ fromSeq, throughSeq, events: n, lastPage }) => ({ fromSeq, throughSeq, events: n, lastPage })),
        activation.historyPages);
    }
    // A value written as a blob holds the canonical bytes of the golden's descriptor.
    for (const bytes of blobs.values()) {
      const digest = createHash('sha256').update(bytes).digest('hex');
      const golden = commands.activations.flatMap((activation) => [...(activation.commands ?? []), ...(activation.deferred ? [activation.deferred] : [])])
        .find((command) => command.valueBlob?.descriptor.sha256 === digest);
      assert.ok(golden, `the blob ${digest} is a golden value`);
      assert.equal(bytes.length, golden.valueBlob!.descriptor.size);
    }
  });
}

for (const reference of REFERENCE_RUNS.filter((run) => run.corpusDefect !== undefined)) {
  test(`${reference.scenario}/${reference.run} cannot be served while its corpus defect stands`, async () => {
    // The defect was relayed to the server lane; this test fails once the corpus is fixed, so the run joins the others.
    const { fake } = await serve(reference);
    assert.ok(fake.underReview || fake.failures.length > 0, `expected the defect to show: ${reference.corpusDefect}`);
  });
}

test('a host refuses workflow types and activities together, and a protocol below 1.10', async () => {
  const config = { grpcUrl: 'http://127.0.0.1:1', namespace: 'default', token: 't', workerName: 'w' };
  const both = new WorkerHost(config);
  both.registerActivity('a.b', async () => null);
  serveWorkflowCode(both, [{ workflowType: 'order', codeDigest: `sha256:${'a'.repeat(64)}`, bundleCode: bundleOf({ order: async () => null }).code }]);
  await Promise.all([
    assert.rejects(both.start(), (error: { code?: string }) => error.code === 'VALIDATION_FAILED'),
    assert.rejects((() => {
      const old = new WorkerHost({ ...config, protocolVersion: '1.9' });
      serveWorkflowCode(old, [{ workflowType: 'order', codeDigest: `sha256:${'a'.repeat(64)}`, bundleCode: bundleOf({ order: async () => null }).code }]);
      return old.start();
    })(), (error: { code?: string }) => error.code === 'UNSUPPORTED_PROTOCOL'),
  ]);
});

test('a lease of a digest the host does not serve is never answered', async () => {
  const reference = REFERENCE_RUNS.find((run) => run.scenario === 'code-run-lifecycle')!;
  const fake = await startFakeWorkflowServer({ scenario: reference.scenario, run: reference.run });
  try {
    const events: WorkflowEvent[] = [];
    await new Promise<void>((resolve, reject) => {
      const host = new WorkerHost({
        grpcUrl: `http://${fake.address}`, namespace: 'default', token: 'test-token', workerName: 'other-digest', maxInflight: 1,
        onEvent: (event) => { if (event.type === 'workflow.task') { events.push(event); void host.stop().then(resolve, reject); } },
      });
      serveWorkflowCode(host, [{ workflowType: 'order', codeDigest: `sha256:${'0'.repeat(64)}`, bundleCode: bundleOf({ order: reference.versions['v1']! }).code }]);
      host.start().catch(reject);
    });
    assert.deepEqual(events.map(({ outcome }) => outcome), ['DROPPED']);
    assert.deepEqual(fake.received.filter((call) => /WorkflowTask$/.test(call.method)), [], 'no report was made');
  } finally {
    await fake.close();
  }
});
