/*
 * The host side of the workflow thread (see `thread.ts`): a `WorkflowThread` owns one `node:worker_threads` thread, the
 * bundles loaded into it and the runs it holds; a `ThreadedRun` is the asynchronous face of one `CodeRun` there.
 */
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { Worker } from 'node:worker_threads';
import type { HistoryEvent, RunInit } from './context.js';
import type { ReplayActivation } from './history.js';
import { checkNodeVersion } from './preconditions.js';
import type { Completion, CompletionAnswer, Failure, ReplayStep } from './run.js';
import type { RunState, ThreadRequest } from './thread.js';

/** The face of a run that the replayer, the test environment and the worker use, wherever its context lives. */
export interface RunHandle {
  readonly windowStart: number;
  readonly acceptedCommands: number;
  readonly chain: Uint8Array;
  readonly broken: boolean;
  readonly closed: boolean;
  readonly deferredBodyDigest: Uint8Array | undefined;
  replay(activation: ReplayActivation): Promise<ReplayStep>;
  activate(window: HistoryEvent[], started: ReplayActivation['started']): Promise<Completion | Failure>;
  commit(answer: CompletionAnswer): Promise<void>;
  abandon(): Promise<void>;
  bodyDigests(): Promise<Uint8Array[]>;
  dispose(): void;
}

/** A bundle loaded into a thread: its key is the SHA-256 of its text, so a bundle is compiled once per digest. */
export interface LoadedBundle { readonly key: string; readonly workflowTypes: readonly string[] }

export class WorkflowThreadError extends Error {
  constructor(name: string, message: string) { super(message); this.name = name; }
}

let shared: WorkflowThread | undefined;

export interface WorkflowThreadOptions {
  /**
   * The longest a request may take before the thread is ended and every run it holds is lost (they are replayed by the
   * next lease): a request runs one activation, whose own deadline is at most the workflow task timeout of 60 s, or a
   * replay of one, so the default is the longest lease of a workflow task, 120,000 ms.
   */
  requestTimeoutMs?: number;
  /** The heap of the thread in MiB: one run's memory cannot take the worker's process with it (default 1,024). */
  maxOldGenerationSizeMb?: number;
  /** The thread's memory outside its heap (ArrayBuffer backing stores) in MiB, checked between evaluations (default 1,024). */
  maxExternalMemoryMb?: number;
}

export const DEFAULT_REQUEST_TIMEOUT_MS = 120_000;
export const DEFAULT_THREAD_HEAP_MB = 1024;
export const DEFAULT_THREAD_EXTERNAL_MB = 1024;

export class WorkflowThread {
  readonly #worker: Worker;
  readonly #pending = new Map<number, { resolve: (value: unknown) => void; reject: (error: unknown) => void; timer: NodeJS.Timeout }>();
  readonly #requestTimeoutMs: number;
  #nextId = 1;
  #nextRun = 1;
  #dead: Error | undefined;

  constructor(options: WorkflowThreadOptions = {}) {
    checkNodeVersion();
    this.#requestTimeoutMs = options.requestTimeoutMs ?? DEFAULT_REQUEST_TIMEOUT_MS;
    // No inherited command-line options (a preloaded instrumentation agent would run here and enable async hooks), only
    // the one that lets node:vm refuse a dynamic import with a primitive; no environment variable; a bounded heap.
    this.#worker = new Worker(new URL('./thread.js', import.meta.url), {
      execArgv: ['--experimental-vm-modules', '--no-warnings'],
      env: {},
      stdout: false,
      stderr: false,
      resourceLimits: { maxOldGenerationSizeMb: options.maxOldGenerationSizeMb ?? DEFAULT_THREAD_HEAP_MB },
      workerData: { maxExternalMemoryBytes: (options.maxExternalMemoryMb ?? DEFAULT_THREAD_EXTERNAL_MB) * 1024 * 1024 },
    });
    this.#worker.unref();
    this.#worker.on('message', (message: { id: number; ok?: unknown; error?: { name: string; message: string } }) => {
      const pending = this.#pending.get(message.id);
      if (!pending) return;
      this.#pending.delete(message.id);
      clearTimeout(pending.timer);
      if (this.#pending.size === 0) this.#worker.unref();
      if (message.error) pending.reject(new WorkflowThreadError(message.error.name, message.error.message));
      else pending.resolve(message.ok);
    });
    this.#worker.on('error', () => this.#fail(new WorkflowThreadError('WorkflowThreadExited', 'the workflow thread failed and its runs are lost')));
    this.#worker.on('exit', () => this.#fail(new WorkflowThreadError('WorkflowThreadExited', 'the workflow thread ended and its runs are lost')));
  }

  #fail(error: Error): void {
    this.#dead ??= error;
    for (const pending of this.#pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.#pending.clear();
  }

  /** Whether the thread ended (a failure, a stuck request or `terminate`): its runs are gone. */
  get dead(): boolean { return this.#dead !== undefined; }

  /** The thread the replayer and the test environment share when none is given. */
  static shared(): WorkflowThread {
    if (!shared || shared.#dead) shared = new WorkflowThread();
    return shared;
  }

  request<T>(request: ThreadRequest): Promise<T> {
    if (this.#dead) return Promise.reject(this.#dead);
    const id = this.#nextId++;
    return new Promise<T>((resolve, reject) => {
      // A request that never answers (code of a context that escaped its deadline) ends the thread rather than hanging.
      const timer = setTimeout(() => {
        this.#fail(new WorkflowThreadError('WorkflowThreadStuck', 'a request to the workflow thread did not answer in time; the thread is ended'));
        void this.#worker.terminate();
      }, this.#requestTimeoutMs);
      timer.unref();
      this.#pending.set(id, { resolve: resolve as (value: unknown) => void, reject, timer });
      this.#worker.ref();
      this.#worker.postMessage({ id, request });
    });
  }

  async loadBundle(code: string, filename = 'workflow-bundle.js'): Promise<LoadedBundle> {
    const key = createHash('sha256').update(code).digest('hex');
    const answer = await this.request<{ workflowTypes: string[] }>({ op: 'bundle', bundleKey: key, code, filename });
    return { key, workflowTypes: Object.freeze(answer.workflowTypes) };
  }

  loadBundleFile(path: string): Promise<LoadedBundle> {
    return this.loadBundle(readFileSync(path, 'utf8'), path);
  }

  async createRun(bundle: LoadedBundle, init: RunInit, options: { activationDeadlineMs?: number } = {}): Promise<ThreadedRun> {
    const runKey = `run-${this.#nextRun++}`;
    const request: ThreadRequest = { op: 'create', runKey, bundleKey: bundle.key, init };
    if (options.activationDeadlineMs !== undefined) request.activationDeadlineMs = options.activationDeadlineMs;
    const answer = await this.request<{ state: RunState }>(request);
    return new ThreadedRun(this, runKey, answer.state);
  }

  /** Ends the thread; the runs it held are gone. */
  async terminate(): Promise<void> {
    if (shared === this) shared = undefined;
    this.#fail(new WorkflowThreadError('WorkflowThreadExited', 'the workflow thread was ended'));
    await this.#worker.terminate();
  }
}

export class ThreadedRun implements RunHandle {
  readonly #thread: WorkflowThread;
  readonly #runKey: string;
  #state: RunState;

  constructor(thread: WorkflowThread, runKey: string, state: RunState) {
    this.#thread = thread;
    this.#runKey = runKey;
    this.#state = state;
  }

  get windowStart(): number { return this.#state.windowStart; }
  get acceptedCommands(): number { return this.#state.acceptedCommands; }
  get chain(): Uint8Array { return this.#state.chain; }
  get broken(): boolean { return this.#state.broken; }
  get closed(): boolean { return this.#state.closed; }
  get deferredBodyDigest(): Uint8Array | undefined { return this.#state.deferredBodyDigest; }

  // Any failure of a request leaves the run's context in an unknown state: the run is broken and must be replayed.
  async #call<T extends { state?: RunState }>(request: ThreadRequest): Promise<T> {
    try {
      const answer = await this.#thread.request<T>(request);
      if (answer.state) this.#state = answer.state;
      return answer;
    } catch (error) {
      this.#state = { ...this.#state, broken: true };
      throw error;
    }
  }

  async replay(activation: ReplayActivation): Promise<ReplayStep> {
    return (await this.#call<{ step: ReplayStep; state: RunState }>({ op: 'replay', runKey: this.#runKey, activation })).step;
  }

  async activate(window: HistoryEvent[], started: ReplayActivation['started']): Promise<Completion | Failure> {
    return (await this.#call<{ answer: Completion | Failure; state: RunState }>({ op: 'activate', runKey: this.#runKey, window, started })).answer;
  }

  async commit(answer: CompletionAnswer): Promise<void> {
    await this.#call<{ state: RunState }>({ op: 'commit', runKey: this.#runKey, answer });
  }

  async abandon(): Promise<void> {
    await this.#call<{ state: RunState }>({ op: 'abandon', runKey: this.#runKey });
  }

  async bodyDigests(): Promise<Uint8Array[]> {
    return (await this.#thread.request<{ digests: Uint8Array[] }>({ op: 'bodyDigests', runKey: this.#runKey })).digests;
  }

  dispose(): void {
    void this.#thread.request({ op: 'drop', runKey: this.#runKey }).catch(() => undefined);
  }
}
