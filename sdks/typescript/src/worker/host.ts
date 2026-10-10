import { PROTOCOL_VERSION, silentLogger, warnIfNotSecure, type Logger, type TlsOptions, type TokenSource } from '../config.js';
import { MAX_WAIT_MS, SagatError, isRetriable } from '../errors.js';
import { encodeJson, parseJson } from '../json.js';
import { DEFAULT_RETRY, backoffDelayMs, sleep, type RetryPolicy } from '../retry.js';
import {
  ActivityFailure,
  isCancellation,
  type ActivityContext,
  type ActivityHandler,
  type RegisterOptions,
  type Secrets,
  type TaskKind,
  type Verdict,
} from './activity.js';
import { WorkerRpc, type TaskLease } from './grpc.js';
import type * as grpc from '@grpc/grpc-js';

export interface WorkerConfig {
  /** `http://host:port` or `https://host:port` of the worker service. */
  grpcUrl: string;
  /** Slug of the namespace the worker serves. */
  namespace: string;
  token: TokenSource;
  /** One live session per name and identity: registering again under the same name replaces it. */
  workerName: string;
  /** Tasks in flight at once, 1 to 1000; 10 by default. */
  maxInflight?: number;
  /** The `languages` capability; `typescript` by default. */
  languages?: string[];
  pools?: string[];
  tls?: TlsOptions;
  timeoutMs?: number;
  retry?: Partial<RetryPolicy>;
  /** Overrides the newest protocol minor the host offers (tests use it to play an older or a newer SDK). */
  protocolVersion?: string;
  /** Overrides the heartbeat interval (otherwise a third of the lease, 500 ms to 10 s). */
  heartbeatIntervalMs?: number;
  logger?: Logger;
  interceptors?: grpc.Interceptor[];
  /** Called for each thing that happens in the host: the hook for metrics. */
  onEvent?: (event: WorkerEvent) => void;
}

export type WorkerEvent =
  | { type: 'registered'; protocolVersion: string }
  | { type: 'poll'; tasks: number }
  | { type: 'task.taken'; taskId: string; activity: string; attempt: number }
  | { type: 'task.completed'; taskId: string; durationMs: number }
  | { type: 'task.failed'; taskId: string; code: string; retryable: boolean; durationMs: number }
  | { type: 'task.stale'; taskId: string }
  | { type: 'task.dropped'; taskId: string; reason: string }
  | { type: 'heartbeat'; taskId: string }
  | { type: 'reconnect' };

type Kind = 'activity' | 'compensation' | 'reconciler';

interface Registered {
  kind: Kind;
  handler: ActivityHandler<unknown, unknown>;
  parse?: (raw: unknown) => unknown;
}

const MAX_OUTPUT_BYTES = 256 * 1024;
const MAX_POLL = 64;
const CODE = /^[A-Z0-9_]{1,64}$/;

function minorOf(version: string): number {
  const match = /^1(?:\.(\d+))?$/.exec(version);
  return match ? Number(match[1] ?? 0) : 0;
}

/** The worker host of the SDK (SPEC §6). */
export class WorkerHost {
  private readonly handlers = new Map<string, Registered>();
  readonly #rpc: WorkerRpc;
  readonly #config: WorkerConfig;
  private readonly log: Logger;
  private readonly retry: RetryPolicy;
  private readonly maxInflight: number;
  private workerId = '';
  #session = '';
  private accepted = '';
  private running = false;
  private stopping = false;
  private loop: Promise<void> | undefined;
  private wake: (() => void) | undefined;
  private readonly inflight = new Map<string, { abort: AbortController; done: Promise<void> }>();

  constructor(config: WorkerConfig) {
    this.#config = config;
    this.maxInflight = config.maxInflight ?? 10;
    if (!Number.isInteger(this.maxInflight) || this.maxInflight < 1 || this.maxInflight > 1000) {
      throw new SagatError({ code: 'VALIDATION_FAILED', message: 'maxInflight is between 1 and 1000' });
    }
    this.log = config.logger ?? silentLogger;
    this.retry = { ...DEFAULT_RETRY, ...config.retry };
    this.#rpc = new WorkerRpc({
      grpcUrl: config.grpcUrl,
      token: config.token,
      tls: config.tls,
      timeoutMs: config.timeoutMs,
      interceptors: config.interceptors,
    });
  }

  /** A hook of the caller that throws must not take the host, or the process, with it. */
  #emit(event: WorkerEvent): void {
    try {
      this.#config.onEvent?.(event);
    } catch {
      this.log.warn('an event hook threw; the host goes on');
    }
  }

  /** The version the server agreed to; empty before `start`. */
  get protocolVersion(): string {
    return this.accepted;
  }

  private add<I, O>(kind: Kind, name: string, handler: ActivityHandler<I, O>, options?: RegisterOptions<I>): this {
    if (this.running) throw new SagatError({ code: 'VALIDATION_FAILED', message: 'activities are registered before the host starts' });
    if (this.handlers.has(name)) throw new SagatError({ code: 'VALIDATION_FAILED', message: `an activity named ${name} is registered already` });
    this.handlers.set(name, {
      kind,
      handler: handler as ActivityHandler<unknown, unknown>,
      parse: options?.parse as ((raw: unknown) => unknown) | undefined,
    });
    return this;
  }

  /** Registers the handler of an activity. `parse` turns the run input into the handler's type. */
  registerActivity<I = unknown, O = unknown>(name: string, handler: ActivityHandler<I, O>, options?: RegisterOptions<I>): this {
    return this.add('activity', name, handler, options);
  }

  /** Registers the handler of the activity a node names in `compensateWith`. It must be idempotent. */
  registerCompensation<I = unknown, O = unknown>(name: string, handler: ActivityHandler<I, O>, options?: RegisterOptions<I>): this {
    return this.add('compensation', name, handler, options);
  }

  /** Registers the handler of the activity a node names in `reconcileWith`; it answers a `Verdict`. */
  registerReconciler<I = unknown>(name: string, handler: ActivityHandler<I, Verdict>, options?: RegisterOptions<I>): this {
    return this.add('reconciler', name, handler, options);
  }

  /** Registers with the server and starts the loop; resolves once registered. */
  async start(): Promise<void> {
    if (this.running) return;
    if (this.handlers.size === 0) throw new SagatError({ code: 'VALIDATION_FAILED', message: 'register at least one activity before start' });
    warnIfNotSecure(this.#config.grpcUrl, this.log);
    await this.register();
    this.running = true;
    this.stopping = false;
    this.loop = this.run();
  }

  /**
   * Stops polling, waits up to `timeoutMs` (30 s by default) for the tasks in flight, then aborts the
   * ones left (reason `shutdown`). A task that did not finish keeps its lease until it lapses.
   */
  async stop(timeoutMs = 30_000): Promise<void> {
    if (!this.running) return;
    this.stopping = true;
    this.wake?.();
    // The poll in progress ends and no other starts; what it leased is in flight from here on.
    await this.loop;
    const pending = Promise.all([...this.inflight.values()].map((task) => task.done));
    let late: NodeJS.Timeout | undefined;
    const timer = new Promise<'late'>((resolve) => {
      late = setTimeout(() => resolve('late'), timeoutMs);
    });
    const outcome = await Promise.race([pending.then(() => 'done' as const), timer]);
    clearTimeout(late);
    if (outcome === 'late') {
      for (const task of this.inflight.values()) task.abort.abort('shutdown');
      await Promise.race([pending, sleep(2000).catch(() => undefined)]);
    }
    this.running = false;
    this.#rpc.close();
  }

  // ------------------------------------------------------------------------------------ registration

  private async register(): Promise<void> {
    const request = {
      worker_name: this.#config.workerName,
      namespace: this.#config.namespace,
      capabilities: {
        languages: this.#config.languages ?? ['typescript'],
        activities: [...this.handlers.keys()].sort(),
        pools: this.#config.pools ?? [],
        max_inflight: this.maxInflight,
        protocol_version: this.#config.protocolVersion ?? PROTOCOL_VERSION,
      },
    };
    let refreshed = false;
    for (let n = 0; ; n++) {
      try {
        const response = await this.#rpc.registerWorker(request);
        this.workerId = response.worker_id;
        this.#session = response.session_token;
        this.accepted = response.accepted_protocol_version;
        this.log.info('worker registered', { worker: this.#config.workerName, protocol: this.accepted });
        this.#emit({ type: 'registered', protocolVersion: this.accepted });
        return;
      } catch (error) {
        if (!(error instanceof SagatError)) throw error;
        // A token the provider can refresh gets one more chance (SPEC §5).
        if (error.code === 'UNAUTHENTICATED' && !refreshed && typeof this.#config.token === 'function') {
          refreshed = true;
          continue;
        }
        if (!isRetriable(error) || n + 1 >= this.retry.maxAttempts) throw error;
        await sleep(Math.max(backoffDelayMs(this.retry, n), error.retryAfterMs ?? 0));
      }
    }
  }

  // ------------------------------------------------------------------------------------------- loop

  // The pause between polls. It keeps the process alive (a host that is running is work to wait for), and a
  // host that is stopping does not pause: `stop` may have run while a poll was in flight, with nothing to wake.
  private idle(ms: number): Promise<void> {
    if (this.stopping) return Promise.resolve();
    return new Promise((resolve) => {
      const timer = setTimeout(done, ms);
      const self = this;
      function done(): void {
        clearTimeout(timer);
        self.wake = undefined;
        resolve();
      }
      this.wake = done;
    });
  }

  private async run(): Promise<void> {
    let failures = 0;
    // Refusals of the session in a row, after registering again: a server that registers a worker and refuses
    // it at once is not asked again at once (SPEC §6.2).
    let refused = 0;
    while (!this.stopping) {
      try {
        const free = this.maxInflight - this.inflight.size;
        if (free <= 0) {
          await this.idle(50);
          continue;
        }
        const response = await this.#rpc.pollTasks({ worker_id: this.workerId, session_token: this.#session, max_tasks: Math.min(free, MAX_POLL) });
        failures = 0;
        refused = 0;
        this.#emit({ type: 'poll', tasks: response.tasks.length });
        // At most as many as there is room for: the platform honours `max_tasks`, a broken one is not believed.
        for (const lease of response.tasks.slice(0, free)) this.dispatch(lease);
        if (response.tasks.length > free) {
          this.log.warn('the platform leased more tasks than were asked for; the others are left to lapse', { asked: free, got: response.tasks.length });
        }
        if (response.tasks.length === 0) {
          const wait = Math.min(MAX_WAIT_MS, Math.max(50, response.retry_after_ms > 0 ? response.retry_after_ms : 1000));
          await this.idle(Math.round(wait * (0.8 + Math.random() * 0.4)));
        }
      } catch (error) {
        if (this.stopping) return;
        if (!(error instanceof SagatError)) {
          this.log.error('the poll loop failed', { error: error instanceof Error ? error.name : 'unknown' });
          await this.idle(1000);
          continue;
        }
        if (error.code === 'UNAUTHENTICATED') {
          if (refused++ > 0) await this.idle(Math.min(30_000, backoffDelayMs({ ...this.retry, maxDelayMs: 30_000 }, refused - 2)));
          if (this.stopping) return;
          this.log.warn('the session is gone: registering again');
          this.#emit({ type: 'reconnect' });
          try {
            await this.register();
            continue;
          } catch (again) {
            this.log.error('registering again failed', { code: again instanceof SagatError ? again.code : 'unknown' });
          }
        }
        if (error.code === 'RATE_LIMITED') {
          await this.idle(Math.max(error.retryAfterMs ?? 0, 100));
          continue;
        }
        this.#emit({ type: 'reconnect' });
        await this.idle(Math.min(30_000, Math.max(backoffDelayMs({ ...this.retry, maxDelayMs: 30_000 }, failures++), error.retryAfterMs ?? 0)));
      }
    }
  }

  // ------------------------------------------------------------------------------------------- tasks

  private dispatch(lease: TaskLease): void {
    const abort = new AbortController();
    const done = this.execute(lease, abort)
      .catch((error: unknown) => {
        // Nothing here may reject: `stop` waits on these and an unhandled rejection ends the process.
        this.log.error('a task failed unexpectedly', { task: lease.task_id, error: error instanceof Error ? error.name : 'unknown' });
      })
      .finally(() => {
      this.inflight.delete(lease.task_id);
      // A task finished: there is room, so the pause after an empty poll ends (SPEC §6.2).
      this.wake?.();
    });
    this.inflight.set(lease.task_id, { abort, done });
    this.#emit({ type: 'task.taken', taskId: lease.task_id, activity: lease.activity_name, attempt: lease.attempt });
  }

  private async execute(lease: TaskLease, abort: AbortController): Promise<void> {
    // The heartbeat runs until the report is made, whatever way the task ends (SPEC §6.3).
    const cleanup: Array<() => void> = [];
    try {
      await this.serve(lease, abort, cleanup);
    } finally {
      for (const done of cleanup) done();
    }
  }

  private async serve(lease: TaskLease, abort: AbortController, cleanup: Array<() => void>): Promise<void> {
    const started = Date.now();
    const registered = this.handlers.get(lease.activity_name);
    const state = { deadline: lease.expires_at_epoch_ms, lost: false };
    const kind: TaskKind = lease.task_kind === 'COMPENSATION' || lease.task_kind === 'RECONCILIATION' ? lease.task_kind : 'FORWARD';
    const minor = minorOf(this.accepted);

    if (!registered) {
      await this.fail(lease, 'NO_HANDLER', 'no handler is registered for the activity', false, started);
      return;
    }

    let document: { node?: string; input?: unknown; steps?: Record<string, { output?: unknown }> };
    try {
      const parsed: unknown = lease.input_json === '' ? {} : parseJson(lease.input_json);
      document = parsed !== null && typeof parsed === 'object' ? (parsed as typeof document) : {};
    } catch (error) {
      // Not JSON, or an integer beyond 2^53 - 1 that a JavaScript number would round (SPEC §8).
      await this.fail(lease, 'INPUT_INVALID', error instanceof SagatError ? error.message : 'the step document is not JSON', false, started);
      return;
    }
    let input: unknown = document.input;
    try {
      if (registered.parse) input = registered.parse(document.input);
    } catch {
      await this.fail(lease, 'INPUT_INVALID', 'the input does not have the type of the handler', false, started);
      return;
    }
    const steps: Record<string, unknown> = {};
    for (const [node, value] of Object.entries(document.steps ?? {})) steps[node] = value?.output;

    // The heartbeat starts before the handler and runs until the report is made (`execute` stops it).
    cleanup.push(this.beat(lease, minor, abort, state).stop);

    let secretsCache: Promise<Secrets> | undefined;
    const secretsHolder: { dispose?: () => void } = {};
    // The secrets are wiped and refused when the handler is done, however it ended.
    cleanup.push(() => secretsHolder.dispose?.());
    const context: ActivityContext = {
      activityName: lease.activity_name,
      taskId: lease.task_id,
      runId: lease.run_id,
      node: typeof document.node === 'string' && document.node !== '' ? document.node : lease.step_id,
      stepId: lease.step_id,
      attempt: lease.attempt,
      generation: lease.generation,
      idempotencyKey: lease.idempotency_key,
      traceparent: lease.traceparent,
      taskKind: kind,
      targetNode: lease.target_node,
      targetIdempotencyKey: lease.target_idempotency_key,
      steps,
      get deadline() {
        return new Date(state.deadline);
      },
      signal: abort.signal,
      log: {
        debug: (message, fields) => this.log.debug(message, { ...fields, task: lease.task_id }),
        info: (message, fields) => this.log.info(message, { ...fields, task: lease.task_id }),
        warn: (message, fields) => this.log.warn(message, { ...fields, task: lease.task_id }),
        error: (message, fields) => this.log.error(message, { ...fields, task: lease.task_id }),
      },
      secrets: () => {
        if (!secretsCache) {
          const pending = this.resolveSecrets(lease, minor, secretsHolder);
          secretsCache = pending;
          // A failed resolution is not remembered for the rest of the attempt: the next call asks again.
          pending.catch(() => {
            if (secretsCache === pending) secretsCache = undefined;
          });
        }
        return secretsCache;
      },
    };

    let result: unknown;
    let failure: ActivityFailure | undefined;
    let leftToLapse = false;
    try {
      result = await registered.handler(input, context);
    } catch (error) {
      if (error instanceof ActivityFailure) {
        failure = error;
      } else if (isCancellation(error) && abort.signal.aborted) {
        // Cancelled by the platform: the step is told so. Cancelled because this host is stopping: the step
        // is not failed for good by a deploy, its lease lapses and it is leased again (SPEC §6.4).
        if (abort.signal.reason === 'shutdown') leftToLapse = true;
        else failure = ActivityFailure.permanent('CANCELLED', 'the activity was cancelled');
      } else {
        failure = ActivityFailure.retryable('ACTIVITY_ERROR', error instanceof Error ? error.name : 'Error');
      }
    }

    if (leftToLapse) {
      this.log.info('the host stopped: the cancelled activity is not reported, its lease will lapse', { task: lease.task_id });
      return;
    }
    if (state.lost) {
      this.#emit({ type: 'task.stale', taskId: lease.task_id });
      this.log.warn('the lease was lost: the result is dropped', { task: lease.task_id });
      return;
    }
    if (failure) {
      await this.fail(lease, failure.code, failure.message, failure.retryable, started);
      return;
    }

    if (lease.task_kind === 'RECONCILIATION' && registered.kind === 'reconciler') {
      const verdict = result as Partial<Verdict> | undefined;
      const effect = verdict?.effect;
      if (effect !== 'applied' && effect !== 'not_applied' && effect !== 'unknown') {
        await this.fail(lease, 'RECONCILIATION_ANSWER_INVALID', 'a reconciler answers applied, notApplied or unknown', false, started);
        return;
      }
      result = effect === 'applied' && (verdict as { output?: unknown }).output !== undefined
        ? { effect, output: (verdict as { output?: unknown }).output }
        : { effect };
    }

    let output: string;
    try {
      output = result === undefined ? '' : encodeJson(result);
    } catch (error) {
      await this.fail(lease, 'OUTPUT_INVALID', error instanceof SagatError ? error.message : 'the output is not JSON', false, started);
      return;
    }
    if (Buffer.byteLength(output) > MAX_OUTPUT_BYTES) {
      await this.fail(lease, 'PAYLOAD_TOO_LARGE', 'the output is over 256 KiB', false, started);
      return;
    }
    await this.complete(lease, output, started);
  }

  // ----------------------------------------------------------------------------------- heartbeat

  private beat(
    lease: TaskLease,
    minor: number,
    abort: AbortController,
    state: { deadline: number; lost: boolean },
  ): { stop(): void } {
    let stopped = false;
    let timer: NodeJS.Timeout | undefined;
    const interval = (): number => {
      if (this.#config.heartbeatIntervalMs) return this.#config.heartbeatIntervalMs;
      const left = Math.max(0, state.deadline - Date.now());
      return Math.min(10_000, Math.max(500, Math.round(left / 3)));
    };
    const tick = async (): Promise<void> => {
      if (stopped) return;
      try {
        const answer = await this.#rpc.heartbeat(
          {
            task_id: lease.task_id,
            worker_id: this.workerId,
            generation: lease.generation,
            lease_token: lease.lease_token,
            // The step identity came with 1.1.
            run_id: minor >= 1 ? lease.run_id : '',
            step_id: minor >= 1 ? lease.step_id : '',
            attempt: minor >= 1 ? lease.attempt : 0,
          },
          lease.traceparent || undefined,
        );
        this.#emit({ type: 'heartbeat', taskId: lease.task_id });
        if (answer.lease_expires_at_epoch_ms > 0) state.deadline = answer.lease_expires_at_epoch_ms;
        if (answer.cancel_requested && !abort.signal.aborted) abort.abort('run-cancelled');
      } catch (error) {
        if (error instanceof SagatError && (error.code === 'STALE_ATTEMPT' || error.code === 'CONFLICT')) {
          state.lost = true;
          if (!abort.signal.aborted) abort.abort('lease-lost');
          return;
        }
        // Anything else: the next beat tries again; the lease is ours until it runs out.
        this.log.warn('a heartbeat failed', { task: lease.task_id, code: error instanceof SagatError ? error.code : 'unknown' });
      }
      if (!stopped) {
        timer = setTimeout(() => void tick(), interval());
      }
    };
    timer = setTimeout(() => void tick(), interval());
    return {
      stop: () => {
        stopped = true;
        if (timer) clearTimeout(timer);
      },
    };
  }

  // ------------------------------------------------------------------------------------ reporting

  private async withRetries<T>(call: () => Promise<T>): Promise<T | 'stale'> {
    const attempts = 5;
    for (let n = 0; ; n++) {
      try {
        return await call();
      } catch (error) {
        if (error instanceof SagatError) {
          if (error.code === 'STALE_ATTEMPT') return 'stale';
          if (isRetriable(error) && n + 1 < attempts) {
            await sleep(Math.max(backoffDelayMs(this.retry, n), error.retryAfterMs ?? 0));
            continue;
          }
        }
        throw error;
      }
    }
  }

  private async complete(lease: TaskLease, output: string, started: number): Promise<void> {
    try {
      const answer = await this.withRetries(() =>
        this.#rpc.completeStep(
          {
            task_id: lease.task_id,
            run_id: lease.run_id,
            step_id: lease.step_id,
            attempt: lease.attempt,
            generation: lease.generation,
            lease_token: lease.lease_token,
            idempotency_key: `done-${lease.task_id}`,
            output_json: output,
            output_blob_ref: '',
            output_sha256: '',
            traceparent: lease.traceparent,
          },
          lease.traceparent || undefined,
        ),
      );
      if (answer === 'stale') {
        this.#emit({ type: 'task.stale', taskId: lease.task_id });
        return;
      }
      this.#emit({ type: 'task.completed', taskId: lease.task_id, durationMs: Date.now() - started });
    } catch (error) {
      this.#emit({ type: 'task.dropped', taskId: lease.task_id, reason: error instanceof SagatError ? error.code : 'unknown' });
      this.log.error('the completion could not be reported', { task: lease.task_id, code: error instanceof SagatError ? error.code : 'unknown' });
    }
  }

  private async fail(lease: TaskLease, code: string, message: string, retryable: boolean, started: number): Promise<void> {
    const safeCode = CODE.test(code) ? code : 'WORKER_ERROR';
    try {
      const answer = await this.withRetries(() =>
        this.#rpc.failStep(
          {
            task_id: lease.task_id,
            run_id: lease.run_id,
            step_id: lease.step_id,
            attempt: lease.attempt,
            generation: lease.generation,
            lease_token: lease.lease_token,
            idempotency_key: `failed-${lease.task_id}`,
            error_code: safeCode,
            safe_message: message.slice(0, 512),
            retryable,
            traceparent: lease.traceparent,
          },
          lease.traceparent || undefined,
        ),
      );
      if (answer === 'stale') {
        this.#emit({ type: 'task.stale', taskId: lease.task_id });
        return;
      }
      this.#emit({ type: 'task.failed', taskId: lease.task_id, code: safeCode, retryable, durationMs: Date.now() - started });
    } catch (error) {
      this.#emit({ type: 'task.dropped', taskId: lease.task_id, reason: error instanceof SagatError ? error.code : 'unknown' });
      this.log.error('the failure could not be reported', { task: lease.task_id, code: error instanceof SagatError ? error.code : 'unknown' });
    }
  }

  // --------------------------------------------------------------------------------------- secrets

  private async resolveSecrets(lease: TaskLease, minor: number, holder: { dispose?: () => void }): Promise<Secrets> {
    if (minor < 2) {
      throw new SagatError({ code: 'UNSUPPORTED_PROTOCOL', message: 'the server speaks a protocol older than 1.2, which has no ResolveSecrets' });
    }
    const answer = await this.#rpc.resolveSecrets(
      {
        task_id: lease.task_id,
        run_id: lease.run_id,
        step_id: lease.step_id,
        attempt: lease.attempt,
        generation: lease.generation,
        lease_token: lease.lease_token,
        traceparent: lease.traceparent,
      },
      lease.traceparent || undefined,
    );
    const values = new Map(answer.secrets.map((secret) => [secret.name, Buffer.from(secret.value)]));
    let disposed = false;
    const alive = (): void => {
      if (disposed) throw new SagatError({ code: 'VALIDATION_FAILED', message: 'the secrets of an attempt are only valid while its handler runs' });
    };
    holder.dispose = () => {
      disposed = true;
      for (const value of values.values()) value.fill(0);
      values.clear();
    };
    return {
      // A copy: what the caller does to it (wiping it, say) is the caller's, and the host's own is wiped by the host.
      get: (name) => {
        alive();
        const value = values.get(name);
        if (!value) throw new SagatError({ code: 'NOT_FOUND', message: `the step declares no secret named ${name}` });
        return Buffer.from(value);
      },
      text: (name) => {
        alive();
        const value = values.get(name);
        if (!value) throw new SagatError({ code: 'NOT_FOUND', message: `the step declares no secret named ${name}` });
        return value.toString('utf8');
      },
      names: () => {
        alive();
        return [...values.keys()];
      },
    };
  }
}
