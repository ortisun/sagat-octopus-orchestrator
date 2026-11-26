/*
 * The test environment of workflow code (design 044, Task 044-10): it runs a workflow function of a bundle against a
 * small in-memory platform that keeps a journal the way the server does (the activation window, command records,
 * deferrals, the command chain), so that a developer's unit test can drive a run with stubbed activities, signals,
 * updates and cancellation, and replay the history it produced. It is not the platform: it applies no envelope, quota or
 * schema, and every activity runs at once with its stub.
 */
import { bodyDigest, EMPTY_CHAIN, extendChain, hex } from './chain.js';
import { TERMINAL_KINDS, type CommandKind, type EventOrigin, type HistoryEvent, type WorkflowTaskFailure } from './context.js';
import { ActivationCutter } from './history.js';
import { replayEvents, type ExportHeader, type ReplayVerdict } from './replayer.js';
import type { Completion, PreparedCommand } from './run.js';
import { WorkflowThread, type LoadedBundle, type RunHandle } from './threaded.js';

/** An activity stub fails its step with this. */
export class StubFailure extends Error {
  readonly code: string;
  readonly status: 'failed' | 'timed_out' | 'cancelled';
  constructor(code: string, status: 'failed' | 'timed_out' | 'cancelled' = 'failed') {
    super(code); this.name = 'StubFailure'; this.code = code; this.status = status;
  }
}

export type Stub = (input: unknown) => unknown;

/** A bundle's text, as `sagat-workflows bundle` wrote it (or a test made it). */
export interface BundleText { code: string; filename?: string }

export interface TestEnvironmentOptions {
  bundle: BundleText;
  workflowType: string;
  /** The run's seed (64 hexadecimal characters); a fixed one by default. */
  seed?: string;
  /** The time of the first event, in microseconds since the epoch. */
  startUs?: number;
  stateMaxBytes?: number;
  gatedEntries?: string[];
  activities?: Record<string, Stub>;
  scripts?: Record<string, Stub>;
  children?: Record<string, Stub>;
  approvals?: (policyName: string, subject: unknown) => 'approved' | 'rejected' | 'expired';
  /** The payload of an event a wait asks for, or `undefined` to let the wait time out. */
  events?: (eventType: string, correlationKey: string) => unknown;
  sagaOutcome?: (scope: string) => 'compensated' | 'partial' | 'compensation_failed';
  /** Replays the whole history in a new context for every workflow task, as a worker without a cache does. */
  coldReplay?: boolean;
  activationDeadlineMs?: number;
  /** The workflow thread the run's context lives in; the process's shared one by default. */
  thread?: WorkflowThread;
}

export type TestRunStatus = 'RUNNING' | 'COMPLETED' | 'FAILED' | 'CANCELLED' | 'CONTINUED_AS_NEW' | 'UNDER_REVIEW';

export interface TestOutcome {
  status: TestRunStatus;
  output?: unknown;
  failure?: { code: string; message: string };
  continuedAs?: Record<string, unknown>;
  review?: { cause: string; failure?: WorkflowTaskFailure };
  state?: unknown;
}

const FIXED_SEED = '000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f';
const WAKING: ReadonlySet<HistoryEvent['kind']> = new Set<HistoryEvent['kind']>([
  'run.started', 'step.outcome', 'timer.fired', 'wait.outcome', 'child.finished', 'approval.decided', 'saga.outcome',
  'signal.buffered', 'update.admitted', 'run.cancel_requested',
]);

interface OpenStep { kind: CommandKind; scope?: string; settled: boolean }
interface ScopeState { members: number[]; aborting: boolean; settled: boolean }

/** One run on an in-memory platform. */
export class WorkflowTestEnvironment {
  readonly #options: TestEnvironmentOptions;
  readonly history: HistoryEvent[] = [];
  readonly updates = new Map<number, { name: string; status: 'ADMITTED' | 'ACCEPTED' | 'REJECTED' | 'COMPLETED' | 'FAILED'; result?: unknown; reason?: unknown }>();
  /** The completions the platform received, in order, with its answer. */
  readonly completions: { completion: Completion; answer: string }[] = [];
  #nowUs: number;
  #seq = 0;
  #lastCompletedStart = 0;
  #lastWaking = 0;
  #run: RunHandle | undefined;
  #loaded: LoadedBundle | undefined;
  #chain: Uint8Array = EMPTY_CHAIN;
  #accepted = 0;
  #steps = new Map<number, OpenStep>();
  #timers: { at: number; commandNumber: number; kind: 'timer' | 'wait' }[] = [];
  #scopes = new Map<string, ScopeState>();
  #pendingSignals = new Set<number>();
  #failures = 0;
  #state: unknown;
  #outcome: TestOutcome = { status: 'RUNNING' };
  #cancelRequested = false;

  constructor(options: TestEnvironmentOptions) {
    this.#options = options;
    this.#nowUs = options.startUs ?? 1_767_225_600_000_000;
  }

  get outcome(): TestOutcome { return { ...this.#outcome, ...(this.#state === undefined ? {} : { state: this.#state }) }; }

  /** The header and the events of the run's history, as an export writes them. */
  exportLines(): string[] {
    const header: ExportHeader = {
      exportFormat: 1, seed: this.#seed(), gatedEntries: this.#options.gatedEntries ?? [], stateMaxBytes: this.#options.stateMaxBytes ?? 0,
    };
    return [JSON.stringify(header), ...this.history.map((event) => JSON.stringify(event))];
  }

  /** Replays the history this environment produced against a bundle (the same one or a changed one). */
  replay(bundle: BundleText = this.#options.bundle): Promise<ReplayVerdict> {
    const header: ExportHeader = {
      exportFormat: 1, seed: this.#seed(), gatedEntries: this.#options.gatedEntries ?? [], stateMaxBytes: this.#options.stateMaxBytes ?? 0,
    };
    const options = { bundleCode: bundle.code, workflowType: this.#options.workflowType, thread: this.#thread() };
    return replayEvents(header, this.history, options);
  }

  /** Drops the run's context from its thread. */
  dispose(): void {
    this.#run?.dispose();
    this.#run = undefined;
  }

  #thread(): WorkflowThread { return this.#options.thread ?? WorkflowThread.shared(); }

  start(input: unknown): this {
    if (this.#seq !== 0) throw new Error('the run has started');
    this.#append({ kind: 'run.started', input }, 'engine');
    return this;
  }

  signal(name: string, payload: unknown): number {
    const seq = this.#append({ kind: 'signal.buffered', name, payload }, 'external');
    this.#pendingSignals.add(seq);
    return seq;
  }

  update(name: string, args: unknown): number {
    const seq = this.#append({ kind: 'update.admitted', name, args }, 'external');
    this.updates.set(seq, { name, status: 'ADMITTED' });
    return seq;
  }

  /** Completes a step that has no stub (or one left open on purpose), as its worker would. */
  completeStep(commandNumber: number, output: unknown): void {
    this.#settle(commandNumber, { kind: 'step.outcome', commandNumber, status: 'completed', output: output === undefined ? null : output });
  }

  /** Fails a step that has no stub. */
  failStep(commandNumber: number, code: string, status: 'failed' | 'timed_out' | 'cancelled' = 'failed'): void {
    this.#settle(commandNumber, { kind: 'step.outcome', commandNumber, status, errorCode: code });
  }

  cancel(): void {
    if (this.#cancelRequested) return;
    this.#cancelRequested = true;
    this.#append({ kind: 'run.cancel_requested' }, 'external');
  }

  /** Runs workflow tasks, then fires the earliest timer, until the run ends, waits for the outside, or the bound. */
  async runUntilBlocked(maxTasks = 1000): Promise<TestOutcome> {
    for (let tasks = 0; tasks < maxTasks && this.#outcome.status === 'RUNNING'; tasks++) {
      if (this.#lastWaking > this.#lastCompletedStart) {
        await this.#workflowTask();
        continue;
      }
      const timer = this.#timers.sort((a, b) => a.at - b.at || a.commandNumber - b.commandNumber).shift();
      if (!timer) break;
      this.#nowUs = Math.max(this.#nowUs, timer.at);
      const step = this.#steps.get(timer.commandNumber);
      if (!step || step.settled) continue;
      step.settled = true;
      if (timer.kind === 'timer') this.#append({ kind: 'timer.fired', commandNumber: timer.commandNumber }, 'engine');
      else this.#append({ kind: 'wait.outcome', commandNumber: timer.commandNumber, status: 'timed_out' }, 'engine');
    }
    return this.outcome;
  }

  #seed(): string { return this.#options.seed ?? FIXED_SEED; }

  #append(body: DistributiveOmit<HistoryEvent, 'seq'>, origin: EventOrigin): number {
    this.#nowUs += 1000;
    const event = { ...body, seq: ++this.#seq, origin, atUs: this.#nowUs } as HistoryEvent;
    this.history.push(event);
    if (WAKING.has(event.kind) && origin !== 'completion') this.#lastWaking = event.seq;
    return event.seq;
  }

  async #workflowTask(): Promise<void> {
    const startedSeq = this.#append({ kind: 'workflow_task.started', continueAsNewSuggested: false, historyLimitReached: false }, 'engine');
    const started = this.history[this.history.length - 1] as Extract<HistoryEvent, { kind: 'workflow_task.started' }>;
    const run = await this.#prepareRun(startedSeq);
    const window = this.history.filter((event) => event.seq > run.windowStart && event.seq <= startedSeq);
    const answer = await run.activate(window, started);
    if (answer.kind === 'failure') {
      run.dispose();
      this.#run = undefined;
      return this.#taskFailed(answer.failure);
    }
    // The checks of the server's completion transaction that this environment keeps (Technical Plan 5.2).
    if (hex(answer.replayedChain) !== hex(this.#chain)) return this.#review('CHAIN_MISMATCH');
    if (answer.appliedThroughSeq < startedSeq) return this.#review('NONDETERMINISM_RISK');
    const terminal = answer.deferredCommand ?? answer.commands.find((command) => TERMINAL_KINDS.includes(command.kind));
    const deferral = terminal ? this.#deferralCause(terminal, answer.commands, startedSeq) : undefined;
    const accepted = answer.commands.filter((command) => !(deferral && command === terminal));
    if (!deferral && answer.deferredCommand) accepted.push({ ...answer.deferredCommand, commandNumber: this.#accepted + accepted.length + 1 });
    this.#append(deferral && terminal ? { kind: 'workflow_task.completed', deferral: { kind: terminal.kind, target: terminal.target } } : { kind: 'workflow_task.completed' }, 'completion');
    for (const command of accepted) {
      this.#chain = extendChain(this.#chain, command.commandNumber!, command.kind, command.target, bodyDigest(command));
      this.#accepted = command.commandNumber!;
      if (command.kind === 'marker_patch') {
        this.#append({ kind: 'marker.recorded', commandNumber: command.commandNumber!, markerKind: 'patch', markerId: command.target }, 'completion');
      } else if (command.kind === 'marker_side_effect') {
        this.#append({ kind: 'marker.recorded', commandNumber: command.commandNumber!, markerKind: 'side_effect', markerId: command.target, value: command.body['value'] }, 'completion');
      } else {
        this.#append({ kind: 'command.record', commandNumber: command.commandNumber!, commandKind: command.kind, target: command.target }, 'completion');
      }
    }
    this.#failures = 0;
    this.#lastCompletedStart = startedSeq;
    const answerKind = deferral ? 'ACCEPTED_DEFERRED' : 'ACCEPTED';
    this.completions.push({ completion: answer, answer: answerKind });
    await run.commit(answerKind);
    if (this.#options.coldReplay) { run.dispose(); this.#run = undefined; }
    for (const command of accepted) await this.#effect(command);
  }

  async #prepareRun(startedSeq: number): Promise<RunHandle> {
    // A run whose state is at the window start is reused (a cache hit); any other is replayed in a new context (4.2).
    if (this.#run && !this.#run.broken && this.#run.windowStart === this.#lastCompletedStart) return this.#run;
    this.#run?.dispose();
    this.#run = undefined;
    const thread = this.#thread();
    this.#loaded ??= await thread.loadBundle(this.#options.bundle.code, this.#options.bundle.filename);
    const run = await thread.createRun(this.#loaded, {
      workflowType: this.#options.workflowType, seed: this.#seed(),
      stateMaxBytes: this.#options.stateMaxBytes ?? 0, gatedEntries: this.#options.gatedEntries ?? [],
    }, this.#options.activationDeadlineMs === undefined ? {} : { activationDeadlineMs: this.#options.activationDeadlineMs });
    const cutter = new ActivationCutter();
    const activations = [...cutter.push(this.history.filter((event) => event.seq < startedSeq)), ...cutter.flush().activations];
    for (const activation of activations) {
      const step = await run.replay(activation);
      if (step.kind !== 'replayed') throw new Error(`the environment's own history did not replay: ${JSON.stringify(step)}`);
    }
    this.#run = run;
    return run;
  }

  #deferralCause(terminal: PreparedCommand, commands: PreparedCommand[], startedSeq: number): string | undefined {
    for (const scope of this.#scopes.values()) if (scope.aborting && !scope.settled) return 'OUTSTANDING_OBLIGATIONS';
    // A scope this completion aborts is aborting when the terminal command is judged.
    if (commands.some((command) => command.kind === 'saga_abort')) return 'OUTSTANDING_OBLIGATIONS';
    if (terminal.kind === 'continue_as_new') {
      if (commands.some((command) => ['activity', 'script', 'child', 'approval'].includes(command.kind))) return 'OUTSTANDING_OBLIGATIONS';
      for (const step of this.#steps.values()) {
        if (!step.settled && (step.kind === 'activity' || step.kind === 'script' || step.kind === 'child' || step.kind === 'approval')) return 'OUTSTANDING_OBLIGATIONS';
      }
    }
    if (this.#lastWaking > startedSeq) return 'UNHANDLED_EVENTS';
    return undefined;
  }

  #taskFailed(failure: WorkflowTaskFailure): void {
    const immediate = failure.cause === 'NONDETERMINISM' || failure.cause === 'VALIDATOR_COMMAND' || failure.cause === 'NONDETERMINISM_RISK';
    this.#failures++;
    // A counted failure leaves the activation pending: the next workflow task offers it again (6.3).
    if (immediate || this.#failures >= 5) this.#outcome = { status: 'UNDER_REVIEW', review: { cause: failure.cause, failure } };
  }

  #review(cause: string): void {
    this.#run?.dispose();
    this.#run = undefined;
    this.#outcome = { status: 'UNDER_REVIEW', review: { cause } };
  }

  async #effect(command: PreparedCommand): Promise<void> {
    const number = command.commandNumber!;
    const body = command.body;
    switch (command.kind) {
      case 'activity': case 'script': {
        const scope = command.sagaScope;
        this.#steps.set(number, { kind: command.kind, ...(scope ? { scope } : {}), settled: false });
        if (scope) this.#scopes.get(scope)?.members.push(number);
        const stub = command.kind === 'activity' ? this.#options.activities?.[command.target] : this.#options.scripts?.[command.target];
        if (!stub) return;
        try {
          const output = await stub(body['input']);
          this.#settle(number, { kind: 'step.outcome', commandNumber: number, status: 'completed', output: output === undefined ? null : output });
        } catch (error) {
          const failure = error instanceof StubFailure ? error : new StubFailure('ACTIVITY_FAILED');
          this.#settle(number, { kind: 'step.outcome', commandNumber: number, status: failure.status, errorCode: failure.code });
        }
        return;
      }
      case 'timer_start':
        this.#steps.set(number, { kind: command.kind, settled: false });
        this.#timers.push({ at: this.#nowUs + Number(body['durationMs']) * 1000, commandNumber: number, kind: 'timer' });
        return;
      case 'timer_cancel': case 'step_cancel': {
        const target = Number(command.target.slice(2));
        const step = this.#steps.get(target);
        if (!step || step.settled) return;
        step.settled = true;
        this.#timers = this.#timers.filter((timer) => timer.commandNumber !== target);
        if (command.kind === 'step_cancel') this.#append({ kind: 'step.outcome', commandNumber: target, status: 'cancelled' }, 'effect');
        return;
      }
      case 'wait_event': {
        this.#steps.set(number, { kind: command.kind, settled: false });
        const payload = this.#options.events?.(String(body['eventType']), String(body['correlationKey']));
        if (payload !== undefined) this.#settle(number, { kind: 'wait.outcome', commandNumber: number, status: 'satisfied', payload });
        else if (typeof body['timeoutMs'] === 'number') this.#timers.push({ at: this.#nowUs + body['timeoutMs'] * 1000, commandNumber: number, kind: 'wait' });
        return;
      }
      case 'child': {
        this.#steps.set(number, { kind: command.kind, settled: false });
        const stub = this.#options.children?.[command.target];
        if (!stub) return;
        try {
          const output = await stub(body['input']);
          this.#settle(number, { kind: 'child.finished', commandNumber: number, status: 'completed', output: output === undefined ? null : output });
        } catch (error) {
          this.#settle(number, { kind: 'child.finished', commandNumber: number, status: 'failed', errorCode: error instanceof StubFailure ? error.code : 'CHILD_FAILED' });
        }
        return;
      }
      case 'approval': {
        this.#steps.set(number, { kind: command.kind, settled: false });
        const decision = this.#options.approvals?.(command.target, body['subject']);
        if (decision) this.#settle(number, { kind: 'approval.decided', commandNumber: number, decision });
        return;
      }
      case 'saga_open':
        this.#scopes.set(command.target, { members: [], aborting: false, settled: false });
        return;
      case 'saga_close':
        this.#scopes.delete(command.target);
        return;
      case 'saga_abort': {
        const scope = this.#scopes.get(command.target);
        if (!scope) return;
        scope.aborting = true;
        this.#settleScopeIfDone(command.target);
        return;
      }
      case 'update_accept': case 'update_reject': case 'update_complete': {
        const update = this.updates.get(Number(command.target));
        if (!update) return;
        if (command.kind === 'update_accept') update.status = 'ACCEPTED';
        else if (command.kind === 'update_reject') { update.status = 'REJECTED'; update.reason = body['reason']; }
        else if ('result' in body) { update.status = 'COMPLETED'; update.result = body['result']; }
        else { update.status = 'FAILED'; update.reason = body['failureCode']; }
        return;
      }
      case 'consume_signals':
        for (const entry of (body['signals'] as { seqs: number[] }[])) for (const seq of entry.seqs) this.#pendingSignals.delete(seq);
        return;
      case 'set_state':
        this.#state = body['state'];
        return;
      case 'complete':
        this.#outcome = { status: 'COMPLETED', output: body['output'] };
        return;
      case 'fail':
        this.#outcome = { status: 'FAILED', failure: { code: String(body['code']), message: String(body['message']) } };
        return;
      case 'continue_as_new':
        this.#outcome = { status: this.#cancelRequested ? 'CANCELLED' : 'CONTINUED_AS_NEW', continuedAs: body };
        return;
      case 'acknowledge_cancellation':
        this.#outcome = { status: 'CANCELLED' };
        return;
      case 'marker_patch': case 'marker_side_effect':
        return;
    }
  }

  #settle(number: number, body: DistributiveOmit<HistoryEvent, 'seq'>): void {
    const step = this.#steps.get(number);
    if (!step || step.settled) return;
    step.settled = true;
    this.#append(body, 'external');
    if (step.scope) this.#settleScopeIfDone(step.scope);
  }

  #settleScopeIfDone(id: string): void {
    const scope = this.#scopes.get(id);
    if (!scope || !scope.aborting || scope.settled) return;
    if (!scope.members.every((member) => this.#steps.get(member)?.settled)) return;
    scope.settled = true;
    this.#append({ kind: 'saga.outcome', scope: id, outcome: this.#options.sagaOutcome?.(id) ?? 'compensated' }, 'engine');
  }
}

type DistributiveOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never;
