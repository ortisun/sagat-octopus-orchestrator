// The conformance runner of the TypeScript SDK (tests/contract/sdk-conformance/README.md). It uses
// only the public API of the SDK: the builders in `build` mode, the client and the worker host in
// `run` mode.
import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import {
  ActivityFailure, BlobOutput, SDK_VERSION, SagatClient, SagatError, WorkerHost, body, flow, reconcile, workflow,
  type ActivityContext, type Body, type Flow, type Run, type StateDefinition, type WorkflowBuilder,
} from '../src/index.js';
import { FaultPlan, type Fault } from './faults.js';

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
type Obj = Record<string, any>;

const read = (path: string): Obj => JSON.parse(readFileSync(path, 'utf8')) as Obj;

/** Replaces `{{unique}}` in every string of the scenario. */
function templated<T>(value: T, unique: string): T {
  if (typeof value === 'string') return value.replaceAll('{{unique}}', unique) as T;
  if (Array.isArray(value)) return value.map((v) => templated(v, unique)) as T;
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, templated(v, unique)])) as T;
  }
  return value;
}

// ----------------------------------------------------------------------------------------- builders

function apply(target: Flow, ops: Obj[]): void {
  for (const op of ops) {
    switch (op['op']) {
      case 'step': target.step(op['id'], op['activity'], op['options']); break;
      case 'sleep': target.sleep(op['id'], op['durationMs']); break;
      case 'waitForEvent': target.waitForEvent(op['id'], op['options']); break;
      case 'invokeChild': target.invokeChild(op['id'], op['workflow'], op['options']); break;
      case 'condition': target.condition(op['id'], op['expression'], op['ifTrue'], op['ifFalse']); break;
      case 'switch': target.switch(op['id'], op['cases'], op['default']); break;
      case 'forEach': target.forEach(op['id'], op['items'], op['body'], op['result'], { maxItems: op['maxItems'], maxConcurrency: op['maxConcurrency'] }); break;
      case 'map': target.map(op['id'], op['items'], op['body'], op['result'], { maxItems: op['maxItems'], maxConcurrency: op['maxConcurrency'] }); break;
      case 'while': target.while(op['id'], op['condition'], op['body'], op['initial'], op['carry'], { maxIterations: op['maxIterations'], timeoutMs: op['timeoutMs'], delayMs: op['delayMs'], onError: op['onError'] }); break;
      case 'repeatUntil': target.repeatUntil(op['id'], op['condition'], op['body'], op['initial'], op['carry'], { maxIterations: op['maxIterations'], timeoutMs: op['timeoutMs'], delayMs: op['delayMs'], onError: op['onError'] }); break;
      case 'approval': target.approval(op['id'], { policy: op['policy'], timeoutMs: op['timeoutMs'], onApprove: op['onApprove'], onReject: op['onReject'], onExpire: op['onExpire'] }); break;
      case 'continueAsNew': target.continueAsNew(op['id'], op['input'], op['state']); break;
      case 'return': (target as Body).returnValue(op['id'], op['result']); break;
      case 'parallel': {
        const branches = (op['branches'] as Obj[][]).map((ops) => {
          const branch = flow();
          apply(branch, ops);
          return branch;
        });
        target.parallel(op['id'], op['join'], branches);
        break;
      }
      case 'saga': {
        const body = flow();
        apply(body, op['body']);
        target.saga(op['id'], op['options'] ?? {}, body);
        break;
      }
      case 'end': target.end(op['id'], op['options']); break;
      default: throw new Error(`unknown builder op ${String(op['op'])}`);
    }
  }
}

function builderOf(definition: Obj): WorkflowBuilder {
  const builder = workflow(definition['name']);
  if (definition['description']) builder.description(definition['description']);
  for (const [k, v] of Object.entries(definition['labels'] ?? {})) builder.label(k, v as string);
  for (const trigger of definition['triggers'] ?? []) builder.trigger(trigger);
  if (definition['inputSchema']) builder.inputSchema(definition['inputSchema']);
  if (definition['outputSchema']) builder.outputSchema(definition['outputSchema']);
  if (definition['state']) builder.state(definition['state'] as StateDefinition);
  if (definition['chainPolicy']) builder.chainPolicy(definition['chainPolicy']);
  for (const [name, bodyDefinition] of Object.entries<Obj>(definition['bodies'] ?? {})) {
    const nested = body();
    apply(nested, bodyDefinition['program']);
    builder.body(name, nested);
  }
  apply(builder, definition['program']);
  return builder;
}

function build(scenario: Obj): Obj {
  const workflows: Record<string, Json> = {};
  const canonical: Record<string, string> = {};
  for (const [key, definition] of Object.entries<Obj>(scenario['workflows'] ?? {})) {
    const builder = builderOf(definition);
    workflows[key] = builder.build();
    // The canonical text is part of the contract (SPEC §7): the same bytes in every language.
    canonical[key] = builder.toJson();
  }
  const errors: Record<string, Json> = {};
  for (const [key, invalid] of Object.entries<Obj>(scenario['invalidWorkflows'] ?? {})) {
    try {
      // Building it and writing its canonical text: what an SDK refuses may be refused by either (SPEC §7, §8).
      const builder = builderOf(invalid['workflow']);
      builder.build();
      builder.toJson();
      errors[key] = { raised: false };
    } catch (error) {
      errors[key] = error instanceof SagatError
        ? { raised: true, code: error.code, mentions: String(invalid['contains'] ?? '') === '' || error.message.includes(String(invalid['contains'])) }
        : { raised: true, code: 'UNEXPECTED', mentions: false };
    }
  }
  return Object.keys(errors).length > 0 ? { workflows, canonical, errors } : { workflows, canonical };
}

// ---------------------------------------------------------------------------------------- observations

const iso = (date: Date | null): string | null => (date ? date.toISOString() : null);

function runJson(run: Run): Obj {
  return {
    runId: run.runId, workflowId: run.workflowId, versionId: run.versionId, status: run.status, version: run.version,
    paused: run.paused, startedBy: run.startedBy, createdAt: iso(run.createdAt), updatedAt: iso(run.updatedAt),
    finishedAt: iso(run.finishedAt), deadline: iso(run.deadline), traceId: run.traceId, correlationId: run.correlationId,
    depth: run.depth, parent: run.parent,
  };
}

function errorJson(error: unknown): Obj {
  if (error instanceof SagatError) return error.toJSON();
  return { code: 'UNEXPECTED', message: error instanceof Error ? error.name : 'unknown' };
}

// ----------------------------------------------------------------------------------------------- workers

interface WorkerDef extends Obj {
  workerName: string;
  activities: Record<string, { as?: string; behavior: Obj }>;
}

/** Waits `ms`, or until `signal` aborts; true if it was aborted. `ms` of 0 or less waits for the abort alone. */
function abortable(ms: number, signal: AbortSignal): Promise<boolean> {
  return new Promise((resolve) => {
    if (signal.aborted) return resolve(true);
    const timer = ms > 0 ? setTimeout(() => {
      signal.removeEventListener('abort', onAbort);
      resolve(false);
    }, ms) : undefined;
    const onAbort = (): void => {
      if (timer) clearTimeout(timer);
      resolve(true);
    };
    signal.addEventListener('abort', onAbort, { once: true });
  });
}

function behaviorHandler(
  name: string,
  kind: string,
  behavior: Obj,
  log: Obj[],
  measurement: { active: number; peakInflight: number; deliveries: Obj[] },
): (input: any, ctx: ActivityContext) => Promise<any> {
  return async (input, ctx) => {
    measurement.active += 1;
    measurement.peakInflight = Math.max(measurement.peakInflight, measurement.active);
    measurement.deliveries.push({ runId: ctx.runId, node: ctx.node, takenAtUnixMs: Date.now() });
    let outcome = 'returned';
    try {
      if (behavior['failsUntilAttempt'] !== undefined && ctx.attempt < behavior['failsUntilAttempt']) {
        outcome = 'failed';
        const f = behavior['fails'];
        throw new ActivityFailure(f.code, f.message, f.retryable);
      }
      if (behavior['fails'] && behavior['failsUntilAttempt'] === undefined) {
        outcome = 'failed';
        const f = behavior['fails'];
        throw new ActivityFailure(f.code, f.message, f.retryable);
      }
      if (behavior['throws'] !== undefined) {
        outcome = 'threw';
        throw new Error(String(behavior['throws']));
      }
      if (behavior['outcomeUnknown'] !== undefined) {
        outcome = 'failed';
        throw ActivityFailure.outcomeUnknown(String(behavior['outcomeUnknown']));
      }
      if (behavior['hang']) {
        await abortable(0, ctx.signal);
        outcome = 'cancelled';
        // `onCancel.raises`: the handler ends by raising the SDK's cancellation error.
        if (behavior['onCancel']?.['raises']) throw new SagatError({ code: 'CANCELLED', message: 'cancelled' });
        return null;
      }
      if (behavior['ignoresCancel']) {
        // A handler that never looks at its token: it runs to its end and what it returns is reported.
        await new Promise((resolve) => setTimeout(resolve, behavior['sleepMs'] ?? 0));
        return behavior['returns'] ?? null;
      }
      if (behavior['sleepMs'] !== undefined) {
        const cancelled = await abortable(behavior['sleepMs'], ctx.signal);
        if (cancelled) {
          outcome = 'cancelled';
          if (behavior['onCancel']?.['raises']) throw new SagatError({ code: 'CANCELLED', message: 'cancelled' });
          return behavior['onCancel']?.['returns'] ?? null;
        }
      }
      if (behavior['readsSecret'] !== undefined) {
        let secrets;
        try {
          secrets = await ctx.secrets();
        } catch (error) {
          // `reportsSecretError`: the canonical code of the refusal is the output, so the scenario can compare it.
          if (behavior['reportsSecretError'] && error instanceof SagatError) return { ...(behavior['returns'] ?? {}), secretError: error.code };
          throw error;
        }
        return { ...(behavior['returns'] ?? {}), secretLength: secrets.get(behavior['readsSecret']).length };
      }
      if (behavior['blobRoundTripBytes'] !== undefined) {
        const body = Buffer.alloc(behavior['blobRoundTripBytes'], 'x');
        const descriptor = await ctx.writeBlob([body], {
          contentType: 'application/octet-stream', expectedSize: body.length,
          expectedSha256: createHash('sha256').update(body).digest('hex'),
        });
        const chunks: Buffer[] = [];
        for await (const chunk of ctx.readBlob(descriptor)) chunks.push(chunk);
        if (!Buffer.concat(chunks).equals(body)) throw new Error('blob round trip failed');
        return { blobSize: descriptor.size, blobSha256: descriptor.sha256, roundTrip: true };
      }
      if (behavior['blobInputBytes'] !== undefined) {
        const raw = (input as Obj)['$blob'] as Obj;
        const descriptor = { id: raw['id'] as string, size: raw['size'] as number, sha256: raw['sha256'] as string, contentType: raw['contentType'] as string };
        const chunks: Buffer[] = [];
        for await (const chunk of ctx.readBlob(descriptor)) chunks.push(chunk);
        const expected = Buffer.alloc(behavior['blobInputBytes'], 'i');
        if (!Buffer.concat(chunks).equals(expected)) throw new Error('run-input blob did not match');
        return { inputBlobSize: descriptor.size, inputBlobSha256: descriptor.sha256 };
      }
      if (behavior['blobOutputBytes'] !== undefined) {
        const descriptor = await ctx.writeBlob([Buffer.alloc(behavior['blobOutputBytes'], 'y')], {
          contentType: 'application/octet-stream',
        });
        return new BlobOutput(descriptor);
      }
      if (behavior['blobInlineBytes'] !== undefined) {
        const descriptor = await ctx.writeBlob([Buffer.alloc(behavior['blobInlineBytes'], 'z')], {
          contentType: 'application/octet-stream',
        });
        return { $blob: descriptor };
      }
      if (behavior['verdict'] !== undefined) {
        const output = behavior['output'];
        return behavior['verdict'] === 'applied' ? reconcile.applied(output) : behavior['verdict'] === 'not_applied' ? reconcile.notApplied() : reconcile.unknown();
      }
      if (behavior['echo']) return { input, steps: ctx.steps };
      if (behavior['echoInput']) return input;
      if (behavior['returnsText'] !== undefined) return 'x'.repeat(behavior['returnsText']);
      return behavior['returns'] ?? null;
    } finally {
      measurement.active -= 1;
      log.push({
        activity: name,
        kind,
        node: ctx.node,
        attempt: ctx.attempt,
        input,
        taskKind: ctx.taskKind,
        targetNode: ctx.targetNode,
        targetIdempotencyKey: ctx.targetIdempotencyKey,
        idempotencyKey: ctx.idempotencyKey,
        traceparentPresent: ctx.traceparent !== '',
        traceId: ctx.traceparent.split('-')[1] ?? '',
        steps: ctx.steps,
        deadlineAhead: ctx.deadline.getTime() > Date.now(),
        abortedReason: ctx.signal.aborted ? String(ctx.signal.reason) : null,
        outcome,
      });
    }
  };
}

function hostOf(
  def: WorkerDef,
  config: Obj,
  plan: FaultPlan,
  log: Obj[],
  measurement: { active: number; peakInflight: number; deliveries: Obj[] },
): WorkerHost {
  const host = new WorkerHost({
    grpcUrl: config['grpcUrl'], namespace: config['namespace'], token: def['token'] === 'unbound' ? config['unboundWorkerToken'] : config['workerToken'], workerName: def.workerName,
    maxInflight: def['maxInflight'] ?? 4, protocolVersion: def['protocolVersion'], heartbeatIntervalMs: def['heartbeatIntervalMs'],
    transport: def['transport'],
    interceptors: plan.interceptors(), retry: { initialDelayMs: 20, maxDelayMs: 200 },
  });
  for (const [name, activity] of Object.entries(def.activities)) {
    const kind = activity.as ?? 'activity';
    const handler = behaviorHandler(name, kind, activity.behavior ?? {}, log, measurement);
    if (kind === 'compensation') host.registerCompensation(name, handler);
    else if (kind === 'reconciler') host.registerReconciler(name, handler);
    else host.registerActivity(name, handler);
  }
  return host;
}

// ------------------------------------------------------------------------------------------------- run

async function run(scenario: Obj, config: Obj): Promise<Obj> {
  const unique = config['unique'] as string;
  const plan = new FaultPlan((scenario['faults'] ?? []) as Fault[]);
  const apiPath = new URL(config['apiUrl']).pathname.replace(/\/$/, '');
  const clientFor = (token: string): SagatClient =>
    new SagatClient({
      apiUrl: config['apiUrl'], namespace: config['namespace'], token, retry: { initialDelayMs: 20, maxDelayMs: 200 },
      fetch: plan.fetch(apiPath),
    });
  const clients: Record<string, SagatClient> = {
    default: clientFor(config['clientToken']),
    bad: clientFor(config['badToken']),
    other: clientFor(config['otherTenantToken']),
  };
  const logs: Record<string, Obj[]> = {};
  const measurements: Record<string, { active: number; peakInflight: number; deliveries: Obj[] }> = {};
  const hosts: Record<string, { def: WorkerDef; host: WorkerHost | undefined }> = {};
  for (const [key, def] of Object.entries<WorkerDef>(scenario['workers'] ?? {})) {
    logs[key] = [];
    measurements[key] = { active: 0, peakInflight: 0, deliveries: [] };
    hosts[key] = { def, host: undefined };
  }
  const startWorker = async (key: string): Promise<Obj> => {
    const entry = hosts[key];
    if (!entry) throw new Error(`no worker ${key}`);
    entry.host = hostOf(entry.def, config, plan, logs[key]!, measurements[key]!);
    await entry.host.start();
    return { protocolVersion: entry.host.protocolVersion };
  };

  const observations: Obj[] = [];
  const results = new Map<string, Obj>();
  const runOf = (step: Obj): string => {
    const result = results.get(step['run']);
    const id = result?.['runId'];
    if (typeof id !== 'string') throw new Error(`step ${String(step['run'])} has no run`);
    return id;
  };

  for (const [key, entry] of Object.entries(hosts)) {
    if (entry.def['autostart'] !== false) await startWorker(key);
  }

  for (const step of scenario['steps'] as Obj[]) {
    const client = clients[step['token'] ?? 'default']!;
    const observation: Obj = { id: step['id'], op: step['op'] };
    if (process.env['SAGAT_RUNNER_TRACE']) process.stderr.write(`step ${String(step['id'])}\n`);
    try {
      let result: Obj | undefined;
      switch (step['op']) {
        case 'start': {
          const workflowId = config['workflows'][step['workflow']].workflowId as string;
          result = runJson(await client.start(workflowId, step['input'] ?? {}, {
            idempotencyKey: step['idempotencyKey'], correlationId: step['correlationId'],
            traceparent: step['traceparent'], deadlineInMs: step['deadlineInMs'],
          }));
          break;
        }
        case 'startWithBlob': {
          const workflowId = config['workflows'][step['workflow']].workflowId as string;
          const body = Buffer.alloc(step['inputBlobBytes'], 'i');
          const sha256 = createHash('sha256').update(body).digest('hex');
          const descriptor = await client.uploadRunInput([body], { expectedSize: body.length, expectedSha256: sha256 });
          result = runJson(await client.startWithBlob(workflowId, descriptor, {
            idempotencyKey: step['idempotencyKey'], correlationId: step['correlationId'],
            traceparent: step['traceparent'], deadlineInMs: step['deadlineInMs'],
          }));
          break;
        }
        case 'workflowByName': {
          const name = config['workflows'][step['workflow']].name as string;
          result = runJson(await client.start(name, step['input'] ?? {}, { idempotencyKey: step['idempotencyKey'] }));
          break;
        }
        case 'get': result = runJson(await client.get(runOf(step))); break;
        case 'list': {
          const ids: string[] = [];
          if (step['all']) {
            for await (const item of client.listAll({ status: step['status'], limit: step['limit'] })) ids.push(item.runId);
          } else {
            ids.push(...(await client.list({ status: step['status'], limit: step['limit'] })).items.map((r) => r.runId));
          }
          // In the order the server gave (newest first): the SDK keeps it, also across pages.
          result = { items: ids };
          break;
        }
        case 'cancel': {
          const r = await client.cancel(runOf(step), step['reason'], { expectedRunVersion: step['expectedRunVersion'] });
          result = { accepted: r.accepted, runStatus: r.runStatus };
          break;
        }
        case 'operate': {
          const r = await client.operate(runOf(step), step['action'], step['reason'], { stepId: step['stepId'], outcome: step['outcome'], errorCode: step['errorCode'] });
          result = { accepted: r.accepted, runStatus: r.runStatus };
          break;
        }
        case 'decideApproval': {
          const r = await client.decideApproval(step['approvalId'], step['decision'], {
            decisionKey: step['decisionKey'], reasonCode: step['reasonCode'], traceparent: step['traceparent'],
          });
          result = {
            receiptId: r.receiptId, approvalId: r.approvalId, decisionKey: r.decisionKey, decision: r.decision,
            reasonCode: r.reasonCode, actor: r.actor, decidedAt: r.decidedAt.toISOString(),
            outcomeSequence: r.outcomeSequence, replayed: r.replayed,
          };
          break;
        }
        case 'query': {
          const r = await client.query(runOf(step), step['name'], step['args'] ?? null, { traceparent: step['traceparent'] });
          result = { runVersion: r.runVersion, stateRevision: r.stateRevision, result: r.result };
          break;
        }
        case 'update': {
          const r = await client.update(runOf(step), step['name'], step['args'] ?? null, {
            updateKey: step['updateKey'], expectedStateRevision: step['expectedStateRevision'], traceparent: step['traceparent'],
          });
          result = {
            operation: r.operation, updateKey: r.updateKey, status: r.status, result: r.result,
            beforeStateRevision: r.beforeStateRevision, afterStateRevision: r.afterStateRevision,
            runVersion: r.runVersion, replayed: r.replayed,
          };
          break;
        }
        case 'inspectChain': {
          const r = await client.inspectChain(step['namespaceId'], step['chainId'], { traceparent: step['traceparent'] });
          result = { chainId: r.chainId, currentRunId: r.currentRunId, revision: r.revision, cancelRequested: r.cancelRequested,
            finalOutcome: r.finalOutcome, segments: r.segments };
          break;
        }
        case 'signalChain': result = { ...(await client.signalChain(step['namespaceId'], step['chainId'], step['name'], step['payload'], {
          expectedRunId: step['expectedRunId'], idempotencyKey: step['idempotencyKey'], traceparent: step['traceparent'],
        })) }; break;
        case 'cancelChain': await client.cancelChain(step['namespaceId'], step['chainId'], step['reason'], {
          expectedRunId: step['expectedRunId'], idempotencyKey: step['idempotencyKey'], traceparent: step['traceparent'],
        }); result = { accepted: true }; break;
        case 'signal': result = { ...(await client.signal(runOf(step), step['name'], step['payload'] ?? {})) }; break;
        case 'publishEvent': {
          const r = await client.publishEvent({ type: step['type'], data: step['data'] ?? {}, eventId: step['eventId'], correlationKey: step['correlationKey'] });
          result = { eventId: r.eventId, status: r.status };
          break;
        }
        case 'waitForCompletion': result = runJson(await client.waitForCompletion(runOf(step), { timeoutMs: step['timeoutMs'] ?? 60_000, pollIntervalMs: 100 })); break;
        case 'events': {
          const page = await client.events(runOf(step), { limit: 200 });
          result = { events: page.items.map((e) => ({ type: e.raw['eventType'], stepId: e.raw['stepId'] ?? null, attempt: e.raw['attempt'] ?? null })) };
          break;
        }
        case 'sleep': await new Promise((resolve) => setTimeout(resolve, step['ms'])); break;
        case 'control': {
          const body = { action: step['action'], run: step['run'] ? runOf(step) : undefined, node: step['node'] };
          const response = await fetch(`${config['controlUrl']}/control`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
          if (!response.ok) throw new Error(`control ${String(step['action'])} answered ${response.status}`);
          break;
        }
        case 'startWorker': result = await startWorker(step['worker']); break;
        case 'stopWorker': await hosts[step['worker']]?.host?.stop(step['timeoutMs'] ?? 5000); break;
        case 'restartWorker': {
          // The same host object, stopped and started again (SPEC §6.8: the host is restartable).
          const host = hosts[step['worker']]?.host;
          if (!host) throw new Error(`worker ${String(step['worker'])} was never started`);
          await host.stop(step['timeoutMs'] ?? 5000);
          await host.start();
          result = { protocolVersion: host.protocolVersion };
          break;
        }
        case 'probeWorker': {
          const probe = new WorkerHost({ grpcUrl: config['grpcUrl'], namespace: config['namespace'], token: config['workerToken'], workerName: `probe-${unique}`, protocolVersion: step['protocolVersion'], retry: { maxAttempts: 1 } });
          probe.registerActivity('conformance.probe', () => null);
          try {
            await probe.start();
            result = { accepted: probe.protocolVersion };
          } finally {
            await probe.stop(500).catch(() => undefined);
          }
          break;
        }
        case 'awaitJournal': {
          const id = runOf(step);
          const end = Date.now() + (step['timeoutMs'] ?? 20_000);
          for (;;) {
            const page = await client.events(id, { limit: 200 });
            if (page.items.some((e) => e.raw['eventType'] === step['eventType'] && (step['stepId'] === undefined || e.raw['stepId'] === step['stepId']))) break;
            if (Date.now() > end) throw new Error(`the journal of ${id} has no ${String(step['eventType'])}`);
            await new Promise((resolve) => setTimeout(resolve, 25));
          }
          break;
        }
        case 'awaitWorkerLog': {
          const log = logs[step['worker']]!;
          const end = Date.now() + (step['timeoutMs'] ?? 20_000);
          while (log.length < step['count'] && Date.now() < end) await new Promise((resolve) => setTimeout(resolve, 20));
          if (log.length < step['count']) throw new Error(`the worker log has ${log.length} entries, not ${String(step['count'])}`);
          break;
        }
        default: throw new Error(`unknown step op ${String(step['op'])}`);
      }
      observation['ok'] = true;
      if (result !== undefined) {
        observation['result'] = result;
        results.set(step['id'], result);
      }
    } catch (error) {
      observation['ok'] = false;
      observation['error'] = errorJson(error);
    }
    observations.push(observation);
  }

  for (const entry of Object.values(hosts)) await entry.host?.stop(2000).catch(() => undefined);
  const workerMeasurements = Object.fromEntries(Object.entries(measurements).map(([key, value]) => [
    key,
    { peakInflight: value.peakInflight, deliveries: value.deliveries },
  ]));
  return { scenario: scenario['name'], language: 'typescript', sdkVersion: SDK_VERSION, observations, workerLog: logs, workerMeasurements, faultsApplied: plan.applied() };
}

// ----------------------------------------------------------------------------------------------- main

async function main(): Promise<void> {
  const [mode, scenarioPath, configPath, outPath] = process.argv.slice(2);
  if (!mode || !scenarioPath) throw new Error('usage: runner build <scenario> | run <scenario> <config> <out>');
  const raw = read(scenarioPath);
  if (mode === 'build') {
    process.stdout.write(JSON.stringify(build(raw)));
    return;
  }
  if (mode === 'run' && configPath && outPath) {
    const config = read(configPath);
    const out = await run(templated(raw, config['unique'] as string), config);
    writeFileSync(outPath, JSON.stringify(out, null, 2));
    return;
  }
  throw new Error(`unknown mode ${mode}`);
}

let finished = false;
process.on('exit', (code) => {
  // A scenario that waits on something that never comes empties the event loop, and Node then exits
  // with 0: that is a failure, not a pass.
  if (!finished && code === 0) {
    process.stderr.write('the event loop emptied before the scenario finished\n');
    process.exitCode = 1;
  }
});

main().then(
  () => {
    finished = true;
    process.exit(0);
  },
  (error) => {
    process.stderr.write(`${error instanceof Error ? error.stack ?? error.message : String(error)}\n`);
    process.exit(1);
  },
);
