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

export class WorkflowThread {
  readonly #worker: Worker;
  readonly #pending = new Map<number, { resolve: (value: unknown) => void; reject: (error: unknown) => void }>();
  #nextId = 1;
  #nextRun = 1;
  #dead: Error | undefined;

  constructor() {
    checkNodeVersion();
    // No inherited command-line options: a preloaded module (an instrumentation agent) would run in the thread.
    this.#worker = new Worker(new URL('./thread.js', import.meta.url), { execArgv: [], stdout: false, stderr: false });
    this.#worker.unref();
    this.#worker.on('message', (message: { id: number; ok?: unknown; error?: { name: string; message: string } }) => {
      const pending = this.#pending.get(message.id);
      if (!pending) return;
      this.#pending.delete(message.id);
      if (this.#pending.size === 0) this.#worker.unref();
      if (message.error) pending.reject(new WorkflowThreadError(message.error.name, message.error.message));
      else pending.resolve(message.ok);
    });
    const fail = (error: Error): void => {
      this.#dead = error;
      for (const pending of this.#pending.values()) pending.reject(error);
      this.#pending.clear();
    };
    this.#worker.on('error', (error) => fail(error));
    this.#worker.on('exit', (code) => fail(new Error(`the workflow thread ended with code ${code}`)));
  }

  /** The thread the replayer and the test environment share when none is given. */
  static shared(): WorkflowThread {
    if (!shared || shared.#dead) shared = new WorkflowThread();
    return shared;
  }

  request<T>(request: ThreadRequest): Promise<T> {
    if (this.#dead) return Promise.reject(this.#dead);
    const id = this.#nextId++;
    return new Promise<T>((resolve, reject) => {
      this.#pending.set(id, { resolve: resolve as (value: unknown) => void, reject });
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

  async replay(activation: ReplayActivation): Promise<ReplayStep> {
    const answer = await this.#thread.request<{ step: ReplayStep; state: RunState }>({ op: 'replay', runKey: this.#runKey, activation });
    this.#state = answer.state;
    return answer.step;
  }

  async activate(window: HistoryEvent[], started: ReplayActivation['started']): Promise<Completion | Failure> {
    const answer = await this.#thread.request<{ answer: Completion | Failure; state: RunState }>({ op: 'activate', runKey: this.#runKey, window, started });
    this.#state = answer.state;
    return answer.answer;
  }

  async commit(answer: CompletionAnswer): Promise<void> {
    const reply = await this.#thread.request<{ state: RunState }>({ op: 'commit', runKey: this.#runKey, answer });
    this.#state = reply.state;
  }

  async abandon(): Promise<void> {
    const reply = await this.#thread.request<{ state: RunState }>({ op: 'abandon', runKey: this.#runKey });
    this.#state = reply.state;
  }

  async bodyDigests(): Promise<Uint8Array[]> {
    return (await this.#thread.request<{ digests: Uint8Array[] }>({ op: 'bodyDigests', runKey: this.#runKey })).digests;
  }

  dispose(): void {
    void this.#thread.request({ op: 'drop', runKey: this.#runKey }).catch(() => undefined);
  }
}
