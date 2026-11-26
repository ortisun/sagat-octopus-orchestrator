/*
 * The host side of one code run: its context, the activations it replays and the live one, the command chain, and the
 * completion it answers (design 044, Technical Plan 4 and 6). A worker keeps a `CodeRun` in its cache between workflow
 * tasks and reuses it when the next lease's window starts where it stopped (section 4.2); any other worker replays.
 */
import { bodyDigest, EMPTY_CHAIN, extendChain } from './chain.js';
import {
  BLOB_CAPABLE_FIELD, canonicalSize, MAX_INLINE_BYTES, TERMINAL_KINDS,
  type ActivationResult, type ExpectedRecord, type HistoryEvent, type LogLine, type RunInit, type WorkflowCommand, type WorkflowTaskFailure,
} from './context.js';
import { appliedEvents, beginOf, type ReplayActivation } from './history.js';
import { ActivationDeadlineExceeded, DEFAULT_ACTIVATION_DEADLINE_MS, MINIMUM_ACTIVATION_DEADLINE_MS, WorkflowSandbox, type WorkflowBundle } from './sandbox.js';

export interface CodeRunOptions {
  /** The activation deadline in milliseconds: from 100 to the workflow task timeout; 2,000 by default. */
  activationDeadlineMs?: number;
  /** The clock of the deadline (tests inject one). */
  clock?: () => number;
}

/** A command ready to send: its body digest, and whether its value field travels as a blob. */
export interface PreparedCommand extends WorkflowCommand {
  bodyDigest: Uint8Array;
  /** The value field (`input`, `value`, `result`, `output`) is written as a blob, largest first (6.2, 13.1). */
  valueAsBlob: boolean;
}

export interface Completion {
  kind: 'completion';
  /** The journal sequence of the last event of the window the function applied: S(k). */
  appliedThroughSeq: number;
  /** The chain over the commands before this activation's first new command. */
  replayedChain: Uint8Array;
  commands: PreparedCommand[];
  /** A terminal or continue-as-new command re-emitted after a deferral: it has no number (4.4). */
  deferredCommand?: PreparedCommand;
  logs: LogLine[];
}

export interface Failure {
  kind: 'failure';
  failure: WorkflowTaskFailure;
  logs: LogLine[];
}

/** The answer of the server to a completion (13.1). */
export type CompletionAnswer = 'ACCEPTED' | 'ACCEPTED_DEFERRED';

/** What a replayed activation found. */
export type ReplayStep =
  | { kind: 'replayed' }
  | { kind: 'failure'; failure: WorkflowTaskFailure }
  | { kind: 'inconclusive'; commandNumber: number };

export class CodeRun {
  readonly init: RunInit;
  readonly #sandbox: WorkflowSandbox;
  readonly #deadlineMs: number;
  readonly #clock: () => number;
  #windowStart = 0;
  #accepted = 0;
  #chain: Uint8Array = EMPTY_CHAIN;
  #bodyDigests: Uint8Array[] = [];
  #deferredDigest: Uint8Array | undefined;
  #inFlight: { completion: Completion; startedSeq: number } | undefined;
  #broken = false;
  #closed = false;

  constructor(bundle: WorkflowBundle, init: RunInit, options: CodeRunOptions = {}) {
    const deadline = options.activationDeadlineMs ?? DEFAULT_ACTIVATION_DEADLINE_MS;
    if (!Number.isFinite(deadline) || deadline < MINIMUM_ACTIVATION_DEADLINE_MS) {
      throw new RangeError(`the activation deadline is at least ${MINIMUM_ACTIVATION_DEADLINE_MS} ms`);
    }
    this.init = init;
    this.#deadlineMs = deadline;
    this.#clock = options.clock ?? (() => performance.now());
    this.#sandbox = new WorkflowSandbox(bundle, deadline);
    this.#sandbox.call(JSON.stringify({ op: 'init', init }), deadline);
  }

  /** S of the last completed activation this run holds: a lease whose window starts here can reuse it. */
  get windowStart(): number { return this.#windowStart; }
  /** The number of accepted commands. */
  get acceptedCommands(): number { return this.#accepted; }
  /** The command chain over the accepted commands. */
  get chain(): Uint8Array { return this.#chain; }
  /** The body digest of every accepted command, by command number (index 0 is command 1). */
  get bodyDigests(): readonly Uint8Array[] { return this.#bodyDigests; }
  /** The body digest of the terminal command a deferral holds. */
  get deferredBodyDigest(): Uint8Array | undefined { return this.#deferredDigest; }
  /** A deadline or a failure left the context in an unknown state: the run must be dropped from the cache. */
  get broken(): boolean { return this.#broken; }
  /** The run's terminal command was accepted: no workflow task follows. */
  get closed(): boolean { return this.#closed; }

  /** Replays one completed activation against its records (6.1). */
  replay(activation: ReplayActivation): ReplayStep {
    this.#ensureUsable();
    const begin = beginOf(activation.window, activation.started, true);
    begin.expected = activation.records;
    if (activation.deferral) begin.expectedDeferral = activation.deferral;
    const result = this.#activate(activation.window, begin);
    if (result.inconclusiveAt !== undefined) {
      this.#broken = true;
      return { kind: 'inconclusive', commandNumber: result.inconclusiveAt };
    }
    if (result.failure) {
      this.#broken = true;
      return { kind: 'failure', failure: result.failure };
    }
    const deferred = activation.deferral !== undefined;
    this.#sandbox.call(JSON.stringify({ op: 'commit', deferred }), this.#deadlineMs);
    const numbered = result.commands.filter((command) => command.commandNumber !== undefined);
    for (const command of numbered) this.#chainCommand(command);
    const terminal = result.commands.find((command) => TERMINAL_KINDS.includes(command.kind));
    if (deferred && terminal) this.#deferredDigest = bodyDigest(terminal);
    else if (terminal) { this.#deferredDigest = undefined; this.#closed = true; }
    this.#windowStart = activation.started.seq;
    return { kind: 'replayed' };
  }

  /** Runs the live activation of a lease's window and prepares its completion or its failure. */
  activate(window: HistoryEvent[], started: ReplayActivation['started']): Completion | Failure {
    this.#ensureUsable();
    if (this.#inFlight) throw new Error('the previous completion of this run has no answer yet');
    const result = this.#activate(window, beginOf(window, started, false));
    if (result.failure) {
      this.#broken = true;
      return { kind: 'failure', failure: result.failure, logs: result.logs };
    }
    const prepared = prepare(result.commands);
    const completion: Completion = {
      kind: 'completion',
      appliedThroughSeq: started.seq,
      replayedChain: this.#chain,
      commands: prepared.filter((command) => command.commandNumber !== undefined),
      logs: result.logs,
    };
    const deferred = prepared.find((command) => command.commandNumber === undefined);
    if (deferred) completion.deferredCommand = deferred;
    this.#inFlight = { completion, startedSeq: started.seq };
    return completion;
  }

  /** Applies the server's answer to the completion in flight: the run's state moves to S(k). */
  commit(answer: CompletionAnswer): void {
    const inFlight = this.#inFlight;
    if (!inFlight) throw new Error('no completion of this run is waiting for an answer');
    this.#inFlight = undefined;
    const deferred = answer === 'ACCEPTED_DEFERRED';
    this.#sandbox.call(JSON.stringify({ op: 'commit', deferred }), this.#deadlineMs);
    const { commands, deferredCommand } = inFlight.completion;
    const last = commands[commands.length - 1];
    const firstTimeTerminal = last && TERMINAL_KINDS.includes(last.kind) ? last : undefined;
    for (const command of commands) {
      if (deferred && command === firstTimeTerminal) continue;
      this.#chainCommand(command);
    }
    if (deferred) {
      const held = firstTimeTerminal ?? deferredCommand;
      if (held) this.#deferredDigest = held.bodyDigest;
    } else {
      if (deferredCommand) this.#chainCommand({ ...deferredCommand, commandNumber: this.#accepted + 1 });
      if (firstTimeTerminal || deferredCommand) { this.#deferredDigest = undefined; this.#closed = true; }
    }
    this.#windowStart = inFlight.startedSeq;
  }

  /** The completion in flight was lost or refused: the run must be replayed by whoever takes it next. */
  abandon(): void {
    this.#inFlight = undefined;
    this.#broken = true;
  }

  #ensureUsable(): void {
    if (this.#broken) throw new Error('this run\'s context was dropped after a failure or a deadline');
  }

  #activate(window: HistoryEvent[], begin: ReturnType<typeof beginOf>): ActivationResult {
    const started = this.#clock();
    const remaining = (): number => {
      const left = this.#deadlineMs - (this.#clock() - started);
      if (left <= 0) throw new ActivationDeadlineExceeded();
      return left;
    };
    try {
      this.#sandbox.call(JSON.stringify({ op: 'begin', begin }), remaining());
      for (const event of appliedEvents(window)) {
        const answer = this.#sandbox.call(JSON.stringify({ op: 'apply', event }), remaining());
        if (answer['stopped'] === true) break;
      }
      return checkResult(this.#sandbox.call(JSON.stringify({ op: 'end' }), remaining()));
    } catch (error) {
      if (error instanceof ActivationDeadlineExceeded) {
        this.#broken = true;
        return { commands: [], failure: { cause: 'ACTIVATION_DEADLINE' }, logs: [], open: true };
      }
      throw error;
    }
  }

  #chainCommand(command: WorkflowCommand & { bodyDigest?: Uint8Array }): void {
    const number = this.#accepted + 1;
    if (command.commandNumber !== undefined && command.commandNumber !== number) {
      throw new Error(`command ${command.commandNumber} follows ${this.#accepted}: the runtime numbered out of order`);
    }
    const digest = command.bodyDigest ?? bodyDigest(command);
    this.#chain = extendChain(this.#chain, number, command.kind, command.target, digest);
    this.#bodyDigests.push(digest);
    this.#accepted = number;
  }
}

/** Expected records shaped for the runtime (tests and the replayer build them from a history). */
export type { ExpectedRecord };

const KINDS = new Set<string>([
  'activity', 'script', 'timer_start', 'timer_cancel', 'wait_event', 'child', 'approval', 'step_cancel', 'saga_open', 'saga_close',
  'saga_abort', 'marker_patch', 'marker_side_effect', 'update_accept', 'update_reject', 'update_complete', 'consume_signals',
  'set_state', 'complete', 'fail', 'continue_as_new', 'acknowledge_cancellation',
]);

// The answer comes from the context: its shape is checked before the host uses it.
function checkResult(answer: Record<string, unknown>): ActivationResult {
  const commands = answer['commands'];
  if (!Array.isArray(commands)) throw new Error('the activation answered no command list');
  for (const command of commands as Record<string, unknown>[]) {
    if (!command || typeof command !== 'object' || !KINDS.has(command['kind'] as string) || typeof command['target'] !== 'string'
      || !command['body'] || typeof command['body'] !== 'object'
      || (command['commandNumber'] !== undefined && !Number.isSafeInteger(command['commandNumber']))) {
      throw new Error('the activation answered a malformed command');
    }
  }
  return answer as unknown as ActivationResult;
}

/**
 * The body digests, and the values that travel as blobs: whenever the completion's values would not fit its 192 KiB of
 * inline values, the largest value that may travel as a blob is moved first (Non-functional requirement 044-006).
 */
export function prepare(commands: WorkflowCommand[]): PreparedCommand[] {
  const prepared: PreparedCommand[] = commands.map((command) => ({ ...command, bodyDigest: bodyDigest(command), valueAsBlob: false }));
  let inline = 0;
  const movable: { command: PreparedCommand; size: number }[] = [];
  for (const command of prepared) {
    const field = BLOB_CAPABLE_FIELD[command.kind];
    for (const key of ['input', 'value', 'result', 'output', 'state', 'subject', 'signals']) {
      if (!(key in command.body)) continue;
      const size = canonicalSize(command.body[key]);
      if (key === field && !command.alwaysInline) movable.push({ command, size });
      else inline += size;
    }
  }
  movable.sort((a, b) => b.size - a.size);
  let movableBytes = movable.reduce((sum, item) => sum + item.size, 0);
  for (const item of movable) {
    if (inline + movableBytes <= MAX_INLINE_BYTES) break;
    item.command.valueAsBlob = true;
    movableBytes -= item.size;
  }
  return prepared;
}
