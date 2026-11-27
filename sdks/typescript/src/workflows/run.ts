/*
 * The host side of one code run: its context, the activations it replays and the live one, the command chain, and the
 * completion it answers (design 044, Technical Plan 4 and 6). A worker keeps a `CodeRun` in its cache between workflow
 * tasks and reuses it when the next lease's window starts where it stopped (section 4.2); any other worker replays.
 */
import { bodyDigest, EMPTY_CHAIN, extendChain } from './chain.js';
import {
  BLOB_CAPABLE_FIELD, canonicalSize, MAX_BLOB_CARRIED_BYTES, MAX_BLOBS, MAX_FUNCTION_COMMANDS, MAX_FUNCTION_METADATA_BYTES,
  MAX_FAILURE_MESSAGE, MAX_INLINE_BYTES, MAX_SIDE_EFFECT_BYTES, metadataSize, TERMINAL_KINDS,
  type ActivationResult, type ExpectedRecord, type HistoryEvent, type LogLine, type RunInit, type WorkflowCommand, type WorkflowTaskFailure,
} from './context.js';
import { appliedEvents, beginOf, type ReplayActivation } from './history.js';
import { ActivationDeadlineExceeded, DEFAULT_ACTIVATION_DEADLINE_MS, MINIMUM_ACTIVATION_DEADLINE_MS, WorkflowSandbox, type WorkflowBundle } from './sandbox.js';

export interface CodeRunOptions {
  /** The activation deadline in milliseconds: from 100 to the workflow task timeout; 2,000 by default. */
  activationDeadlineMs?: number;
  /** The clock of the deadline (tests inject one). */
  clock?: () => number;
  /**
   * Whether the thread's memory outside its heap is over its bound, asked after every evaluation (Decision 044-323);
   * the workflow thread gives it.
   */
  memoryExceeded?: () => boolean;
}

/** The thread's external memory (the backing stores of ArrayBuffers) went over its bound during an activation. */
export class ExternalMemoryExceeded extends Error {
  constructor() { super('the workflow thread\'s memory outside its heap is over its bound'); this.name = 'ExternalMemoryExceeded'; }
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
  readonly #memoryExceeded: (() => boolean) | undefined;
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
    this.#memoryExceeded = options.memoryExceeded;
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
    try {
      const deferred = activation.deferral !== undefined;
      const numbered = result.commands.filter((command) => command.commandNumber !== undefined);
      checkNumbering(numbered, this.#accepted);
      for (const command of numbered) this.#chainCommand(command);
      const terminal = result.commands.find((command) => TERMINAL_KINDS.includes(command.kind));
      if (deferred && terminal) this.#deferredDigest = bodyDigest(terminal);
      else if (terminal) { this.#deferredDigest = undefined; this.#closed = true; }
      this.#windowStart = activation.started.seq;
      this.#sandbox.call(JSON.stringify({ op: 'commit', deferred }), this.#deadlineMs);
      return { kind: 'replayed' };
    } catch {
      this.#broken = true;
      return { kind: 'failure', failure: { cause: 'NONDETERMINISM_RISK' } };
    }
  }

  /** Runs the live activation of a lease's window and prepares its completion or its failure. */
  activate(window: HistoryEvent[], started: ReplayActivation['started']): Completion | Failure {
    this.#ensureUsable();
    if (this.#inFlight) throw new Error('the previous completion of this run has no answer yet');
    // The window is (S(k − 1), S(k)]: an event committed after this lease's start belongs to the next activation.
    const own = window.filter((event) => event.seq > this.#windowStart && event.seq <= started.seq);
    const result = this.#activate(own, beginOf(own, started, false));
    if (result.failure) {
      this.#broken = true;
      return { kind: 'failure', failure: result.failure, logs: result.logs };
    }
    let prepared: PreparedCommand[];
    try {
      // The runtime's bounds are checked again here, on values the context can no longer change: a context whose code
      // defeated its own checks fails the workflow task as a risk instead of sending what the server would refuse.
      checkNumbering(result.commands.filter((command) => command.commandNumber !== undefined), this.#accepted);
      prepared = prepareChecked(result.commands, this.init);
    } catch {
      this.#broken = true;
      return { kind: 'failure', failure: { cause: 'NONDETERMINISM_RISK' }, logs: result.logs };
    }
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

  /**
   * Applies the server's answer to the completion in flight: the run's state moves to S(k). A failure here leaves the
   * run broken (dropped from the cache and replayed by the next lease); the server's state is not affected.
   */
  commit(answer: CompletionAnswer): void {
    const inFlight = this.#inFlight;
    if (!inFlight) throw new Error('no completion of this run is waiting for an answer');
    this.#inFlight = undefined;
    const deferred = answer === 'ACCEPTED_DEFERRED';
    try {
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
      this.#sandbox.call(JSON.stringify({ op: 'commit', deferred }), this.#deadlineMs);
    } catch {
      this.#broken = true;
    }
  }

  /** The completion in flight was lost or refused: the run must be replayed by whoever takes it next. */
  abandon(): void {
    this.#inFlight = undefined;
    this.#broken = true;
  }

  #ensureUsable(): void {
    if (this.#broken) throw new Error('this run\'s context was dropped after a failure or a deadline');
  }

  // Never throws: a deadline is ACTIVATION_DEADLINE, and anything else that goes wrong between the host and the context
  // (a bridge that fails, an answer the host refuses) is NONDETERMINISM_RISK; either way the context is dropped.
  #activate(window: HistoryEvent[], begin: ReturnType<typeof beginOf>): ActivationResult {
    const started = this.#clock();
    const remaining = (): number => {
      const left = this.#deadlineMs - (this.#clock() - started);
      if (left <= 0) throw new ActivationDeadlineExceeded();
      return left;
    };
    const call = (request: string): Record<string, unknown> => {
      const answer = this.#sandbox.call(request, remaining());
      if (this.#memoryExceeded?.()) throw new ExternalMemoryExceeded();
      return answer;
    };
    try {
      call(JSON.stringify({ op: 'begin', begin }));
      for (const event of appliedEvents(window)) {
        const answer = call(JSON.stringify({ op: 'apply', event }));
        if (answer['stopped'] === true) break;
      }
      return checkActivationAnswer(call(JSON.stringify({ op: 'end' })));
    } catch (error) {
      this.#broken = true;
      if (error instanceof ExternalMemoryExceeded) {
        return { commands: [], failure: { cause: 'WORKFLOW_TASK_FAILED', exceptionType: 'ExternalMemoryExceeded' }, logs: [], open: true };
      }
      const cause = error instanceof ActivationDeadlineExceeded ? 'ACTIVATION_DEADLINE' : 'NONDETERMINISM_RISK';
      return { commands: [], failure: { cause }, logs: [], open: true };
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
const SDK_CAUSES = new Set<string>(['NONDETERMINISM', 'WORKFLOW_TASK_FAILED', 'ACTIVATION_DEADLINE', 'VALIDATOR_COMMAND', 'NONDETERMINISM_RISK']);
const EXCEPTION_TYPE = /^[A-Za-z_][A-Za-z0-9_.$]{0,127}$/;
// Every target of Technical Plan 6.2: a name, an identifier `c-<n>` or `s-<n>`, a script digest, a decimal sequence, or empty.
const TARGET = /^$|^[A-Za-z0-9][A-Za-z0-9_.:-]{0,255}$/;
const LOG_LEVELS = new Set(['debug', 'info', 'warn', 'error']);
/** `FailWorkflowTaskRequest.actual_saga_scope`: a scope `s-<n>`, or empty outside any. */
const SAGA_SCOPE_OR_NONE = /^$|^s-[1-9][0-9]{0,9}$/;
/** The SDK's share of an activation (Requirement 044-012) and a completion's commands (Non-functional requirement 044-006). */
const MAX_SDK_COMMANDS = 147;
const MAX_COMPLETION_COMMANDS = 403;

class AnswerRefused extends Error {
  constructor(what: string) { super(`the context answered ${what}`); this.name = 'AnswerRefused'; }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

// The answer comes from the context, whose code may have replaced the runtime's own checks: every field the host uses,
// or that may reach a report, is checked here, and a field outside its pattern is dropped rather than relayed.
/** Checks the context's answer to an activation (exported for the tests of this package). */
export function checkActivationAnswer(answer: Record<string, unknown>): ActivationResult {
  const commands = answer['commands'];
  if (!Array.isArray(commands) || commands.length > MAX_COMPLETION_COMMANDS) throw new AnswerRefused('no command list');
  for (const command of commands as unknown[]) {
    if (!isRecord(command) || !KINDS.has(command['kind'] as string) || typeof command['target'] !== 'string' || !TARGET.test(command['target'])
      || !isRecord(command['body']) || typeof command['issuedBySdk'] !== 'boolean' || typeof command['alwaysInline'] !== 'boolean'
      || (command['commandNumber'] !== undefined && (!Number.isSafeInteger(command['commandNumber']) || (command['commandNumber'] as number) < 1))
      || (command['sagaScope'] !== undefined && !/^s-[1-9][0-9]{0,15}$/.test(String(command['sagaScope'])))
      || !typedBodyHolds(command['kind'] as string, command['target'], command['body'])) {
      throw new AnswerRefused('a malformed command');
    }
  }
  const result: ActivationResult = { commands: commands as WorkflowCommand[], logs: [], open: answer['open'] === true };
  const failure = answer['failure'];
  if (failure !== undefined) {
    if (!isRecord(failure) || !SDK_CAUSES.has(failure['cause'] as string)) throw new AnswerRefused('a failure of no cause the SDK raises');
    const checked: WorkflowTaskFailure = { cause: failure['cause'] as WorkflowTaskFailure['cause'] };
    if (Number.isSafeInteger(failure['commandNumber']) && (failure['commandNumber'] as number) >= 0) checked.commandNumber = failure['commandNumber'] as number;
    if (typeof failure['actualKind'] === 'string' && (failure['actualKind'] === '' || KINDS.has(failure['actualKind']))) checked.actualKind = failure['actualKind'];
    if (typeof failure['actualTarget'] === 'string' && TARGET.test(failure['actualTarget'])) checked.actualTarget = failure['actualTarget'];
    // Empty outside a scope; anything but `s-<n>` is dropped, the report keeping its cause (Decision 044-250).
    if (typeof failure['actualSagaScope'] === 'string' && SAGA_SCOPE_OR_NONE.test(failure['actualSagaScope'])) checked.actualSagaScope = failure['actualSagaScope'];
    if (typeof failure['exceptionType'] === 'string' && EXCEPTION_TYPE.test(failure['exceptionType'])) checked.exceptionType = failure['exceptionType'];
    result.failure = checked;
  }
  if (answer['inconclusiveAt'] !== undefined) {
    if (!Number.isSafeInteger(answer['inconclusiveAt'])) throw new AnswerRefused('an inconclusive point that is no command number');
    result.inconclusiveAt = answer['inconclusiveAt'] as number;
  }
  // The function's log lines are the tenant's own output for its own logger (Technical Plan 12, design 041's row), bounded.
  let logBytes = 0;
  for (const line of Array.isArray(answer['logs']) ? answer['logs'].slice(0, 1000) : []) {
    if (!isRecord(line) || !LOG_LEVELS.has(line['level'] as string) || typeof line['message'] !== 'string') continue;
    const message = line['message'].slice(0, 8192);
    logBytes += message.length;
    if (logBytes > 256 * 1024) break;
    result.logs.push({ level: line['level'] as LogLine['level'], message });
  }
  return result;
}

const CODE = /^[A-Z][A-Z0-9_]{0,63}$/;
const SCOPE = /^s-[1-9][0-9]{0,15}$/;
const REJECTION_REASONS = new Set(['VALIDATION_FAILED', 'HANDLER_NOT_REGISTERED']);
const ON_COMPENSATION_FAILURE = new Set(['manual_review', 'fail']);
const CANCEL_POLICIES = new Set(['compensate_completed', 'wait_in_flight_then_compensate', 'abort_no_compensation']);
const MARKER = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$/;

/** Exactly the keys of each typed body, each within its pattern, list or bound (Decision 044-324). */
function typedBodyHolds(kind: string, target: string, body: Record<string, unknown>): boolean {
  const keys = (...names: string[]): boolean => Object.keys(body).every((key) => names.includes(key));
  const seqOfTarget = (): boolean => Number.isSafeInteger(body['admittedSeq']) && String(body['admittedSeq']) === target;
  switch (kind) {
    case 'timer_cancel': case 'step_cancel': return keys('target') && body['target'] === target && /^c-[1-9][0-9]{0,15}$/.test(target);
    case 'saga_open':
      return keys('scope', 'parentScope', 'onCompensationFailure', 'cancelPolicy') && body['scope'] === target && SCOPE.test(target)
        && (body['parentScope'] === null || (typeof body['parentScope'] === 'string' && SCOPE.test(body['parentScope'])))
        && ON_COMPENSATION_FAILURE.has(body['onCompensationFailure'] as string) && CANCEL_POLICIES.has(body['cancelPolicy'] as string);
    case 'saga_close': case 'saga_abort': return keys('scope') && body['scope'] === target && SCOPE.test(target);
    case 'marker_patch': return keys('markerId') && body['markerId'] === target && MARKER.test(target);
    case 'marker_side_effect': return keys('markerId', 'value') && body['markerId'] === target && MARKER.test(target) && 'value' in body;
    case 'update_accept': return keys('admittedSeq', 'accepted') && seqOfTarget() && body['accepted'] === true;
    case 'update_reject':
      return keys('admittedSeq', 'accepted', 'reason') && seqOfTarget() && body['accepted'] === false && REJECTION_REASONS.has(body['reason'] as string);
    case 'update_complete':
      return seqOfTarget() && ((keys('admittedSeq', 'result') && 'result' in body)
        || (keys('admittedSeq', 'failureCode') && typeof body['failureCode'] === 'string' && CODE.test(body['failureCode'])));
    case 'consume_signals': return keys('signals') && Array.isArray(body['signals']) && target === '';
    case 'set_state': return keys('state') && target === '';
    case 'complete': return keys('output') && target === '';
    case 'fail':
      return keys('code', 'message') && target === '' && typeof body['code'] === 'string' && CODE.test(body['code'])
        && typeof body['message'] === 'string' && body['message'].length <= MAX_FAILURE_MESSAGE;
    case 'continue_as_new': return keys('input', 'state', 'version') && target === '' && (body['version'] === 'same' || body['version'] === 'active');
    case 'acknowledge_cancellation': return keys() && target === '';
    default: return true;
  }
}

function checkNumbering(numbered: WorkflowCommand[], accepted: number): void {
  numbered.forEach((command, index) => {
    if (command.commandNumber !== accepted + index + 1) throw new AnswerRefused('commands numbered out of order');
  });
}

// The bounds of Requirement 044-012 and Non-functional requirement 044-006, on the values as they will be sent; the
// host decides which values stay inline from the kind and the gated entries, not from the context's flags.
function prepareChecked(commands: WorkflowCommand[], init: RunInit): PreparedCommand[] {
  const gated = new Set(init.gatedEntries);
  let functionCommands = 0;
  let functionMetadata = 0;
  let sdkCommands = 0;
  const checked = commands.map((command) => {
    const terminal = TERMINAL_KINDS.includes(command.kind);
    if (command.issuedBySdk) sdkCommands++;
    else if (!terminal) {
      functionCommands++;
      functionMetadata += metadataSize(command.kind, command.target, command.body);
    }
    if (command.kind === 'set_state' && canonicalSize(command.body['state']) > init.stateMaxBytes) throw new AnswerRefused('a declared state over its maxBytes');
    if (command.kind === 'continue_as_new' && command.body['state'] !== null && canonicalSize(command.body['state']) > init.stateMaxBytes) {
      throw new AnswerRefused('a carried state over its maxBytes');
    }
    if (command.kind === 'marker_side_effect' && canonicalSize(command.body['value']) > MAX_SIDE_EFFECT_BYTES) throw new AnswerRefused('a side-effect value over its bound');
    const alwaysInline = command.kind === 'approval' || (command.kind === 'activity' && gated.has(command.target));
    return { ...command, alwaysInline };
  });
  if (functionCommands > MAX_FUNCTION_COMMANDS || functionMetadata > MAX_FUNCTION_METADATA_BYTES || sdkCommands > MAX_SDK_COMMANDS) {
    throw new AnswerRefused('more commands than an activation may make');
  }
  const prepared = prepare(checked);
  let inline = 0;
  let blobs = 0;
  let blobBytes = 0;
  for (const command of prepared) {
    const field = BLOB_CAPABLE_FIELD[command.kind];
    for (const key of ['input', 'value', 'result', 'output', 'state', 'subject', 'signals', 'message']) {
      if (!(key in command.body)) continue;
      const size = canonicalSize(command.body[key]);
      if (key === field && command.valueAsBlob) { blobs++; blobBytes += size; } else inline += size;
    }
  }
  if (inline > MAX_INLINE_BYTES || blobs > MAX_BLOBS || blobBytes > MAX_BLOB_CARRIED_BYTES) throw new AnswerRefused('values one completion cannot carry');
  return prepared;
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
