// The conformance runner of the TypeScript SDK (tests/contract/sdk-conformance/README.md). It uses
// only the public API of the SDK: the builders in `build` mode, the client and the worker host in
// `run` mode.
import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import {
  ActivityFailure, BlobOutput, SDK_VERSION, SagatClient, SagatError, WorkerHost, body, flow, reconcile, workflow,
  type ActivityContext, type BatchJob, type Body, type Flow, type Reset, type Run, type Schedule, type ScheduleActionRecord,
  type ScheduleBackfill, type ScheduleDescription, type StateDefinition, type UpdateWithStartReceipt, type WorkflowBuilder,
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
      case 'database': target.database(op['id'], op['options']); break;
      case 'icebergWrite': target.icebergWrite(op['id'], op['options']); break;
      case 'script': target.script(op['id'], op['artifactDigest'], op['options']); break;
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
  if (definition['limits']) builder.limits(definition['limits']);
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
    // Present only for a run in a chain, so the observations of every other run stay as they were.
    ...(run.chain ? { chain: run.chain } : {}),
    // Present only for a run started again from an ended one.
    ...(run.rerunOf ? { rerunOf: run.rerunOf } : {}),
    // Present only for a run started under a business identifier, or held by a start delay.
    ...(run.businessId !== null ? { businessId: run.businessId } : {}),
    ...(run.startDelayedUntil !== null ? { startDelayedUntil: iso(run.startDelayedUntil) } : {}),
    // Present only for a run a schedule started, or a reset made.
    ...(run.schedule !== null
      ? { schedule: { scheduleId: run.schedule.scheduleId, nominalTime: iso(run.schedule.nominalTime), kind: run.schedule.kind, revision: run.schedule.revision } }
      : {}),
    ...(run.resetFrom !== null ? { resetFrom: { runId: run.resetFrom.runId, sequence: run.resetFrom.sequence } } : {}),
  };
}

// The feature 018 results. Instants that depend on when the scenario ran are written as instants (the harness
// compares them as `<time>`); the ones a fixed spec determines are written as epoch milliseconds, so that a wrong
// parse of an instant shows.
const epoch = (date: Date | null): number | null => (date ? date.getTime() : null);

function scheduleJson(s: Schedule): Obj {
  return {
    scheduleId: s.scheduleId, name: s.name, origin: s.origin, state: s.state, revision: s.revision, exhausted: s.exhausted,
    action: s.action, note: s.note, noteBy: s.noteBy, remainingActions: s.remainingActions, nextActionEpochMs: epoch(s.nextActionAt),
    createdAt: iso(s.createdAt), updatedAt: iso(s.updatedAt),
  };
}

function actionJson(a: ScheduleActionRecord): Obj {
  return {
    kind: a.kind, origin: a.origin, revision: a.revision, outcome: a.outcome, reason: a.reason, runId: a.runId, backfillId: a.backfillId,
    nominalTime: iso(a.nominalTime), actualTime: iso(a.actualTime), summaryCount: a.summaryCount,
    summaryLastNominalTime: iso(a.summaryLastNominalTime), decidedAt: iso(a.decidedAt),
  };
}

function descriptionJson(d: ScheduleDescription): Obj {
  return {
    ...scheduleJson(d),
    nextActions: d.nextActions.map((n) => ({ nominalTimeEpochMs: n.nominalTime.getTime(), actualTimeEpochMs: n.actualTime.getTime() })),
    recentActions: d.recentActions.map(actionJson), openRuns: d.openRuns, bufferedCount: d.bufferedCount, counters: d.counters,
  };
}

function backfillJson(b: ScheduleBackfill): Obj {
  return { backfillId: b.backfillId, start: iso(b.start), end: iso(b.end), cursor: iso(b.cursor), state: b.state, overlap: b.overlap, counts: b.counts };
}

function jobJson(j: BatchJob): Obj {
  return {
    batchId: j.batchId, action: j.action, state: j.state, failureReason: j.failureReason, createdBy: j.createdBy, hasPayload: j.hasPayload,
    maxItems: j.maxItems, itemsPerSecond: j.itemsPerSecond, counts: j.counts, createdAt: iso(j.createdAt), startedAt: iso(j.startedAt),
    finishedAt: iso(j.finishedAt), expiresAt: iso(j.expiresAt),
  };
}

function resetJson(r: Reset): Obj {
  return {
    resetId: r.resetId, sourceRunId: r.sourceRunId, sequence: r.sequence, state: r.state, failureReason: r.failureReason,
    newRunId: r.newRunId, reapplyReport: r.reapplyReport, createdAt: iso(r.createdAt),
  };
}

function receiptJson(r: UpdateWithStartReceipt | null): Obj | null {
  return r && {
    operation: r.operation, updateKey: r.updateKey, status: r.status, result: r.result, beforeStateRevision: r.beforeStateRevision,
    afterStateRevision: r.afterStateRevision, runVersion: r.runVersion, replayed: r.replayed,
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
      // `holdUntilLogged`: a scoped instance waits until the worker has logged that many finished attempts,
      // which orders completions by what happened rather than by how long anything took.
      const holdUntil = (behavior['holdUntilLogged'] as number[] | undefined)?.[ctx.index ?? 0] ?? 0;
      while (log.length < holdUntil) {
        if (await abortable(10, ctx.signal)) {
          outcome = 'cancelled';
          return null;
        }
      }
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
        ...(ctx.instance ? {
          instance: {
            instancePath: ctx.instance.instancePath.map((segment) => ({
              scopeNodeId: segment.scopeNodeId,
              iterationIndex: segment.iterationIndex,
            })),
            definitionNodeId: ctx.instance.definitionNodeId,
          },
        } : {}),
        ...(ctx.index !== undefined ? { scopeInput: { item: ctx.item, index: ctx.index } } : {}),
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
    // Members of the tenant with one role each (`operator`, `viewer`, `runner`), the worker identity, and an
    // operator the scenario revokes: the scenarios about who may do what.
    worker: clientFor(config['workerToken']),
    operator: clientFor(config['operatorToken']),
    viewer: clientFor(config['viewerToken']),
    runner: clientFor(config['runnerToken']),
    revoked: clientFor(config['revokedToken']),
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
    if (typeof step['page'] === 'string') {
      // A segment of a page of `listChainRuns`, by position.
      const items = results.get(step['page'])?.['items'];
      const id = Array.isArray(items) ? (items[step['index'] ?? 0] as Obj | undefined)?.['runId'] : undefined;
      if (typeof id !== 'string') throw new Error(`step ${String(step['page'])} has no segment ${String(step['index'])}`);
      return id;
    }
    // A rerun answers `newRunId` and no `runId`: the step then names the run it started.
    const result = results.get(step['run']);
    const id = result?.['runId'] ?? result?.['newRunId'];
    if (typeof id !== 'string') throw new Error(`step ${String(step['run'])} has no run`);
    return id;
  };
  const approvalOf = (step: Obj): string => {
    if (typeof step['approvalId'] === 'string') return step['approvalId'];
    const items = results.get(step['approval'])?.['items'];
    const id = Array.isArray(items) ? (items[0] as Obj | undefined)?.['approvalId'] : undefined;
    if (typeof id !== 'string') throw new Error(`step ${String(step['approval'])} has no approval`);
    return id;
  };
  const chainRef = (value: string): string => {
    if (value === '{{namespaceId}}') return config['namespaceId'] as string;
    const match = /^\{\{(chainId|currentRunId):([^}]+)\}\}$/.exec(value);
    if (!match) return value;
    const field = match[1] === 'chainId' ? 'runId' : 'currentRunId';
    const resolved = results.get(match[2]!)?.[field];
    if (typeof resolved !== 'string') throw new Error(`step ${match[2]} has no ${field}`);
    return resolved;
  };

  // References inside the parameters of a step: a value an earlier step produced (`{{step:<id>:<field>}}`), a workflow
  // id (`{{workflow:<key>}}`), the namespace id, and instants computed when the step runs (`{{now:<offsetMs>}}`, and
  // `{{hour:<offsetHours>}}` for the start of a UTC hour). A string that is exactly one reference becomes its value.
  const refs = (value: any): any => {
    if (typeof value === 'string') {
      if (value === '{{namespaceId}}') return config['namespaceId'];
      let match = /^\{\{workflow:([^}]+)\}\}$/.exec(value);
      if (match) return config['workflows'][match[1]!].workflowId;
      match = /^\{\{step:([^:}]+):([^}]+)\}\}$/.exec(value);
      if (match) {
        const found = results.get(match[1]!)?.[match[2]!];
        if (found === undefined) throw new Error(`step ${match[1]} has no ${match[2]}`);
        return found;
      }
      match = /^\{\{now:(-?\d+)\}\}$/.exec(value);
      if (match) return new Date(Date.now() + Number(match[1])).toISOString();
      match = /^\{\{hour:(-?\d+)\}\}$/.exec(value);
      if (match) {
        const at = new Date();
        at.setUTCMinutes(0, 0, 0);
        at.setUTCHours(at.getUTCHours() + Number(match[1]));
        return at.toISOString();
      }
      return value;
    }
    if (Array.isArray(value)) return value.map(refs);
    if (value !== null && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, refs(v)]));
    return value;
  };
  const namespaceId = config['namespaceId'] as string;
  /** The id of the step whose result holds this run, or `?` for a run no step of the scenario started. */
  const stepOfRun = (runId: string): string => {
    for (const [id, found] of results) if (found['runId'] === runId || found['newRunId'] === runId) return id;
    return '?';
  };
  const stepResult =(id: string, field: string): string => {
    const found = results.get(id)?.[field];
    if (typeof found !== 'string') throw new Error(`step ${id} has no ${field}`);
    return found;
  };
  const awaiting = async (what: string, timeoutMs: number, done: () => Promise<boolean>): Promise<void> => {
    const end = Date.now() + timeoutMs;
    while (!(await done())) {
      if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
  };

  for (const [key, entry] of Object.entries(hosts)) {
    if (entry.def['autostart'] !== false) await startWorker(key);
  }

  for (const given of scenario['steps'] as Obj[]) {
    const client = clients[given['token'] ?? 'default']!;
    let step: Obj = given;
    try {
      step = refs(given) as Obj;
    } catch (error) {
      // A reference to a step that failed is a failure of this step, recorded like any other.
      observations.push({ id: given['id'], op: given['op'], ok: false, error: errorJson(error) });
      continue;
    }
    const observation: Obj = { id: step['id'], op: step['op'] };
    if (process.env['SAGAT_RUNNER_TRACE']) process.stderr.write(`step ${String(step['id'])}\n`);
    try {
      let result: Obj | undefined;
      switch (step['op']) {
        case 'start': {
          const workflowId = config['workflows'][step['workflow']].workflowId as string;
          const started = await client.start(workflowId, step['input'] ?? {}, {
            idempotencyKey: step['idempotencyKey'], correlationId: step['correlationId'],
            traceparent: step['traceparent'], deadlineInMs: step['deadlineInMs'],
            businessId: step['businessId'], businessIdConflictPolicy: step['businessIdConflictPolicy'],
            businessIdReusePolicy: step['businessIdReusePolicy'], startDelayMs: step['startDelayMs'],
          });
          // `started` only when the step asked for a business identifier: the other observations stay as they were.
          result = { ...runJson(started), ...(step['businessId'] !== undefined ? { started: started.started } : {}) };
          break;
        }
        case 'scheduleCreate': result = scheduleJson(await client.schedules.create(namespaceId, step['write'], { idempotencyKey: step['idempotencyKey'] })); break;
        case 'scheduleGet': result = descriptionJson(await client.schedules.get(namespaceId, stepResult(step['schedule'], 'scheduleId'), { nextCount: step['nextCount'], recentCount: step['recentCount'] })); break;
        case 'scheduleList': {
          const names: Obj[] = [];
          const read = (s: Schedule): void => { names.push({ name: s.name, state: s.state, origin: s.origin, revision: s.revision }); };
          let nextCursor: string | null = null;
          if (step['all']) {
            for await (const item of client.schedules.listAll(namespaceId, { ...(step['filter'] ?? {}), limit: step['limit'] })) read(item);
          } else {
            const listed = await client.schedules.list(namespaceId, { ...(step['filter'] ?? {}), limit: step['limit'] });
            listed.items.forEach(read);
            nextCursor = listed.nextCursor;
          }
          result = { items: names, nextCursor: nextCursor === null ? null : '<cursor>' };
          break;
        }
        case 'scheduleUpdate': result = scheduleJson(await client.schedules.update(namespaceId, stepResult(step['schedule'], 'scheduleId'), step['write'], step['expectedRevision'], { idempotencyKey: step['idempotencyKey'] })); break;
        case 'scheduleDelete': await client.schedules.delete(namespaceId, stepResult(step['schedule'], 'scheduleId'), step['expectedRevision'], { idempotencyKey: step['idempotencyKey'] }); result = { deleted: true }; break;
        case 'schedulePause': result = scheduleJson(await client.schedules.pause(namespaceId, stepResult(step['schedule'], 'scheduleId'), step['note'], { idempotencyKey: step['idempotencyKey'] })); break;
        case 'scheduleResume': result = scheduleJson(await client.schedules.resume(namespaceId, stepResult(step['schedule'], 'scheduleId'), step['note'], { idempotencyKey: step['idempotencyKey'] })); break;
        case 'scheduleTrigger': result = actionJson(await client.schedules.trigger(namespaceId, stepResult(step['schedule'], 'scheduleId'), { overlap: step['overlap'], idempotencyKey: step['idempotencyKey'] })); break;
        case 'scheduleBackfill': result = backfillJson(await client.schedules.backfill(namespaceId, stepResult(step['schedule'], 'scheduleId'), step['start'], step['end'], { overlap: step['overlap'], idempotencyKey: step['idempotencyKey'] })); break;
        case 'scheduleCancelBackfill': result = backfillJson(await client.schedules.cancelBackfill(namespaceId, stepResult(step['schedule'], 'scheduleId'), stepResult(step['backfill'], 'backfillId'), { idempotencyKey: step['idempotencyKey'] })); break;
        case 'scheduleBackfills': result = { items: (await client.schedules.listBackfills(namespaceId, stepResult(step['schedule'], 'scheduleId'))).map(backfillJson) }; break;
        case 'scheduleActions': {
          const listed = await client.schedules.listActions(namespaceId, stepResult(step['schedule'], 'scheduleId'), { outcome: step['outcome'], limit: step['limit'] });
          result = { items: listed.items.map(actionJson), nextCursor: listed.nextCursor === null ? null : '<cursor>' };
          break;
        }
        case 'awaitSchedule': {
          // The scheduler decides the times of a backfill: wait for its counters, never for a delay.
          const id = stepResult(step['schedule'], 'scheduleId');
          await awaiting(`schedule counter ${String(step['counter'])}`, step['timeoutMs'] ?? 30_000, async () =>
            ((await client.schedules.get(namespaceId, id, { nextCount: 0, recentCount: 0 })).counters[step['counter']] ?? 0) >= (step['atLeast'] ?? 1));
          break;
        }
        case 'awaitBackfill': {
          // A backfill counts the times it found already acted on in its own counts, not in the schedule's counters.
          const schedule = stepResult(step['schedule'], 'scheduleId');
          const id = stepResult(step['backfill'], 'backfillId');
          await awaiting(`backfill state ${String(step['state'])}`, step['timeoutMs'] ?? 30_000, async () =>
            (await client.schedules.listBackfills(namespaceId, schedule)).some((b) => b.backfillId === id && b.state === step['state']));
          break;
        }
        case 'businessIdGet': {
          const b = await client.businessIds.get(namespaceId, step['businessId']);
          result = {
            businessId: b.businessId, executionId: b.executionId, currentRunId: b.currentRunId, open: b.open, lastStatus: b.lastStatus,
            workflowId: b.workflowId, boundAt: iso(b.boundAt), reusableAfter: iso(b.reusableAfter),
          };
          break;
        }
        case 'signalWithStart': {
          const r = await client.signalWithStart(step['workflow'] ? config['workflows'][step['workflow']].workflowId : step['workflowId'], step['businessId'], step['input'] ?? {}, step['signal']['name'], step['signal']['payload'] ?? {}, {
            idempotencyKey: step['idempotencyKey'], businessIdReusePolicy: step['businessIdReusePolicy'],
          });
          result = { runId: r.runId, started: r.started, deliveredTo: r.deliveredTo };
          break;
        }
        case 'updateWithStart': {
          const u = step['update'];
          const r = await client.updateWithStart(config['workflows'][step['workflow']].workflowId, step['businessId'], step['input'] ?? {}, u['name'], u['updateKey'], u['args'] ?? null, {
            idempotencyKey: step['idempotencyKey'], businessIdConflictPolicy: step['businessIdConflictPolicy'],
            businessIdReusePolicy: step['businessIdReusePolicy'], startDelayMs: step['startDelayMs'], expectedStateRevision: u['expectedStateRevision'],
          });
          result = { runId: r.runId, started: r.started, update: { outcome: r.update.outcome, reason: r.update.reason, receipt: receiptJson(r.update.receipt) } };
          break;
        }
        case 'batchPreview': {
          const p = await client.batch.preview(namespaceId, step['selector'], step['maxItems']);
          // The sample is a set of runs in the order the server chose; the scenario compares which of its own runs it holds.
          result = { matched: p.matched, capped: p.capped, sampleSize: p.sample.length, sampleSteps: p.sample.map(stepOfRun).sort() };
          break;
        }
        case 'batchCreate': {
          // What does not change while the job runs (its state and counters do): the job as it was accepted.
          const j = await client.batch.create(namespaceId, step['request'], { idempotencyKey: step['idempotencyKey'] });
          result = { batchId: j.batchId, action: j.action, createdBy: j.createdBy, hasPayload: j.hasPayload, maxItems: j.maxItems, itemsPerSecond: j.itemsPerSecond };
          break;
        }
        case 'batchGet': result = jobJson(await client.batch.get(namespaceId, stepResult(step['batch'], 'batchId'))); break;
        case 'batchList': {
          const listed = await client.batch.list(namespaceId, { state: step['state'], limit: step['limit'] });
          result = { items: listed.items.map((j) => ({ action: j.action, state: j.state, counts: j.counts })), nextCursor: listed.nextCursor === null ? null : '<cursor>' };
          break;
        }
        case 'batchCancel': result = jobJson(await client.batch.cancel(namespaceId, stepResult(step['batch'], 'batchId'), step['reason'], { idempotencyKey: step['idempotencyKey'] })); break;
        case 'batchItems': {
          const listed = await client.batch.listItems(namespaceId, stepResult(step['batch'], 'batchId'), { state: step['state'], limit: step['limit'] });
          // The order of the items is the server's; which of the scenario's runs sits at which ordinal is not part of the
          // contract, so the items are named by the step that started the run and sorted by it.
          result = {
            items: listed.items.map((i) => ({ runStep: stepOfRun(i.runId), state: i.state, reason: i.reason, errorCode: i.errorCode, attempts: i.attempts, newRunId: i.newRunId }))
              .sort((a, b) => a.runStep.localeCompare(b.runStep)),
            nextCursor: listed.nextCursor === null ? null : '<cursor>',
          };
          break;
        }
        case 'awaitBatch': {
          const id = stepResult(step['batch'], 'batchId');
          await awaiting(`batch state ${String(step['state'])}`, step['timeoutMs'] ?? 30_000, async () => (await client.batch.get(namespaceId, id)).state === step['state']);
          break;
        }
        case 'resetPoints': {
          const listed = await client.resetPoints(runOf(step), { limit: step['limit'] });
          result = { items: listed.items.map((p) => ({ sequence: p.sequence, kind: p.kind, node: p.node, occurredAt: iso(p.occurredAt) })), nextCursor: listed.nextCursor === null ? null : '<cursor>' };
          break;
        }
        case 'resetPreview': {
          const plan = await client.previewReset(runOf(step), step['sequence'], step['reapply']);
          result = { sequence: plan.sequence, restored: plan.restored, reexecuted: plan.reexecuted, reapply: plan.reapply, requiresAcknowledgement: plan.requiresAcknowledgement, prefixEvents: plan.prefixEvents };
          break;
        }
        case 'reset': result = resetJson(await client.reset(runOf(step), {
          sequence: step['sequence'], reason: step['reason'], reapply: step['reapply'], sourcePolicy: step['sourcePolicy'],
          businessId: step['businessId'], acknowledgeReexecution: step['acknowledgeReexecution'],
        }, { idempotencyKey: step['idempotencyKey'] })); break;
        case 'resets': result = { items: (await client.listResets(runOf(step))).map(resetJson) }; break;
        case 'getReset': result = resetJson(await client.getReset(runOf(step), stepResult(step['reset'], 'resetId'))); break;
        case 'startWithBlob': {
          const workflowId = config['workflows'][step['workflow']].workflowId as string;
          const body = typeof step['inputBlobText'] === 'string' ? Buffer.from(step['inputBlobText'], 'utf8') : Buffer.alloc(step['inputBlobBytes'], 'i');
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
            // `waitForCount`: the runs of a chain are made by the scheduler, so a scenario that needs a hand-off
            // to have happened waits for it by the count of runs in a status, as inspectChain waits for a head.
            for (let attempt = 0; attempt < 100 && ids.length < (step['waitForCount'] ?? 0); attempt++) {
              await new Promise((resolve) => setTimeout(resolve, 50));
              ids.length = 0;
              ids.push(...(await client.list({ status: step['status'], limit: step['limit'] })).items.map((r) => r.runId));
            }
          }
          // In the order the server gave (newest first): the SDK keeps it, also across pages. `countOnly` answers
          // the number alone, for runs whose ids are derived from their parent's and so sort anywhere.
          result = step['countOnly'] ? { count: ids.length } : { items: ids };
          break;
        }
        case 'listApprovals': {
          let page = await client.listApprovals(runOf(step), { cursor: step['cursor'], limit: step['limit'], traceparent: step['traceparent'] });
          for (let attempt = 0; step['waitForItem'] && page.items.length === 0 && attempt < 100; attempt++) {
            await new Promise((resolve) => setTimeout(resolve, 50));
            page = await client.listApprovals(runOf(step), { cursor: step['cursor'], limit: step['limit'], traceparent: step['traceparent'] });
          }
          result = { items: page.items.map((item) => ({
            approvalId: item.approvalId, runId: item.runId, nodeId: item.nodeId, instancePath: item.instancePath,
            status: item.status, deadline: item.deadline.toISOString(), settledAt: item.settledAt?.toISOString() ?? null,
          })), nextCursor: page.nextCursor };
          break;
        }
        case 'cancel': {
          const r = await client.cancel(runOf(step), step['reason'], { expectedRunVersion: step['expectedRunVersion'] });
          result = { accepted: r.accepted, runStatus: r.runStatus };
          break;
        }
        case 'operate': {
          const r = await client.operate(runOf(step), step['action'], step['reason'], { stepId: step['stepId'], outcome: step['outcome'], errorCode: step['errorCode'] });
          // `newRunId` only for the action that starts a run, so the other operations observe as they did.
          result = { accepted: r.accepted, runStatus: r.runStatus, ...(r.newRunId ? { newRunId: r.newRunId } : {}) };
          break;
        }
        case 'decideApproval': {
          const r = await client.decideApproval(approvalOf(step), step['decision'], {
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
        case 'queryChain': {
          const r = await client.queryChain(
            chainRef(step['namespaceId']), chainRef(step['chainId']), step['name'], step['args'] ?? null,
            { expectedRunId: step['expectedRunId'] ? chainRef(step['expectedRunId']) : undefined, traceparent: step['traceparent'] },
          );
          result = { runId: r.runId, runVersion: r.runVersion, stateRevision: r.stateRevision, result: r.result };
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
        case 'updateChain': {
          const r = await client.updateChain(
            chainRef(step['namespaceId']), chainRef(step['chainId']), step['name'], step['args'] ?? null,
            {
              updateKey: step['updateKey'],
              expectedRunId: step['expectedRunId'] ? chainRef(step['expectedRunId']) : undefined,
              expectedStateRevision: step['expectedStateRevision'], traceparent: step['traceparent'],
            },
          );
          result = {
            runId: r.runId, operation: r.operation, updateKey: r.updateKey, status: r.status, result: r.result,
            beforeStateRevision: r.beforeStateRevision, afterStateRevision: r.afterStateRevision,
            runVersion: r.runVersion, replayed: r.replayed,
          };
          break;
        }
        case 'listChainRuns': {
          const named = /^\{\{nextCursor:([^}]+)\}\}$/.exec(String(step['cursor'] ?? ''));
          const cursor = named ? (results.get(named[1]!)?.['nextCursor'] as string | undefined) : (step['cursor'] as string | undefined);
          if (named && typeof cursor !== 'string') throw new Error(`step ${named[1]} has no next page`);
          const page = await client.listChainRuns(chainRef(step['namespaceId']), chainRef(step['chainId']), {
            cursor, limit: step['limit'], traceparent: step['traceparent'],
          });
          result = {
            items: page.items.map((item) => ({
              index: item.index, runId: item.runId, status: item.status, completionKind: item.completionKind,
              predecessorRunId: item.predecessorRunId, successorRunId: item.successorRunId,
              createdAt: iso(item.createdAt), finishedAt: iso(item.finishedAt),
            })),
            nextCursor: page.nextCursor,
          };
          break;
        }
        case 'inspectChain': {
          let r = await client.inspectChain(chainRef(step['namespaceId']), chainRef(step['chainId']), { traceparent: step['traceparent'] });
          for (let attempt = 0; step['waitForHeadChange'] && r.currentRunId === chainRef(step['waitForHeadChange']) && attempt < 100; attempt++) {
            await new Promise((resolve) => setTimeout(resolve, 50));
            r = await client.inspectChain(chainRef(step['namespaceId']), chainRef(step['chainId']), { traceparent: step['traceparent'] });
          }
          result = { chainId: r.chainId, currentRunId: r.currentRunId, revision: r.revision, cancelRequested: r.cancelRequested,
            finalOutcome: r.finalOutcome, segments: r.segments };
          break;
        }
        case 'signalChain': result = { ...(await client.signalChain(chainRef(step['namespaceId']), chainRef(step['chainId']), step['name'], step['payload'], {
          expectedRunId: chainRef(step['expectedRunId']), idempotencyKey: step['idempotencyKey'], traceparent: step['traceparent'],
        })) }; break;
        case 'cancelChain': await client.cancelChain(chainRef(step['namespaceId']), chainRef(step['chainId']), step['reason'], {
          expectedRunId: chainRef(step['expectedRunId']), idempotencyKey: step['idempotencyKey'], traceparent: step['traceparent'],
        }); result = { accepted: true }; break;
        case 'signal': result = { ...(await client.signal(runOf(step), step['name'], step['payload'] ?? {})) }; break;
        case 'publishEvent': {
          const r = await client.publishEvent({ type: step['type'], data: step['data'] ?? {}, eventId: step['eventId'], correlationKey: step['correlationKey'] });
          result = { eventId: r.eventId, status: r.status };
          break;
        }
        case 'waitForCompletion': result = runJson(await client.waitForCompletion(runOf(step), { timeoutMs: step['timeoutMs'] ?? 60_000, pollIntervalMs: 100 })); break;
        case 'waitForChainCompletion': {
          const r = await client.waitForChainCompletion(chainRef(step['namespaceId']), chainRef(step['chainId']),
            { timeoutMs: step['timeoutMs'] ?? 60_000, pollIntervalMs: 100 });
          result = { chainId: r.chainId, currentRunId: r.currentRunId, revision: r.revision, cancelRequested: r.cancelRequested,
            finalOutcome: r.finalOutcome, segments: r.segments };
          break;
        }
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
    // The harness reads a pipe: exiting before its write completes truncates larger IRs.
    await new Promise<void>((resolve, reject) => {
      process.stdout.write(JSON.stringify(build(raw)), (error) => error ? reject(error) : resolve());
    });
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
