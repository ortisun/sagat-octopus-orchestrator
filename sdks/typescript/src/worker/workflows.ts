/*
 * The worker side of workflow tasks (design 044, Technical Plan 4 and 13.1; Decision 044-319 of
 * `specs/044-workflow-as-code/research.md`): a `WorkerHost` given workflow bundles hands each `WORKFLOW` lease here. The
 * history is read from the lease's first page and `ReadHistory`, a cached run is reused only at the lease's window start
 * and any other is replayed in a new context, the live activation runs on the workflow thread, and its commands or its
 * failure are reported under the lease with a key that is the same on every retry. The host loads this module only when
 * it serves workflow code, so an activity-only host keeps Node 20.
 */
import { createHash } from 'node:crypto';
import { SagatError, isRetriable } from '../errors.js';
import { backoffDelayMs, sleep, type RetryPolicy } from '../retry.js';
import type { Logger } from '../config.js';
import { DEFAULT_CACHE_SIZE, WorkflowRunCache } from '../workflows/cache.js';
import { hex } from '../workflows/chain.js';
import type { HistoryEvent, WorkflowTaskFailure } from '../workflows/context.js';
import { ActivationCutter, HistoryOrderError, type ReplayActivation } from '../workflows/history.js';
import { checkWorkflowRuntime } from '../workflows/preconditions.js';
import {
  HistoryValuesExceeded, MAX_HISTORY_EVENTS, MalformedHistory, blobRefsOf, blobValueOf, commandToWire, eventFromWire, failureToWire,
  type HistoryBlobRef, type ValueCarrier,
} from '../workflows/protocol.js';
import { WorkflowThread, WorkflowThreadError, type LoadedBundle, type RunHandle } from '../workflows/threaded.js';
import { rfc8785 } from '../workflows/context.js';
import type { Completion, PreparedCommand } from '../workflows/run.js';
import type { TaskLease, WireCommand, WireHistoryEvent, WorkerRpc, WorkflowTaskLease, WorkflowTypeOffer } from './grpc.js';

/** The protocol minor of workflow tasks (`sdks/SPEC.md` section 9, Architecture decision 0109). */
export const WORKFLOW_PROTOCOL_MINOR = 10;
/** At most this many (workflow type, code digest) pairs per session (`WorkerCapabilities.workflow_types`). */
export const MAX_WORKFLOW_OFFERS = 64;
/** The reserved step of a run's workflow tasks: blobs are read and written under it. */
const WORKFLOW_STEP = '~workflow';
const REPORT_ATTEMPTS = 5;

/** One code version a host serves: the pair it registers and the bundle text whose digest it is. */
export interface OfferedCode { workflowType: string; codeDigest: string; bundleCode: string; filename?: string }

/** What became of one workflow lease (the `workflow.task` event of the host). */
export type WorkflowTaskOutcome =
  | 'ACCEPTED' | 'ACCEPTED_DEFERRED' | 'REVIEW' | 'FAILED' | 'STALE' | 'DROPPED' | 'LOST' | 'UNREPORTED';

export interface WorkflowTaskEvent {
  type: 'workflow.task';
  taskId: string;
  runId: string;
  outcome: WorkflowTaskOutcome;
  /** The server's cause for REVIEW and ACCEPTED_DEFERRED, the SDK's for FAILED. */
  cause?: string;
  /** The run's context was reused from the cache (no replay). */
  cacheHit: boolean;
  durationMs: number;
}

export interface WorkflowTasksOptions {
  offers: OfferedCode[];
  rpc: () => WorkerRpc;
  log: Logger;
  emit: (event: WorkflowTaskEvent) => void;
  retry: RetryPolicy;
  cacheSize?: number;
  activationDeadlineMs?: number;
}

/** One lease's fence, its channel and the expiry its history pages extend. */
interface LeaseScope { lease: TaskLease; task: WorkflowTaskLease; rpc: WorkerRpc; expiresAt: number }

/** The blobs of a lease's history: those named so far, their bytes in all, and the values read. */
interface HistoryBlobs { refs: Map<string, HistoryBlobRef>; total: number; values: Map<string, unknown> }

/** A replayed activation that differs: the lease ends with this failure. */
class ReplayFailed extends Error {
  constructor(readonly failure: WorkflowTaskFailure) { super(failure.cause); this.name = 'ReplayFailed'; }
}

const offerKey = (workflowType: string, codeDigest: string): string => `${workflowType}\n${codeDigest}`;

/**
 * What a cached context was made under: the code digest, the state bound and the gated entries its lease carried. A
 * lease that carries any other reuses no context (Decision 044-324): a republished envelope changes the last two.
 */
const contextIdentity = (task: WorkflowTaskLease): string => JSON.stringify([task.code_digest, task.state_max_bytes, [...task.gated_entries].sort()]);

/** The workflow tasks of one host: its offers, its workflow thread and bundles, and its cache of runs. */
export class WorkflowTasks {
  readonly #options: WorkflowTasksOptions;
  readonly #offers = new Map<string, OfferedCode>();
  readonly #cache: WorkflowRunCache;
  #thread: WorkflowThread | undefined;
  #bundles = new Map<string, Promise<LoadedBundle>>();

  constructor(options: WorkflowTasksOptions) {
    this.#options = options;
    if (options.offers.length === 0) throw new SagatError({ code: 'VALIDATION_FAILED', message: 'a workflow host serves at least one workflow type' });
    if (options.offers.length > MAX_WORKFLOW_OFFERS) {
      throw new SagatError({ code: 'VALIDATION_FAILED', message: `a session serves at most ${MAX_WORKFLOW_OFFERS} (workflow type, code digest) pairs` });
    }
    for (const offer of options.offers) {
      if (!/^sha256:[0-9a-f]{64}$/.test(offer.codeDigest)) throw new SagatError({ code: 'VALIDATION_FAILED', message: `the code digest of ${offer.workflowType} is not sha256:<64 hexadecimal characters>` });
      const key = offerKey(offer.workflowType, offer.codeDigest);
      if (this.#offers.has(key)) throw new SagatError({ code: 'VALIDATION_FAILED', message: `the pair (${offer.workflowType}, ${offer.codeDigest}) is offered twice` });
      this.#offers.set(key, offer);
    }
    this.#cache = new WorkflowRunCache(options.cacheSize ?? DEFAULT_CACHE_SIZE);
  }

  /** The pairs the session registers. */
  get offers(): WorkflowTypeOffer[] {
    return [...this.#offers.values()].map((offer) => ({ workflow_type: offer.workflowType, code_digest: offer.codeDigest }));
  }

  /** Checks the runtime's precondition and loads every bundle, refusing one that does not export its workflow type. */
  async load(): Promise<void> {
    checkWorkflowRuntime();
    for (const offer of this.#offers.values()) {
      const bundle = await this.#bundle(offer);
      if (!bundle.workflowTypes.includes(offer.workflowType)) {
        throw new SagatError({ code: 'VALIDATION_FAILED', message: `the bundle of ${offer.codeDigest} exports no workflow ${offer.workflowType}` });
      }
    }
  }

  async close(): Promise<void> {
    const thread = this.#thread;
    this.#thread = undefined;
    this.#bundles = new Map();
    if (thread) await thread.terminate();
  }

  #threadNow(): WorkflowThread {
    if (!this.#thread || this.#thread.dead) {
      // A thread that ended took its runs and bundles with it (Decision 044-319): the next lease starts a new one.
      this.#thread = new WorkflowThread();
      this.#bundles = new Map();
    }
    return this.#thread;
  }

  #bundle(offer: OfferedCode): Promise<LoadedBundle> {
    const thread = this.#threadNow();
    const key = offerKey(offer.workflowType, offer.codeDigest);
    let loaded = this.#bundles.get(key);
    if (!loaded) {
      loaded = thread.loadBundle(offer.bundleCode, offer.filename);
      this.#bundles.set(key, loaded);
      loaded.catch(() => { if (this.#bundles.get(key) === loaded) this.#bundles.delete(key); });
    }
    return loaded;
  }

  /** Serves one lease of the kind WORKFLOW; never rejects. */
  async serve(lease: TaskLease): Promise<void> {
    const started = Date.now();
    let cacheHit = false;
    const emit = (outcome: WorkflowTaskOutcome, cause?: string): void => {
      const event: WorkflowTaskEvent = { type: 'workflow.task', taskId: lease.task_id, runId: lease.run_id, outcome, cacheHit, durationMs: Date.now() - started };
      if (cause !== undefined) event.cause = cause;
      this.#options.emit(event);
    };
    const task = lease.workflow_task ?? undefined;
    const offer = task ? this.#offers.get(offerKey(task.workflow_type, task.code_digest)) : undefined;
    if (lease.task_kind !== 'WORKFLOW' || !task || !offer || task.run_id !== lease.run_id || task.lease_token !== lease.lease_token
      || task.attempt !== lease.attempt || task.generation !== lease.generation || !(task.started_at_us > 0)
      || !(task.seed instanceof Uint8Array) || task.seed.length !== 32) {
      // No code of this host may answer it: it lapses, and the server leases it again (Decision 044-319).
      this.#options.log.warn('a workflow lease this host does not serve is left to lapse', { task: lease.task_id, kind: lease.task_kind });
      emit('DROPPED');
      return;
    }
    // One channel for the whole lease: a report still on its way after `stop` never opens another one.
    const scope: LeaseScope = { lease, task, rpc: this.#options.rpc(), expiresAt: task.expires_at_epoch_ms || lease.expires_at_epoch_ms };
    let run: RunHandle | undefined = this.#cache.take(lease.run_id, task.window_start_seq, contextIdentity(task));
    cacheHit = run !== undefined;
    const thread = this.#threadNow();
    try {
      // A cold worker replays each page's completed activations as the page arrives (Technical Plan 4.2): the lease's
      // time counts from the last page delivered.
      const cutter = run ? undefined : new ActivationCutter();
      if (!run) run = await this.#newRun(task, offer);
      const replaying = run;
      const history: HistoryEvent[] = [];
      const blobs: HistoryBlobs = { refs: new Map(), total: 0, values: new Map() };
      await this.#readHistory(scope, cacheHit ? task.window_start_seq + 1 : 1, async (page) => {
        for (const ref of blobRefsOf(page, blobs)) blobs.values.set(ref.blobId, await this.#readBlob(scope, ref));
        const events = page.map((event) => eventFromWire(event, blobs.values));
        history.push(...events);
        if (cutter) for (const activation of cutter.push(events)) await this.#replayOne(replaying, activation);
      });
      if (cutter) {
        for (const activation of cutter.flush().activations) await this.#replayOne(replaying, activation);
        // The history and the lease disagree on where the last completed activation started.
        if (replaying.windowStart !== task.window_start_seq) throw new ReplayFailed({ cause: 'NONDETERMINISM_RISK' });
      }
      const startedEvent = lastStarted(history);
      if (!startedEvent || startedEvent.atUs !== task.started_at_us) {
        throw new MalformedHistory('the history does not end at the workflow_task.started of the lease');
      }
      const answer = await run.activate(history, startedEvent);
      this.#relayLogs(lease, answer.logs);
      if (answer.kind === 'failure') {
        run.dispose();
        run = undefined;
        emit(...await this.#fail(scope, answer.failure));
        return;
      }
      const reported = await this.#complete(scope, answer);
      if (reported.outcome === 'ACCEPTED' || reported.outcome === 'ACCEPTED_DEFERRED') {
        // The server's state moved whatever happens here: a context that cannot take the answer is only dropped.
        const accepted = run;
        run = undefined;
        try {
          await accepted.commit(reported.outcome);
          this.#cache.put(lease.run_id, accepted, contextIdentity(task));
        } catch {
          accepted.dispose();
        }
      }
      emit(reported.outcome, reported.cause);
    } catch (error) {
      if (error instanceof ReplayFailed) {
        emit(...await this.#fail(scope, error.failure));
      } else if (error instanceof HistoryValuesExceeded) {
        // Counted: the run goes under review after its fifth failure, never by allocating what it names.
        emit(...await this.#fail(scope, { cause: 'WORKFLOW_TASK_FAILED', exceptionType: 'HistoryValuesExceeded' }));
      } else if (error instanceof MalformedHistory || error instanceof HistoryOrderError) {
        // A history this SDK cannot read cannot be replayed with certainty.
        emit(...await this.#fail(scope, { cause: 'NONDETERMINISM_RISK' }));
      } else if (error instanceof SagatError && error.code === 'STALE_ATTEMPT') {
        emit('STALE');
      } else if (error instanceof WorkflowThreadError && thread.dead) {
        this.#options.log.error('the workflow thread ended; the lease lapses and a new thread serves the next one', { task: lease.task_id });
        emit('LOST');
      } else if (error instanceof WorkflowThreadError && error.name === 'ActivationDeadlineExceeded') {
        // A context whose bundle did not evaluate within the activation deadline (the thread is alive).
        emit(...await this.#fail(scope, { cause: 'ACTIVATION_DEADLINE' }));
      } else {
        this.#options.log.error('a workflow task could not be served; its lease lapses', {
          task: lease.task_id, code: error instanceof SagatError ? error.code : error instanceof Error ? error.name : 'unknown',
        });
        emit('UNREPORTED');
      }
    } finally {
      run?.dispose();
    }
  }

  /** A new context of the lease's code, before any activation. */
  async #newRun(task: WorkflowTaskLease, offer: OfferedCode): Promise<RunHandle> {
    const bundle = await this.#bundle(offer);
    const init = {
      workflowType: task.workflow_type, seed: hex(task.seed), stateMaxBytes: task.state_max_bytes, gatedEntries: [...task.gated_entries],
    };
    const deadline = this.#options.activationDeadlineMs;
    return await this.#threadNow().createRun(bundle, init, deadline === undefined ? {} : { activationDeadlineMs: deadline });
  }

  /** Replays one completed activation; a difference ends the lease with its failure. */
  async #replayOne(run: RunHandle, activation: ReplayActivation): Promise<void> {
    const step = await run.replay(activation);
    if (step.kind === 'replayed') {
      this.#options.log.debug('replayed an activation', { through: activation.started.seq });
      return;
    }
    // A replay that differs after reading a withheld value may differ because of that value (Decision 044-319).
    throw new ReplayFailed(step.kind === 'failure' ? step.failure : { cause: 'NONDETERMINISM_RISK', commandNumber: step.commandNumber });
  }

  /**
   * The visible history from `from`, page by page (the first page, then ReadHistory until the last page). A retriable
   * error resumes from the next sequence, at most five times and within the lease's expiry (Decision 044-325).
   */
  async #readHistory(scope: LeaseScope, from: number, onPage: (events: WireHistoryEvent[]) => Promise<void>): Promise<void> {
    const { lease, task } = scope;
    // A history over the largest a function is asked to replay belongs to no valid lease (Decision 044-324).
    if (!Number.isSafeInteger(task.visible_count) || task.visible_count > MAX_HISTORY_EVENTS) {
      throw new MalformedHistory(`a history of ${String(task.visible_count)} events is over ${MAX_HISTORY_EVENTS}`);
    }
    let last = 0;
    let read = 0;
    const take = (events: WireHistoryEvent[]): WireHistoryEvent[] => {
      const page: WireHistoryEvent[] = [];
      for (const event of events) {
        if (!(event.seq > last)) throw new MalformedHistory(`history event ${String(event.seq)} is not after ${last}`);
        if (++read > MAX_HISTORY_EVENTS) throw new MalformedHistory(`the history is over ${MAX_HISTORY_EVENTS} events`);
        last = event.seq;
        if (event.seq >= from) page.push(event);
      }
      return page;
    };
    await onPage(take(task.first_page));
    if (task.first_page.length >= task.visible_count) return;
    last = Math.max(last, from - 1);
    for (let attempt = 0; ; attempt++) {
      try {
        for await (const page of scope.rpc.readHistory({
          task_id: lease.task_id, run_id: lease.run_id, attempt: lease.attempt, generation: lease.generation, lease_token: lease.lease_token,
          read_token: '', from_seq: last + 1,
        }, task.traceparent || undefined)) {
          if (page.lease_expires_at_epoch_ms > 0) scope.expiresAt = page.lease_expires_at_epoch_ms;
          await onPage(take(page.events));
          if (page.last_page) return;
        }
        // A stream that ended without its last page lost the rest: it is read again, never answered partial.
        throw new SagatError({ code: 'TRANSPORT', message: 'the history stream ended before its last page' });
      } catch (error) {
        await this.#beforeRetry(scope, error, attempt);
      }
    }
  }

  /** Throws the error unless it is retriable, a retry is left and the lease outlives the wait; otherwise waits. */
  async #beforeRetry(scope: LeaseScope, error: unknown, attempt: number): Promise<void> {
    if (!(error instanceof SagatError) || error.code === 'STALE_ATTEMPT' || !isRetriable(error) || attempt + 1 >= REPORT_ATTEMPTS) throw error;
    const delay = Math.max(backoffDelayMs(this.#options.retry, attempt), error.retryAfterMs ?? 0);
    if (Date.now() + delay >= scope.expiresAt) throw error;
    await sleep(delay);
  }

  async #readBlob(scope: LeaseScope, ref: { blobId: string; size: number; sha256: string }): Promise<unknown> {
    for (let attempt = 0; ; attempt++) {
      try {
        return await this.#readBlobOnce(scope, ref);
      } catch (error) {
        await this.#beforeRetry(scope, error, attempt);
      }
    }
  }

  async #readBlobOnce(scope: LeaseScope, ref: { blobId: string; size: number; sha256: string }): Promise<unknown> {
    const { lease } = scope;
    const hash = createHash('sha256');
    const chunks: Buffer[] = [];
    let size = 0;
    let header = false;
    for await (const part of scope.rpc.readBlob({
      task_id: lease.task_id, run_id: lease.run_id, step_id: WORKFLOW_STEP, attempt: lease.attempt, generation: lease.generation,
      lease_token: lease.lease_token, blob_id: ref.blobId,
    }, lease.traceparent || undefined)) {
      if (part.header) {
        if (header || part.header.blob_id !== ref.blobId || part.header.size !== ref.size || part.header.sha256 !== ref.sha256) {
          throw new MalformedHistory('a history blob stream does not match its reference');
        }
        header = true;
        continue;
      }
      if (!part.chunk || !header) throw new MalformedHistory('a history blob stream is malformed');
      size += part.chunk.length;
      if (size > ref.size) throw new MalformedHistory('a history blob is larger than its reference');
      hash.update(part.chunk);
      chunks.push(Buffer.from(part.chunk));
    }
    if (size !== ref.size || hash.digest('hex') !== ref.sha256) throw new MalformedHistory('a history blob does not match its reference');
    try {
      return JSON.parse(Buffer.concat(chunks).toString('utf8')) as unknown;
    } catch {
      throw new MalformedHistory('a history blob is not JSON');
    }
  }

  async #writeBlob(scope: LeaseScope, value: unknown): Promise<string> {
    const { lease } = scope;
    const bytes = Buffer.from(rfc8785(value), 'utf8');
    const sha256 = createHash('sha256').update(bytes).digest('hex');
    async function* parts(): AsyncIterable<{ header?: Record<string, unknown>; chunk?: Buffer }> {
      yield { header: {
        task_id: lease.task_id, run_id: lease.run_id, step_id: WORKFLOW_STEP, attempt: lease.attempt, generation: lease.generation,
        lease_token: lease.lease_token, content_type: 'application/json', expected_size: bytes.length, expected_sha256: sha256,
      } };
      for (let offset = 0; offset < bytes.length; offset += 256 * 1024) yield { chunk: bytes.subarray(offset, offset + 256 * 1024) };
    }
    const response = await scope.rpc.writeBlob(parts() as AsyncIterable<never>, lease.traceparent || undefined);
    if (response.size !== bytes.length || response.sha256 !== sha256 || response.blob_id === '') {
      throw new SagatError({ code: 'PROTOCOL', message: 'the command value blob does not match the bytes uploaded' });
    }
    return response.blob_id;
  }

  async #complete(scope: LeaseScope, completion: Completion): Promise<{ outcome: WorkflowTaskOutcome; cause?: string }> {
    const { lease, task } = scope;
    const commands: WireCommand[] = [];
    for (const command of completion.commands) commands.push(commandToWire(command, command.commandNumber!, await this.#carrier(scope, command, false)));
    const request = {
      task_id: lease.task_id, run_id: lease.run_id, attempt: lease.attempt, generation: lease.generation, lease_token: lease.lease_token,
      idempotency_key: `workflow-complete-${lease.task_id}-${lease.generation}`,
      applied_through_seq: completion.appliedThroughSeq, replayed_chain: Buffer.from(completion.replayedChain), commands, traceparent: task.traceparent,
      ...(completion.deferredCommand ? { deferred_command: commandToWire(completion.deferredCommand, 0, await this.#carrier(scope, completion.deferredCommand, true)) } : {}),
    };
    const answer = await this.#report(() => scope.rpc.completeWorkflowTask(request, task.traceparent || undefined));
    if (answer === 'stale') return { outcome: 'STALE' };
    const cause = answer.cause && answer.cause !== 'WORKFLOW_TASK_FAILURE_CAUSE_UNSPECIFIED' ? answer.cause : undefined;
    switch (answer.outcome) {
      case 'WORKFLOW_TASK_ACCEPTED': return { outcome: 'ACCEPTED' };
      case 'WORKFLOW_TASK_ACCEPTED_DEFERRED': return cause === undefined ? { outcome: 'ACCEPTED_DEFERRED' } : { outcome: 'ACCEPTED_DEFERRED', cause };
      case 'WORKFLOW_TASK_REVIEW': return cause === undefined ? { outcome: 'REVIEW' } : { outcome: 'REVIEW', cause };
      default:
        this.#options.log.error('the server answered a workflow completion with no outcome this SDK knows', { task: lease.task_id });
        return { outcome: 'UNREPORTED' };
    }
  }

  /** How a command's blob-capable value travels: inline, a blob written now, or a re-emitted deferral's descriptor. */
  async #carrier(scope: LeaseScope, command: PreparedCommand, reEmitted: boolean): Promise<ValueCarrier> {
    if (!command.valueAsBlob) return undefined;
    // A re-emission keeps the carrier of its deferral (Decision 044-325): as a blob, the one the deferral holds.
    if (reEmitted) return 'descriptor';
    return { blobId: await this.#writeBlob(scope, blobValueOf(command)) };
  }

  async #fail(scope: LeaseScope, failure: WorkflowTaskFailure): Promise<[WorkflowTaskOutcome, string]> {
    const { lease, task } = scope;
    const request = {
      task_id: lease.task_id, run_id: lease.run_id, attempt: lease.attempt, generation: lease.generation, lease_token: lease.lease_token,
      idempotency_key: `workflow-fail-${lease.task_id}-${lease.generation}`, code_digest: task.code_digest, traceparent: task.traceparent,
      ...failureToWire(failure),
    };
    try {
      const answer = await this.#report(() => scope.rpc.failWorkflowTask(request, task.traceparent || undefined));
      return answer === 'stale' ? ['STALE', request.cause] : ['FAILED', request.cause];
    } catch (error) {
      this.#options.log.error('the failure of a workflow task could not be reported', { task: lease.task_id, code: error instanceof SagatError ? error.code : 'unknown' });
      return ['UNREPORTED', request.cause];
    }
  }

  /** A report repeated with the same idempotency key on a retriable error, at most five times. */
  async #report<T>(call: () => Promise<T>): Promise<T | 'stale'> {
    for (let n = 0; ; n++) {
      try {
        return await call();
      } catch (error) {
        if (error instanceof SagatError) {
          if (error.code === 'STALE_ATTEMPT') return 'stale';
          if (isRetriable(error) && n + 1 < REPORT_ATTEMPTS) {
            await sleep(Math.max(backoffDelayMs(this.#options.retry, n), error.retryAfterMs ?? 0));
            continue;
          }
        }
        throw error;
      }
    }
  }

  /** The function's own log output goes to the host's logger (it is empty in replay). */
  #relayLogs(lease: TaskLease, logs: { level: 'debug' | 'info' | 'warn' | 'error'; message: string }[]): void {
    for (const line of logs) this.#options.log[line.level](line.message, { task: lease.task_id, run: lease.run_id, source: 'workflow' });
  }
}

function lastStarted(history: HistoryEvent[]): Extract<HistoryEvent, { kind: 'workflow_task.started' }> | undefined {
  for (let i = history.length - 1; i >= 0; i--) {
    const event = history[i]!;
    if (event.kind === 'workflow_task.started') return event;
  }
  return undefined;
}
