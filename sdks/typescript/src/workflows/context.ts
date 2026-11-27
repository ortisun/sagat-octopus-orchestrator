/*
 * The workflow runtime that runs inside a run's `node:vm` context (design 044, Technical Plan 1.2).
 *
 * This file is self-contained on purpose: it imports nothing and touches no Node API, because the bundler puts it into
 * the workflow bundle (the package export condition `sagat-workflow`) and the bundle is evaluated in a context that holds
 * no host object. The host talks to it only through `globalThis.__sagat_bridge.call(<JSON string>)`, which answers a
 * JSON string. Everything a run can observe besides its history is made deterministic here: the time is the time of the
 * activation's last `workflow_task.started`, random numbers come from xoshiro256** seeded by the run, timers are durable
 * timer commands, and the default locale and time zone are fixed.
 *
 * The host process imports this module too (for its types, the canonical form and the generator); there its run state is
 * never set, so every workflow call throws `NotInWorkflow`.
 */

/** The version of the string bridge between the host and this runtime; the host refuses a bundle of another one. */
export const BRIDGE_VERSION = 1;

// ---------------------------------------------------------------------------------------------------------------------
// Bounds of one activation (Requirement 044-012, Non-functional requirement 044-006).

/** Commands the function's own calls may make in one activation. */
export const MAX_FUNCTION_COMMANDS = 256;
/**
 * Bytes of command metadata (fragments without their literal values) the function's calls may make in one activation;
 * the SDK's own commands have 16 KiB more, so a completion holds at most 64 KiB (Requirement 044-012).
 */
export const MAX_FUNCTION_METADATA_BYTES = 48 * 1024;
/** Bytes of values a completion carries inline: the declared state, gated inputs, subjects, continue-as-new values. */
export const MAX_INLINE_BYTES = 192 * 1024;
/** Values written as blobs in one completion. */
export const MAX_BLOBS = 16;
/** Bytes of blob-carried values the server validates for one completion. */
export const MAX_BLOB_CARRIED_BYTES = 4 * 1024 * 1024;
/** The SDK's own commands in one activation, and of them the saga scope closes and aborts (Requirement 044-012). */
export const MAX_SDK_COMMANDS = 147;
export const MAX_SCOPE_COMMANDS = 16;
/** Open saga scopes of a run (Requirement 044-032). */
export const MAX_OPEN_SCOPES = 16;
/** Characters of a workflow failure's message: it is stored as a payload, and the completion must fit its message. */
export const MAX_FAILURE_MESSAGE = 4096;
/** Bytes of one side-effect value. */
export const MAX_SIDE_EFFECT_BYTES = 64 * 1024;

const MARKER_ID = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$/;
const FAILURE_CODE = /^[A-Z][A-Z0-9_]{0,63}$/;
const EXCEPTION_TYPE = /^[A-Za-z_][A-Za-z0-9_.$]{0,127}$/;
/** The fixed locale of workflow code: V8 has no data for the root locale `und` and falls back to the process's default. */
export const WORKFLOW_LOCALE = 'en-US';
/** The identifier of every side-effect marker: the kind already says what it is, and a run cannot name one. */
export const SIDE_EFFECT_MARKER_ID = 'side-effect';

// ---------------------------------------------------------------------------------------------------------------------
// The wire-neutral model the host and this runtime exchange. The worker protocol's `HistoryEvent` and `Command`
// (Technical Plan 13.1) are mapped to and from it by the host, in one module.

export type EventOrigin = 'completion' | 'effect' | 'engine' | 'external';

/** A command kind, as the command chain names it (Technical Plan 6.2). */
export type CommandKind =
  | 'activity' | 'script' | 'timer_start' | 'timer_cancel' | 'wait_event' | 'child' | 'approval' | 'step_cancel'
  | 'saga_open' | 'saga_close' | 'saga_abort' | 'marker_patch' | 'marker_side_effect'
  | 'update_accept' | 'update_reject' | 'update_complete' | 'consume_signals' | 'set_state'
  | 'complete' | 'fail' | 'continue_as_new' | 'acknowledge_cancellation';

export const TERMINAL_KINDS: readonly CommandKind[] = ['complete', 'fail', 'continue_as_new', 'acknowledge_cancellation'];

interface EventBase { seq: number; origin?: EventOrigin; atUs?: number; redacted?: boolean }

export type HistoryEvent = EventBase & (
  | { kind: 'run.started'; input?: unknown; carriedState?: unknown }
  // `sagaScope` is the scope `s-<n>` an activity or a script was scheduled in (`HistoryCommandRecord.saga_scope`).
  | { kind: 'command.record'; commandNumber: number; commandKind: CommandKind; target: string; sagaScope?: string }
  | { kind: 'marker.recorded'; commandNumber: number; markerKind: 'patch' | 'side_effect'; markerId: string; value?: unknown }
  | { kind: 'step.outcome'; commandNumber: number; status: 'completed' | 'failed' | 'timed_out' | 'cancelled'; output?: unknown; errorCode?: string; errorMessage?: string }
  | { kind: 'timer.fired'; commandNumber: number }
  | { kind: 'wait.outcome'; commandNumber: number; status: 'satisfied' | 'timed_out'; payload?: unknown }
  | { kind: 'child.finished'; commandNumber: number; status: 'completed' | 'failed' | 'cancelled' | 'timed_out' | 'partial'; output?: unknown; errorCode?: string }
  | { kind: 'approval.decided'; commandNumber: number; decision: 'approved' | 'rejected' | 'expired' }
  | { kind: 'saga.outcome'; scope: string; outcome: 'compensated' | 'partial' | 'compensation_failed' }
  | { kind: 'signal.buffered'; name: string; payload?: unknown }
  | { kind: 'update.admitted'; name: string; args?: unknown }
  | { kind: 'run.cancel_requested' }
  | { kind: 'workflow_task.started'; continueAsNewSuggested?: boolean; historyLimitReached?: boolean }
  | { kind: 'workflow_task.completed'; deferral?: { kind: CommandKind; target: string } }
);

/**
 * The value field of each kind that enters the hashed body as its descriptor `{"sha256", "size"}` (Technical Plan 6.2,
 * the body table of the corpus `specs/044-workflow-as-code/contracts/goldens/README.md`).
 */
export const DESCRIPTOR_FIELD: Partial<Record<CommandKind, 'input' | 'subject' | 'value' | 'result' | 'output' | 'message'>> = {
  activity: 'input', script: 'input', child: 'input', approval: 'subject', marker_side_effect: 'value', update_complete: 'result',
  complete: 'output', fail: 'message',
};

/** The value field of each kind that may travel as a blob when the completion would not fit (an approval's never). */
export const BLOB_CAPABLE_FIELD: Partial<Record<CommandKind, 'input' | 'value' | 'result' | 'output'>> = {
  activity: 'input', script: 'input', child: 'input', marker_side_effect: 'value', update_complete: 'result', complete: 'output',
};

/** One command an activation made, as the runtime hands it to the host. */
export interface WorkflowCommand {
  /** The command number; absent for a terminal command re-emitted after a deferral (the server numbers it). */
  commandNumber?: number;
  kind: CommandKind;
  target: string;
  /**
   * The body the chain hashes, with its value field as the value itself: a work command's code-profile fragment with
   * its value (`input`, an approval's `subject`) beside the fragment's own keys, or a typed command's body. The worker
   * protocol sends a work command's value apart from its fragment.
   */
  body: Record<string, unknown>;
  /** The saga scope `s-<n>` an activity or script is scheduled in. */
  sagaScope?: string;
  /** Made by the SDK itself rather than by one of the function's calls (Requirement 044-012). */
  issuedBySdk: boolean;
  /** The value must stay inline (a gated activity input, an approval subject, the declared state, continue-as-new). */
  alwaysInline: boolean;
}

/** Why a workflow task fails (Technical Plan 6.3, the causes the SDK raises). */
export type FailureCause = 'NONDETERMINISM' | 'WORKFLOW_TASK_FAILED' | 'ACTIVATION_DEADLINE' | 'VALIDATOR_COMMAND' | 'NONDETERMINISM_RISK';

export interface WorkflowTaskFailure {
  cause: FailureCause;
  commandNumber?: number;
  actualKind?: string;
  actualTarget?: string;
  /** For an activity or a script: the saga scope the function scheduled it in, empty outside any (Decision 044-244). */
  actualSagaScope?: string;
  exceptionType?: string;
}

export interface LogLine { level: 'debug' | 'info' | 'warn' | 'error'; message: string }

export interface ActivationResult {
  commands: WorkflowCommand[];
  failure?: WorkflowTaskFailure;
  /** A replay read a redacted value: the verdict is `INCONCLUSIVE_REDACTED` at this command number. */
  inconclusiveAt?: number;
  logs: LogLine[];
  /** The function or a handler is still running (false once the run has a terminal command). */
  open: boolean;
}

/** A command record the replay compares against, with a side effect's recorded value. */
export interface ExpectedRecord { commandNumber: number; kind: CommandKind; target: string; sagaScope?: string; value?: unknown; redacted?: boolean }

export interface BeginActivation {
  replaying: boolean;
  /** The time of the activation's last `workflow_task.started`, in microseconds since the epoch. */
  atUs: number;
  continueAsNewSuggested: boolean;
  historyLimitReached: boolean;
  /** Replay only: the records of the completion this activation produced. */
  expected?: ExpectedRecord[];
  /** Replay only: the deferral that completion recorded. */
  expectedDeferral?: { kind: CommandKind; target: string };
  /** The signals of this activation's window (sequence and name), for the consumption reserve (Task 044-37). */
  windowSignals?: { seq: number; name: string }[];
}

export interface RunInit {
  workflowType: string;
  /** The run's 32-byte seed as 64 hexadecimal characters. */
  seed: string;
  /** The declared state's `maxBytes` of the run's current version (0 without declared state). */
  stateMaxBytes: number;
  /** The activity names of the current version whose envelope entry requires approval. */
  gatedEntries: string[];
}

// ---------------------------------------------------------------------------------------------------------------------
// Errors workflow code sees.

/** A workflow call made outside a workflow function (in the host, or at a module's top level). */
export class NotInWorkflow extends Error {
  constructor(what: string) { super(`${what} is available only inside a workflow function`); this.name = 'NotInWorkflow'; }
}

/** Ends the run `FAILED` with a code (`^[A-Z][A-Z0-9_]{0,63}$`, otherwise `WORKFLOW_FAILED`) and a message kept as `pii`. */
export class WorkflowFailure extends Error {
  readonly code: string;
  readonly safeMessage: string;
  constructor(code: string, safeMessage: string) {
    super(`${code}: ${safeMessage}`);
    this.name = 'WorkflowFailure';
    this.code = FAILURE_CODE.test(code) ? code : 'WORKFLOW_FAILED';
    this.safeMessage = String(safeMessage).slice(0, MAX_FAILURE_MESSAGE);
  }
}

/** Thrown at the call that would make the activation's 257th command or cross its 64 KiB of command metadata. */
export class CommandBatchLimitExceeded extends Error {
  constructor(message: string) { super(message); this.name = 'CommandBatchLimitExceeded'; }
}

/** Thrown at the call after which the activation's values could no longer be carried by one completion. */
export class InlineValuesExceeded extends Error {
  constructor(message: string) { super(message); this.name = 'InlineValuesExceeded'; }
}

/** An activity or script that ended failed or timed out (a cancelled one rejects with `CommandCancelled`). */
export class ActivityError extends Error {
  readonly status: 'failed' | 'timed_out';
  readonly code: string | undefined;
  constructor(status: 'failed' | 'timed_out', code: string | undefined, message: string | undefined) {
    super(message ?? `the activity ended ${status}${code ? ` (${code})` : ''}`);
    this.name = 'ActivityError';
    this.status = status;
    this.code = code;
  }
}

/** A child run that did not complete (`partial`: it ended with a saga only partly compensated, Decision 044-321). */
export class ChildWorkflowError extends Error {
  readonly status: 'failed' | 'cancelled' | 'timed_out' | 'partial';
  readonly code: string | undefined;
  constructor(status: 'failed' | 'cancelled' | 'timed_out' | 'partial', code: string | undefined) {
    super(`the child run ended ${status}${code ? ` (${code})` : ''}`);
    this.name = 'ChildWorkflowError';
    this.status = status;
    this.code = code;
  }
}

/** A durable timer whose cancellation the function requested. */
export class TimerCancelled extends Error {
  constructor() { super('the timer was cancelled'); this.name = 'TimerCancelled'; }
}

/** An activity, a script, a wait or an approval that ended cancelled (its outcome in the history, Decision 044-248). */
export class CommandCancelled extends Error {
  constructor() { super('the command was cancelled'); this.name = 'CommandCancelled'; }
}

/** The saga scope was aborted and every compensation succeeded. */
export class SagaCompensated extends Error {
  constructor(cause: unknown) { super('the saga was aborted and compensated', { cause }); this.name = 'SagaCompensated'; }
}
/** The saga scope was aborted and some members were not compensated (feature 003's `PARTIAL`). */
export class SagaPartial extends Error {
  constructor(cause: unknown) { super('the saga was aborted and partly compensated', { cause }); this.name = 'SagaPartial'; }
}
/** The saga scope was aborted and a compensation failed. */
export class SagaCompensationFailed extends Error {
  constructor(cause: unknown) { super('the saga was aborted and a compensation failed', { cause }); this.name = 'SagaCompensationFailed'; }
}

// Control flow that ends the function: never seen by workflow code unless it catches everything.
class ContinueAsNewRequest extends Error {
  readonly body: Record<string, unknown>;
  constructor(body: Record<string, unknown>) { super('continue as new'); this.name = 'ContinueAsNew'; this.body = body; }
}
class CancellationAcknowledged extends Error {
  constructor() { super('cancellation acknowledged'); this.name = 'CancellationAcknowledged'; }
}
/** Thrown into the function once the activation has failed, so that it stops making commands. */
class ActivationStopped extends Error {
  constructor() { super('the activation has stopped'); this.name = 'ActivationStopped'; }
}

// ---------------------------------------------------------------------------------------------------------------------
// RFC 8785 canonical JSON and byte sizes, without any host API.

/** The RFC 8785 text of a JSON value: keys sorted by UTF-16 code unit, ECMAScript number printing, no whitespace. */
export function rfc8785(value: unknown): string {
  return canonical(value === undefined ? null : value, '$', 0);
}

function canonical(value: unknown, path: string, depth: number): string {
  if (depth > 256) throw new TypeError(`${path} is nested more than 256 levels`);
  if (value === null) return 'null';
  switch (typeof value) {
    case 'boolean': return value ? 'true' : 'false';
    case 'number':
      if (!Number.isFinite(value)) throw new TypeError(`${path} is not a finite number`);
      return JSON.stringify(value);
    case 'string':
      checkWellFormed(value, path);
      return JSON.stringify(value);
    case 'bigint': throw new TypeError(`${path} is a bigint: write it as a string`);
    case 'object': {
      const withJson = value as { toJSON?: () => unknown };
      if (typeof withJson.toJSON === 'function') return canonical(withJson.toJSON(), path, depth + 1);
      if (Array.isArray(value)) {
        const items: string[] = [];
        for (let i = 0; i < value.length; i++) {
          const item: unknown = value[i];
          items.push(item === undefined || typeof item === 'function' || typeof item === 'symbol' ? 'null' : canonical(item, `${path}[${i}]`, depth + 1));
        }
        return `[${items.join(',')}]`;
      }
      const record = value as Record<string, unknown>;
      const keys = Object.keys(record).filter((key) => {
        const item = record[key];
        return item !== undefined && typeof item !== 'function' && typeof item !== 'symbol';
      }).sort();
      const parts: string[] = [];
      for (const key of keys) {
        checkWellFormed(key, path);
        parts.push(`${JSON.stringify(key)}:${canonical(record[key], `${path}.${key}`, depth + 1)}`);
      }
      return `{${parts.join(',')}}`;
    }
    default: throw new TypeError(`${path} is not a JSON value`);
  }
}

function checkWellFormed(text: string, path: string): void {
  for (let i = 0; i < text.length; i++) {
    const unit = text.charCodeAt(i);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = text.charCodeAt(i + 1);
      if (next >= 0xdc00 && next <= 0xdfff) { i++; continue; }
      throw new TypeError(`${path} holds a lone surrogate`);
    }
    if (unit >= 0xdc00 && unit <= 0xdfff) throw new TypeError(`${path} holds a lone surrogate`);
  }
}

/** The number of bytes of a string encoded as UTF-8 (a well-formed string: lone surrogates are refused before). */
export function utf8Length(text: string): number {
  let bytes = 0;
  for (let i = 0; i < text.length; i++) {
    const unit = text.charCodeAt(i);
    if (unit < 0x80) bytes += 1;
    else if (unit < 0x800) bytes += 2;
    else if (unit >= 0xd800 && unit <= 0xdbff) { bytes += 4; i++; }
    else bytes += 3;
  }
  return bytes;
}

/** The size in bytes of a value's RFC 8785 text. */
export function canonicalSize(value: unknown): number {
  return utf8Length(rfc8785(value));
}

/**
 * A value as the call made it: its RFC 8785 text parsed again, with its size. Later changes of the caller's object, a
 * `toJSON` that answers differently, or a getter, cannot change what the command carries or what its bounds checked.
 */
export function snapshot(value: unknown): { value: unknown; size: number } {
  const text = rfc8785(value === undefined ? null : value);
  return { value: JSON.parse(text) as unknown, size: utf8Length(text) };
}

// ---------------------------------------------------------------------------------------------------------------------
// The deterministic generator (Technical Plan 1.8): xoshiro256** over the run's seed.

const MASK64 = (1n << 64n) - 1n;

function rotl(x: bigint, k: bigint): bigint {
  return ((x << k) | (x >> (64n - k))) & MASK64;
}

/** SplitMix64 from a start value: the four words that replace an all-zero seed. */
export function splitMix64Words(start: bigint, count: number): bigint[] {
  let state = start & MASK64;
  const words: bigint[] = [];
  for (let i = 0; i < count; i++) {
    state = (state + 0x9e3779b97f4a7c15n) & MASK64;
    let z = state;
    z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & MASK64;
    z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & MASK64;
    words.push(z ^ (z >> 31n));
  }
  return words;
}

/** xoshiro256** whose four state words are the seed read as four little-endian unsigned 64-bit integers. */
export class Xoshiro256 {
  #s: [bigint, bigint, bigint, bigint];

  constructor(seedHex: string) {
    if (!/^[0-9a-f]{64}$/.test(seedHex)) throw new TypeError('a seed is 32 bytes as 64 lowercase hexadecimal characters');
    const words: bigint[] = [];
    for (let w = 0; w < 4; w++) {
      let word = 0n;
      for (let b = 7; b >= 0; b--) word = (word << 8n) | BigInt(parseInt(seedHex.slice((w * 8 + b) * 2, (w * 8 + b) * 2 + 2), 16));
      words.push(word);
    }
    const all = words.every((word) => word === 0n) ? splitMix64Words(0n, 4) : words;
    this.#s = [all[0]!, all[1]!, all[2]!, all[3]!];
  }

  /** The generator's state, to put back after a side effect computed with it (a replay does not compute it). */
  save(): [bigint, bigint, bigint, bigint] { return [...this.#s]; }
  restore(state: [bigint, bigint, bigint, bigint]): void { this.#s = [...state]; }

  /** The next 64-bit output. */
  next(): bigint {
    const s = this.#s;
    const result = (rotl((s[1] * 5n) & MASK64, 7n) * 9n) & MASK64;
    const t = (s[1] << 17n) & MASK64;
    s[2] ^= s[0];
    s[3] ^= s[1];
    s[1] ^= s[2];
    s[0] ^= s[3];
    s[2] ^= t;
    s[3] = rotl(s[3], 45n);
    return result;
  }

  /** A number in [0, 1): `(next() >> 11) × 2^-53`. */
  random(): number {
    return Number(this.next() >> 11n) * 2 ** -53;
  }

  /** An integer in [0, n) for 0 < n ≤ 2^32, without bias: draws until `x ≥ (2^64 − n) mod n`. */
  randomInt(n: number): number {
    if (!Number.isInteger(n) || n <= 0 || n > 2 ** 32) throw new RangeError('randomInt takes an integer n with 0 < n ≤ 2^32');
    const bound = BigInt(n);
    const threshold = ((1n << 64n) - bound) % bound;
    for (;;) {
      const x = this.next();
      if (x >= threshold) return Number(x % bound);
    }
  }

  /** A version 4 UUID from 16 bytes of two outputs, little-endian. */
  uuid(): string {
    const bytes: number[] = [];
    for (let i = 0; i < 2; i++) {
      let word = this.next();
      for (let b = 0; b < 8; b++) { bytes.push(Number(word & 0xffn)); word >>= 8n; }
    }
    bytes[6] = (bytes[6]! & 0x0f) | 0x40;
    bytes[8] = (bytes[8]! & 0x3f) | 0x80;
    const hex = bytes.map((b) => b.toString(16).padStart(2, '0')).join('');
    return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
  }
}

// ---------------------------------------------------------------------------------------------------------------------
// The workflow API.

export interface ActivityRetryPolicy {
  maxAttempts?: number;
  initialDelayMs?: number;
  maxDelayMs?: number;
  strategy?: 'fixed' | 'exponential';
  jitter?: boolean;
}

export interface ActivityOptions {
  retry?: ActivityRetryPolicy;
  timeoutMs?: number;
  concurrencyKey?: string;
  rateLimitKey?: string;
}

export interface ScriptOptions { retry?: ActivityRetryPolicy; timeoutMs?: number }
/** A wait is bounded, as a graph waitEvent node is: `timeoutMs` from 1 ms to 366 days (Decision 044-247). */
export interface WaitOptions { timeoutMs: number; lookbackMs?: number }
export interface ChildOptions { cancelPolicy?: 'propagate' | 'request_cancel' | 'detach'; businessId?: string }
export interface SagaOptions {
  onCompensationFailure?: 'manual_review' | 'fail';
  cancelPolicy?: 'compensate_completed' | 'wait_in_flight_then_compensate' | 'abort_no_compensation';
}
export interface ContinueAsNewOptions { version?: 'same' | 'active'; state?: unknown }
/** `id`: the side effect's marker identifier, `side-effect` by default (Decision 044-322). */
export interface SideEffectOptions { id?: string }

/** A promise of a command's outcome, which `requestCancellation` can name. */
export interface CommandPromise<T> extends Promise<T> { readonly commandNumber: number }

/** The scope of a saga opened from code: its activities are compensated by the engine when the scope aborts. */
export interface SagaScope {
  readonly scope: string;
  activity<T = unknown>(name: string, input?: unknown, options?: ActivityOptions): CommandPromise<T>;
  script<T = unknown>(artifactDigest: string, input?: unknown, options?: ScriptOptions): CommandPromise<T>;
  saga<T>(options: SagaOptions, body: (scope: SagaScope) => Promise<T>): Promise<T>;
  /** Aborts the scope: the engine compensates what succeeded, and the saga call throws once it settles. */
  abort(): never;
}

export interface Cancellation { readonly requested: boolean; readonly promise: Promise<void> }

/** Everything a workflow function may call; also passed to it as its first argument. */
export interface WorkflowContext {
  activity<T = unknown>(name: string, input?: unknown, options?: ActivityOptions): CommandPromise<T>;
  script<T = unknown>(artifactDigest: string, input?: unknown, options?: ScriptOptions): CommandPromise<T>;
  sleep(durationMs: number): CommandPromise<void>;
  waitForEvent<T = unknown>(eventType: string, correlationKey: string, options: WaitOptions): CommandPromise<T | undefined>;
  child<T = unknown>(workflowId: string, input?: unknown, options?: ChildOptions): CommandPromise<T>;
  approval(policyName: string, subject: unknown): CommandPromise<'approved' | 'rejected' | 'expired'>;
  saga<T>(options: SagaOptions, body: (scope: SagaScope) => Promise<T>): Promise<T>;
  requestCancellation(command: CommandPromise<unknown>): void;
  onSignal(name: string, handler: (payload: unknown) => void | Promise<void>): void;
  signal<T = unknown>(name: string): Promise<T>;
  onUpdate(name: string, validator: (args: unknown) => void, handler: (args: unknown) => Promise<unknown>): void;
  setState(state: unknown): void;
  patched(id: string): boolean;
  deprecatePatch(id: string): void;
  sideEffect<T>(compute: () => T, options?: SideEffectOptions): T;
  now(): Date;
  random(): number;
  randomInt(n: number): number;
  uuid(): string;
  continueAsNew(input: unknown, options?: ContinueAsNewOptions): never;
  readonly cancellation: Cancellation;
  acknowledgeCancellation(): never;
  continueAsNewSuggested(): boolean;
  historyLimitReached(): boolean;
  /** The state a predecessor carried by continue-as-new into this run, or undefined. */
  carriedState(): unknown;
}

export type WorkflowFunction = (ctx: WorkflowContext, input: unknown) => Promise<unknown>;

// ---------------------------------------------------------------------------------------------------------------------
// The run's state inside its context.

interface Pending { settle(event: HistoryEvent): void; scope?: Scope }
interface Timer { callback: () => void; intervalMs?: number; handle: number; onCancel?: () => void }
interface Scope { id: string; commandNumber: number; parent: Scope | undefined; members: Set<Promise<unknown>>; state: 'open' | 'closed' | 'aborting'; settle?: (outcome: string) => void }
interface BufferedSignal { seq: number; name: string; payload: unknown; redacted: boolean }
interface Terminal { command: WorkflowCommand; deferred: boolean }

class Run {
  readonly workflowType: string;
  readonly fn: WorkflowFunction;
  readonly rng: Xoshiro256;
  readonly stateMaxBytes: number;
  readonly gated: Set<string>;

  // Time and flags of the current activation.
  nowMs = 0;
  casSuggested = false;
  historyLimit = false;

  // Numbering and the activation in progress.
  accepted = 0;
  next = 1;
  activating = false;
  replaying = false;
  expected = new Map<number, ExpectedRecord>();
  expectedDeferral: { kind: CommandKind; target: string } | undefined;
  commands: WorkflowCommand[] = [];
  functionCommands = 0;
  functionMetadata = 0;
  failure: WorkflowTaskFailure | undefined;
  inconclusiveAt: number | undefined;
  /** A withheld value reached the function in some activation of this replay: it may shape every later command. */
  sawRedacted = false;
  inSideEffect = false;
  scopeCommands = 0;
  sdkCommands = 0;
  timerNumbers = new Set<number>();
  carried: unknown;
  logs: LogLine[] = [];
  logBytes = 0;
  inValidator = false;

  // Inline budget of the activation (Non-functional requirement 044-006, Tasks 044-36 and 044-37).
  reserve = 0;
  alwaysInlineBytes = 0;
  blobValueSizes: number[] = [];

  // Outstanding commands.
  pending = new Map<number, Pending>();
  timers = new Map<number, Timer>();
  timerOfHandle = new Map<number, number>();
  nextHandle = 1;
  settled = new Set<number>();

  // Signals, updates, state, cancellation, patches, sagas.
  signalHandlers = new Map<string, (payload: unknown) => void | Promise<void>>();
  signalWaiters = new Map<string, ((payload: unknown) => void)[]>();
  buffered: BufferedSignal[] = [];
  consumed = new Map<string, number[]>();
  updateHandlers = new Map<string, { validator: (args: unknown) => void; handler: (args: unknown) => Promise<unknown> }>();
  stateValue: { value: unknown } | undefined;
  cancelRequested = false;
  cancelResolve: (() => void) | undefined;
  readonly cancelPromise: Promise<void>;
  patches = new Map<string, boolean>();
  scopes = new Map<string, Scope>();

  // The function's outcome.
  started = false;
  mainOutcome: { kind: 'complete'; output: unknown } | { kind: 'error'; error: unknown } | undefined;
  terminal: Terminal | undefined;
  closed = false;

  constructor(init: RunInit, fn: WorkflowFunction) {
    this.workflowType = init.workflowType;
    this.fn = fn;
    this.rng = new Xoshiro256(init.seed);
    this.stateMaxBytes = init.stateMaxBytes;
    this.gated = new Set(init.gatedEntries);
    this.cancelPromise = new Promise<void>((resolve) => { this.cancelResolve = resolve; });
  }

  // --- the activation lifecycle ----------------------------------------------------------------------------------

  begin(input: BeginActivation): void {
    this.activating = true;
    this.replaying = input.replaying;
    this.nowMs = Math.trunc(input.atUs / 1000);
    this.casSuggested = input.continueAsNewSuggested;
    this.historyLimit = input.historyLimitReached;
    this.expected = new Map((input.expected ?? []).map((record) => [record.commandNumber, record]));
    this.expectedDeferral = input.expectedDeferral;
    this.commands = [];
    this.functionCommands = 0;
    this.functionMetadata = 0;
    this.failure = undefined;
    this.inconclusiveAt = undefined;
    this.scopeCommands = 0;
    this.sdkCommands = 0;
    this.logs = [];
    this.logBytes = 0;
    this.next = this.accepted + 1;
    this.alwaysInlineBytes = 0;
    this.blobValueSizes = [];
    this.consumed = new Map();
    this.stateValue = undefined;
    // The reserve is known at the start from the history: the declared state's maximum and the consumption list of every
    // signal that could be consumed in this activation, so a gated call throws at the same point live and in replay.
    const pending = new Map<string, number[]>();
    for (const signal of this.buffered) pending.set(signal.name, [...(pending.get(signal.name) ?? []), signal.seq]);
    for (const signal of input.windowSignals ?? []) pending.set(signal.name, [...(pending.get(signal.name) ?? []), signal.seq]);
    this.reserve = this.stateMaxBytes + (pending.size === 0 ? 0 : canonicalSize(consumptionBody(pending)));
  }

  apply(event: HistoryEvent): void {
    if (event.redacted) this.sawRedacted = true;
    switch (event.kind) {
      case 'run.started':
        if ('carriedState' in event && event.carriedState !== undefined) this.carried = event.carriedState;
        return this.startFunction(event.input);
      case 'step.outcome': case 'wait.outcome': case 'child.finished': case 'approval.decided':
        return this.settle(event.commandNumber, event);
      case 'timer.fired': return this.fire(event.commandNumber);
      case 'saga.outcome': {
        const scope = this.scopes.get(event.scope);
        if (scope?.settle) scope.settle(event.outcome);
        return;
      }
      case 'signal.buffered': return this.deliverSignal({ seq: event.seq, name: event.name, payload: event.payload, redacted: event.redacted === true });
      case 'update.admitted': return this.admitUpdate(event.seq, event.name, event.args);
      case 'run.cancel_requested':
        if (!this.cancelRequested) { this.cancelRequested = true; this.cancelResolve?.(); }
        return;
      case 'command.record': case 'marker.recorded': case 'workflow_task.started': case 'workflow_task.completed':
        return;
    }
  }

  end(): ActivationResult {
    if (!this.failure) {
      try { this.finishActivation(); } catch (error) { if (!(error instanceof ActivationStopped)) this.failTask(error); }
    }
    const result: ActivationResult = {
      commands: this.failure ? [] : this.commands,
      logs: this.replaying ? [] : this.logs,
      open: !this.terminal,
    };
    if (this.failure) result.failure = this.failure;
    if (this.inconclusiveAt !== undefined) result.inconclusiveAt = this.inconclusiveAt;
    this.activating = false;
    return result;
  }

  /** The host's answer to the completion: its numbered commands were accepted, the terminal one perhaps deferred. */
  commit(deferred: boolean): void {
    let count = 0;
    for (const command of this.commands) if (command.commandNumber !== undefined) count++;
    if (this.terminal && TERMINAL_KINDS.includes(this.terminal.command.kind) && this.commands.includes(this.terminal.command)) {
      if (deferred) {
        if (this.terminal.command.commandNumber !== undefined) count--;
        this.terminal.deferred = true;
      } else {
        if (this.terminal.command.commandNumber === undefined) count++;
        this.terminal.deferred = false;
        this.closed = true;
      }
    }
    this.accepted += count;
    for (const [, scope] of this.scopes) if (scope.state === 'closed') this.scopes.delete(scope.id);
  }

  private finishActivation(): void {
    // A terminal outcome of the function becomes its command at the end, after every event of the window was applied.
    if (!this.terminal && this.mainOutcome) {
      const outcome = this.mainOutcome;
      this.mainOutcome = undefined;
      if (outcome.kind === 'error' && !isTerminalError(outcome.error)) return this.failTask(outcome.error);
      this.abortOpenScopes();
      const command = this.terminalCommand(outcome);
      if (!command) return;
      this.terminal = { command, deferred: false };
    }
    if (this.consumed.size > 0) {
      this.issue('consume_signals', '', consumptionBody(this.consumed), { sdk: true });
    }
    if (this.stateValue) this.issue('set_state', '', { state: this.stateValue.value }, { sdk: true, alwaysInline: true });
    if (this.terminal && !this.closed) {
      const command = this.terminal.command;
      if (this.terminal.deferred) {
        // A deferred terminal command is re-emitted last at each later activation, live or replayed (4.4).
        this.emitTerminal({ ...command, issuedBySdk: true });
      } else if (!this.commands.includes(command)) {
        this.emitTerminal(command);
      }
    }
    if (this.replaying) this.checkReplayEnd();
  }

  private emitTerminal(command: WorkflowCommand): void {
    const number = this.next;
    if (this.replaying) {
      const record = this.expected.get(number);
      if (record) {
        if (record.kind !== command.kind || record.target !== command.target) this.mismatch(number, command.kind, command.target);
      } else if (this.expectedDeferral) {
        if (this.expectedDeferral.kind !== command.kind || this.expectedDeferral.target !== command.target) {
          this.mismatch(number, command.kind, command.target);
        }
        this.expectedDeferral = undefined;
        delete command.commandNumber;
        this.commands.push(command);
        if (this.terminal) this.terminal.command = command;
        return;
      } else {
        this.mismatch(number, command.kind, command.target);
      }
    }
    if (this.terminal?.deferred) {
      // The server numbers a re-emitted command when it accepts it; replay numbers it from its record.
      if (this.replaying && this.expected.has(number)) command.commandNumber = number;
      else delete command.commandNumber;
    } else {
      command.commandNumber = number;
    }
    if (command.commandNumber !== undefined) this.next++;
    this.commands.push(command);
    if (this.terminal) this.terminal.command = command;
  }

  private checkReplayEnd(): void {
    if (this.failure) return;
    for (let number = this.next; this.expected.has(number); number++) {
      // A recorded command the function no longer makes.
      return this.mismatch(number, '', '');
    }
    if (this.expectedDeferral) this.mismatch(this.next, '', '');
  }

  private terminalCommand(outcome: { kind: 'complete'; output: unknown } | { kind: 'error'; error: unknown }): WorkflowCommand | undefined {
    if (outcome.kind === 'complete') {
      const { value: output, size } = snapshot(outcome.output);
      if (!this.admitBlobValue(size)) {
        this.failTask(new InlineValuesExceeded('the output of the workflow cannot be carried by one completion'));
        return undefined;
      }
      return command('complete', '', { output }, false, false);
    }
    const error = outcome.error;
    if (error instanceof WorkflowFailure) {
      // The message is an inline value of the completion (FailCommand.safe_message).
      if (!this.blobsFit(this.blobValueSizes, this.alwaysInlineBytes + canonicalSize(error.safeMessage))) {
        this.failTask(new InlineValuesExceeded('the failure message cannot be carried by one completion'));
        return undefined;
      }
      return command('fail', '', { code: error.code, message: error.safeMessage }, false, false);
    }
    if (error instanceof ContinueAsNewRequest) return command('continue_as_new', '', error.body, false, true);
    return command('acknowledge_cancellation', '', {}, false, false);
  }

  private abortOpenScopes(): void {
    const open = [...this.scopes.values()].filter((scope) => scope.state === 'open');
    open.sort((a, b) => b.commandNumber - a.commandNumber);
    for (const scope of open) {
      if (scope.state !== 'open') continue;
      this.abortRegion(scope);
      this.issue('saga_abort', scope.id, { scope: scope.id }, { sdk: true });
    }
  }

  /** Aborting a scope aborts its open nested scopes too: the engine aborts the whole region (Technical Plan 5.1). */
  abortRegion(scope: Scope): void {
    scope.state = 'aborting';
    for (const other of this.scopes.values()) {
      if (other.state !== 'open') continue;
      for (let parent = other.parent; parent; parent = parent.parent) {
        if (parent === scope) { other.state = 'aborting'; break; }
      }
    }
  }

  // --- making commands ---------------------------------------------------------------------------------------------

  issue(kind: CommandKind, target: string | ((n: number) => string),
    bodyOf: Record<string, unknown> | ((n: number) => Record<string, unknown>),
    options: { sdk?: boolean; alwaysInline?: boolean; scope?: string; values?: { size: number; inline: boolean; what: string }[] } = {}): number {
    // The body is taken as the host will hash it: a value nested too deep, or with no canonical form, throws here, at the
    // call, inside the context.
    const body = JSON.parse(rfc8785(typeof bodyOf === 'function' ? bodyOf(this.next) : bodyOf)) as Record<string, unknown>;
    if (!this.activating) throw new NotInWorkflow('a workflow command');
    if (this.failure) throw new ActivationStopped();
    this.guardEffects();
    const sdk = options.sdk === true;
    // The SDK's share holds one re-emitted terminal command besides these (Requirement 044-012).
    if (sdk && this.sdkCommands + 1 > MAX_SDK_COMMANDS - 1) throw new CommandBatchLimitExceeded(`the SDK may make at most ${MAX_SDK_COMMANDS} commands in an activation`);
    if (!sdk) {
      const metadata = metadataSize(kind, typeof target === 'string' ? target : target(this.next), body);
      if (this.functionCommands + 1 > MAX_FUNCTION_COMMANDS) {
        throw new CommandBatchLimitExceeded(`an activation may make at most ${MAX_FUNCTION_COMMANDS} commands`);
      }
      if (this.functionMetadata + metadata > MAX_FUNCTION_METADATA_BYTES) {
        throw new CommandBatchLimitExceeded(`an activation may make at most ${MAX_FUNCTION_METADATA_BYTES} bytes of command metadata`);
      }
    }
    // The values are judged after the command bounds, and recorded only once the call cannot throw any more.
    let alwaysInline = this.alwaysInlineBytes;
    const blobSizes = [...this.blobValueSizes];
    for (const value of options.values ?? []) {
      if (value.inline) {
        if (alwaysInline + value.size > MAX_INLINE_BYTES - this.reserve || !this.blobsFit(blobSizes, alwaysInline + value.size)) {
          throw new InlineValuesExceeded(`${value.what} of ${value.size} bytes cannot be carried inline by this activation's completion`);
        }
        alwaysInline += value.size;
      } else {
        if (!this.blobsFit([...blobSizes, value.size], alwaysInline)) throw new InlineValuesExceeded(`${value.what} cannot be carried by this activation's completion`);
        blobSizes.push(value.size);
      }
    }
    const number = this.next++;
    const resolved = typeof target === 'string' ? target : target(number);
    if (this.replaying) {
      // An activity's or a script's saga scope is compared with the kind and the target (Decision 044-244).
      const record = this.expected.get(number);
      const scoped = kind === 'activity' || kind === 'script';
      if (!record || record.kind !== kind || record.target !== resolved || (scoped && (record.sagaScope ?? '') !== (options.scope ?? ''))) {
        this.mismatch(number, kind, resolved, options.scope ?? '');
        throw new ActivationStopped();
      }
    }
    const made = command(kind, resolved, body, sdk, options.alwaysInline === true, number);
    if (options.scope !== undefined) made.sagaScope = options.scope;
    this.commands.push(made);
    if (sdk) this.sdkCommands++;
    else {
      this.functionCommands++;
      this.functionMetadata += metadataSize(kind, resolved, body);
    }
    this.alwaysInlineBytes = alwaysInline;
    this.blobValueSizes = blobSizes;
    return number;
  }

  /**
   * A replay that differs from its record. When the activation was given a redacted value (an exported history), the
   * difference may come from the value the export withheld: the verdict is then inconclusive, never a false mismatch.
   */
  mismatch(commandNumber: number, actualKind: string, actualTarget: string, actualSagaScope = ''): void {
    if (this.failure || this.inconclusiveAt !== undefined) return;
    const failure: WorkflowTaskFailure = { cause: 'NONDETERMINISM', commandNumber, actualKind, actualTarget };
    if (actualKind === 'activity' || actualKind === 'script') failure.actualSagaScope = actualSagaScope;
    if (this.sawRedacted) this.inconclusiveAt = commandNumber;
    this.failure = failure;
  }

  failTask(error: unknown): void {
    if (this.failure) return;
    const failure: WorkflowTaskFailure = { cause: 'WORKFLOW_TASK_FAILED' };
    const type = typeNameOf(error);
    if (type) failure.exceptionType = type;
    this.failure = failure;
  }

  /**
   * What a validator or a side effect's computation may not do: a validator that makes a command, consumes a signal or
   * sets the state fails the workflow task `VALIDATOR_COMMAND`; a side effect's computation makes no command at all
   * (a replay does not run it).
   */
  guardEffects(): void {
    if (this.inValidator) {
      this.failure = { cause: 'VALIDATOR_COMMAND' };
      throw new ActivationStopped();
    }
    if (this.inSideEffect) throw new TypeError('a side effect\'s computation makes no workflow command: a replay does not run it');
  }

  /** Whether one more value that may travel as a blob still lets the activation's values be carried (nothing recorded). */
  fitsBlob(size: number): boolean {
    return this.blobsFit([...this.blobValueSizes, size], this.alwaysInlineBytes);
  }

  /** The inline check of a value that must stay inline: throws at the call that would cross 192 KiB less the reserve. */
  admitInlineValue(size: number, what: string): void {
    if (this.alwaysInlineBytes + size > MAX_INLINE_BYTES - this.reserve || !this.blobsFit(this.blobValueSizes, this.alwaysInlineBytes + size)) {
      throw new InlineValuesExceeded(`${what} of ${size} bytes cannot be carried inline by this activation's completion`);
    }
    this.alwaysInlineBytes += size;
  }

  /** Whether a value that may travel as a blob still lets the activation's values be carried; records it if so. */
  admitBlobValue(size: number): boolean {
    const sizes = [...this.blobValueSizes, size];
    if (!this.blobsFit(sizes, this.alwaysInlineBytes)) return false;
    this.blobValueSizes = sizes;
    return true;
  }

  blobsFit(sizes: number[], alwaysInline: number): boolean {
    const room = MAX_INLINE_BYTES - this.reserve - alwaysInline;
    if (room < 0) return false;
    const sorted = [...sizes].sort((a, b) => b - a);
    let inline = sorted.reduce((sum, size) => sum + size, 0);
    let blobs = 0;
    let blobBytes = 0;
    while (inline > room) {
      const largest = sorted[blobs];
      if (largest === undefined) return false;
      inline -= largest;
      blobBytes += largest;
      blobs++;
    }
    return blobs <= MAX_BLOBS && blobBytes <= MAX_BLOB_CARRIED_BYTES;
  }

  // --- the function ------------------------------------------------------------------------------------------------

  private startFunction(input: unknown): void {
    if (this.started) return;
    this.started = true;
    let promise: Promise<unknown>;
    try {
      promise = Promise.resolve(this.fn(api, input));
    } catch (error) {
      promise = Promise.reject(error);
    }
    promise.then(
      (output) => {
        if (this.mainOutcome || this.terminal) return;
        // The output as the function returned it, not as it may be changed before the activation ends.
        try { this.mainOutcome = { kind: 'complete', output: snapshot(output).value }; } catch (error) { this.mainOutcome = { kind: 'error', error }; }
      },
      (error: unknown) => { if (!this.mainOutcome && !this.terminal) this.mainOutcome = { kind: 'error', error }; },
    );
  }

  private settle(commandNumber: number, event: HistoryEvent): void {
    const pending = this.pending.get(commandNumber);
    if (!pending) return;
    this.pending.delete(commandNumber);
    this.settled.add(commandNumber);
    pending.settle(event);
  }

  private fire(commandNumber: number): void {
    const timer = this.timers.get(commandNumber);
    this.settle(commandNumber, { seq: 0, kind: 'timer.fired', commandNumber });
    if (!timer) return;
    this.timers.delete(commandNumber);
    this.timerOfHandle.delete(timer.handle);
    if (timer.intervalMs !== undefined) this.startTimer(timer.intervalMs, timer.callback, timer.handle, timer.intervalMs);
    timer.callback();
  }

  startTimer(durationMs: number, callback: () => void, handle: number, intervalMs?: number): number {
    const duration = Math.max(1, Math.ceil(Number.isFinite(durationMs) ? durationMs : 1));
    const number = this.issue('timer_start', (n) => `c-${n}`, { type: 'timer', durationMs: duration });
    this.timerNumbers.add(number);
    const timer: Timer = { callback, handle };
    if (intervalMs !== undefined) timer.intervalMs = intervalMs;
    this.timers.set(number, timer);
    this.timerOfHandle.set(handle, number);
    return number;
  }

  cancelTimer(handle: unknown): void {
    if (typeof handle !== 'number') return;
    const number = this.timerOfHandle.get(handle);
    if (number === undefined) return;
    this.timerOfHandle.delete(handle);
    this.timers.delete(number);
    this.issue('timer_cancel', `c-${number}`, { target: `c-${number}` });
  }

  // --- signals and updates -----------------------------------------------------------------------------------------

  private deliverSignal(signal: BufferedSignal): void {
    if (this.takeSignal(signal)) return;
    this.buffered.push(signal);
  }

  /** Gives a signal to its handler or its first waiter, recording its consumption; false when nobody takes it. */
  takeSignal(signal: BufferedSignal): boolean {
    const handler = this.signalHandlers.get(signal.name);
    const waiters = this.signalWaiters.get(signal.name);
    if (!handler && (!waiters || waiters.length === 0)) return false;
    this.guardEffects();
    if (signal.redacted) this.sawRedacted = true;
    this.consumed.set(signal.name, [...(this.consumed.get(signal.name) ?? []), signal.seq]);
    if (handler) {
      this.runHandler(() => handler(signal.payload));
    } else {
      waiters!.shift()!(signal.payload);
    }
    return true;
  }

  // A handler may itself wait for or register a signal, which drains the buffer again: each signal is taken out of the
  // buffer before it is given, so no signal is given twice.
  drainBuffered(name: string): void {
    for (;;) {
      const index = this.buffered.findIndex((signal) => signal.name === name);
      if (index < 0) return;
      const signal = this.buffered[index]!;
      if (!this.signalHandlers.has(name) && (this.signalWaiters.get(name)?.length ?? 0) === 0) return;
      this.buffered.splice(index, 1);
      this.takeSignal(signal);
    }
  }

  runHandler(handler: () => unknown): void {
    let result: unknown;
    try {
      result = handler();
    } catch (error) {
      if (!(error instanceof ActivationStopped)) this.failTask(error);
      return;
    }
    if (result && typeof (result as Promise<unknown>).then === 'function') {
      (result as Promise<unknown>).then(undefined, (error: unknown) => {
        if (error instanceof ActivationStopped) return;
        if (isTerminalError(error)) { if (!this.mainOutcome && !this.terminal) this.mainOutcome = { kind: 'error', error }; return; }
        this.failTask(error);
      });
    }
  }

  private admitUpdate(seq: number, name: string, args: unknown): void {
    const target = String(seq);
    const entry = this.updateHandlers.get(name);
    if (!entry) {
      this.issue('update_reject', target, { admittedSeq: seq, accepted: false, reason: 'HANDLER_NOT_REGISTERED' }, { sdk: true });
      return;
    }
    this.inValidator = true;
    let rejected = false;
    try {
      const verdict = entry.validator(args) as unknown;
      // A validator runs at once, before the next event: one that answers a promise cannot be waited for, so it rejects.
      if (verdict !== null && typeof verdict === 'object' && typeof (verdict as { then?: unknown }).then === 'function') {
        (verdict as Promise<unknown>).then(undefined, () => undefined);
        rejected = true;
      }
    } catch (error) {
      if (error instanceof ActivationStopped || this.failure) { this.inValidator = false; throw new ActivationStopped(); }
      rejected = true;
    } finally {
      this.inValidator = false;
    }
    if (this.failure) throw new ActivationStopped();
    if (rejected) {
      this.issue('update_reject', target, { admittedSeq: seq, accepted: false, reason: 'VALIDATION_FAILED' }, { sdk: true });
      return;
    }
    this.issue('update_accept', target, { admittedSeq: seq, accepted: true }, { sdk: true });
    let promise: Promise<unknown>;
    try { promise = Promise.resolve(entry.handler(args)); } catch (error) { promise = Promise.reject(error); }
    promise.then(
      (result) => {
        const { value, size } = snapshot(result);
        if (!this.fitsBlob(size)) {
          this.issue('update_complete', target, { admittedSeq: seq, failureCode: 'RETAINED_BYTES_LIMIT' }, { sdk: true });
          return;
        }
        this.issue('update_complete', target, { admittedSeq: seq, result: value }, { sdk: true, values: [{ size, inline: false, what: 'the update result' }] });
      },
      (error: unknown) => {
        if (error instanceof ActivationStopped) return;
        // A terminal command made in a handler ends the run, as in a signal handler.
        if (error instanceof ContinueAsNewRequest || error instanceof CancellationAcknowledged) {
          if (!this.mainOutcome && !this.terminal) this.mainOutcome = { kind: 'error', error };
          return;
        }
        if (error instanceof WorkflowFailure) {
          this.issue('update_complete', target, { admittedSeq: seq, failureCode: error.code }, { sdk: true });
          return;
        }
        this.failTask(error);
      },
    ).then(undefined, (error: unknown) => { if (!(error instanceof ActivationStopped)) this.failTask(error); });
  }

  // --- the time and the log ----------------------------------------------------------------------------------------

  log(level: LogLine['level'], args: unknown[]): void {
    if (this.replaying || !this.activating) return;
    if (this.logs.length >= 1000 || this.logBytes >= 256 * 1024) return;
    const message = args.map(formatLogArgument).join(' ').slice(0, 8192);
    this.logBytes += message.length;
    this.logs.push({ level, message });
  }
}

function command(kind: CommandKind, target: string, body: Record<string, unknown>, sdk: boolean, alwaysInline: boolean,
  commandNumber?: number): WorkflowCommand {
  const made: WorkflowCommand = { kind, target, body, issuedBySdk: sdk, alwaysInline };
  if (commandNumber !== undefined) made.commandNumber = commandNumber;
  return made;
}

function consumptionBody(consumed: Map<string, number[]>): Record<string, unknown> {
  const names = [...consumed.keys()].sort();
  return { signals: names.map((name) => ({ name, seqs: [...consumed.get(name)!].sort((a, b) => a - b) })) };
}

// The literal values a body may hold, which the metadata bound does not count (a fragment's business literals and a
// typed command's values).
const LITERAL_FIELDS = ['input', 'correlationKey', 'subject', 'businessId', 'value', 'result', 'output', 'state', 'signals', 'message'];

/** The bytes a command counts against the 64 KiB of metadata: its kind, target and body without literal values. */
export function metadataSize(kind: CommandKind, target: string, body: Record<string, unknown>): number {
  const meta: Record<string, unknown> = {};
  for (const [key, item] of Object.entries(body)) {
    if (LITERAL_FIELDS.includes(key)) continue;
    meta[key] = item;
  }
  return canonicalSize({ kind, target, body: meta });
}

function isTerminalError(error: unknown): boolean {
  return error instanceof WorkflowFailure || error instanceof ContinueAsNewRequest || error instanceof CancellationAcknowledged;
}

function typeNameOf(error: unknown): string | undefined {
  let name: unknown;
  if (error && typeof error === 'object') {
    const constructorName = (error as { constructor?: { name?: unknown } }).constructor?.name;
    name = typeof constructorName === 'string' && constructorName !== '' ? constructorName : (error as { name?: unknown }).name;
  }
  return typeof name === 'string' && EXCEPTION_TYPE.test(name) ? name : undefined;
}

function formatLogArgument(value: unknown): string {
  if (typeof value === 'string') return value;
  if (value instanceof Error) return `${value.name}: ${value.message}`;
  try { return rfc8785(value); } catch { return String(value); }
}

// ---------------------------------------------------------------------------------------------------------------------
// The workflow API, bound to the run of the context.

let currentRun: Run | undefined;
const registered = new Map<string, WorkflowFunction>();

function run(what: string): Run {
  if (!currentRun || !currentRun.activating) throw new NotInWorkflow(what);
  return currentRun;
}

function commandPromise<T>(number: number, executor: (resolve: (value: T) => void, reject: (error: unknown) => void) => Pending,
  scope?: Scope): CommandPromise<T> {
  const r = currentRun!;
  let pending!: Pending;
  const promise = new Promise<T>((resolve, reject) => { pending = executor(resolve, reject); }) as CommandPromise<T>;
  Object.defineProperty(promise, 'commandNumber', { value: number, enumerable: false });
  if (scope) {
    pending.scope = scope;
    scope.members.add(promise);
    const forget = (): void => { scope.members.delete(promise); };
    promise.then(forget, forget);
  }
  r.pending.set(number, pending);
  return promise;
}

function stepOutcome<T>(resolve: (value: T) => void, reject: (error: unknown) => void): Pending {
  return {
    settle(event: HistoryEvent): void {
      if (event.kind !== 'step.outcome') return;
      if (event.status === 'completed') resolve(event.output as T);
      else if (event.status === 'cancelled') reject(new CommandCancelled());
      else reject(new ActivityError(event.status, event.errorCode, event.errorMessage));
    },
  };
}

function scheduleActivity<T>(name: string, input: unknown, options: ActivityOptions | undefined, scope: Scope | undefined): CommandPromise<T> {
  const r = run('activity');
  if (typeof name !== 'string' || name === '') throw new TypeError('an activity has a name');
  if (scope && scope.state !== 'open') throw new TypeError(`the saga scope ${scope.id} is not open`);
  if (options?.concurrencyKey !== undefined || options?.rateLimitKey !== undefined) {
    // The literal keys of feature 022 enter the code profile with Task 044-27.
    throw new TypeError('concurrencyKey and rateLimitKey wait for feature 022 in the code profile (design 044, Task 044-27)');
  }
  const { value, size } = snapshot(input);
  const fragment: Record<string, unknown> = { type: 'activity', activity: name, input: value };
  if (options?.retry) fragment['retry'] = { ...options.retry };
  if (options?.timeoutMs !== undefined) fragment['timeoutMs'] = options.timeoutMs;
  const gated = r.gated.has(name);
  // A gated activity's input is always inline; any other may travel as a blob.
  const values = [{ size, inline: gated, what: gated ? `the input of the gated activity ${name}` : `the input of ${name}` }];
  const number = r.issue('activity', name, fragment, scope ? { alwaysInline: gated, scope: scope.id, values } : { alwaysInline: gated, values });
  return commandPromise<T>(number, stepOutcome, scope);
}

function scheduleScript<T>(digest: string, input: unknown, options: ScriptOptions | undefined, scope: Scope | undefined): CommandPromise<T> {
  const r = run('script');
  if (scope && scope.state !== 'open') throw new TypeError(`the saga scope ${scope.id} is not open`);
  const { value, size } = snapshot(input);
  const fragment: Record<string, unknown> = { type: 'script', artifactDigest: digest, input: value };
  if (options?.retry) fragment['retry'] = { ...options.retry };
  // As for a graph script node; the artifact's policy (`wall`) bounds the attempt as well (Decision 044-247).
  if (options?.timeoutMs !== undefined) fragment['timeoutMs'] = checkedDuration(options.timeoutMs, 'timeoutMs', MAX_CODE_DURATION_MS);
  const values = [{ size, inline: false, what: `the input of the script ${digest}` }];
  const number = r.issue('script', digest, fragment, scope ? { scope: scope.id, values } : { values });
  return commandPromise<T>(number, stepOutcome, scope);
}

// The SDK's closes and aborts of scopes in one activation are bounded; the one beyond the bound throws into the saga.
function countScopeCommand(r: Run): void {
  if (r.scopeCommands + 1 > MAX_SCOPE_COMMANDS) {
    throw new CommandBatchLimitExceeded(`an activation may close or abort at most ${MAX_SCOPE_COMMANDS} saga scopes`);
  }
  r.scopeCommands++;
}

function openSaga<T>(options: SagaOptions, body: (scope: SagaScope) => Promise<T>, parent: Scope | undefined): Promise<T> {
  const r = run('saga');
  if (parent && parent.state !== 'open') throw new TypeError(`the saga scope ${parent.id} is not open`);
  if (options?.onCompensationFailure !== undefined && !['manual_review', 'fail'].includes(options.onCompensationFailure)) {
    throw new TypeError('onCompensationFailure is manual_review or fail');
  }
  if (options?.cancelPolicy !== undefined && !['compensate_completed', 'wait_in_flight_then_compensate', 'abort_no_compensation'].includes(options.cancelPolicy)) {
    throw new TypeError('cancelPolicy is compensate_completed, wait_in_flight_then_compensate or abort_no_compensation');
  }
  if ([...r.scopes.values()].filter((scope) => scope.state !== 'closed').length >= MAX_OPEN_SCOPES) {
    throw new CommandBatchLimitExceeded(`a run has at most ${MAX_OPEN_SCOPES} open saga scopes`);
  }
  const number = r.issue('saga_open', (n) => `s-${n}`, (n) => ({
    scope: `s-${n}`,
    parentScope: parent ? parent.id : null,
    onCompensationFailure: options?.onCompensationFailure ?? 'manual_review',
    cancelPolicy: options?.cancelPolicy ?? 'compensate_completed',
  }));
  const scope: Scope = { id: `s-${number}`, commandNumber: number, parent, members: new Set(), state: 'open' };
  r.scopes.set(scope.id, scope);
  const handle = sagaScopeOf(scope);
  const outcome = new Promise<string>((resolve) => { scope.settle = resolve; });
  const result = (async (): Promise<T> => {
    let value: T;
    try {
      value = await body(handle);
    } catch (error) {
      if (error instanceof ActivationStopped) throw error;
      if (scope.state === 'open') {
        countScopeCommand(r);
        r.abortRegion(scope);
        r.issue('saga_abort', scope.id, { scope: scope.id }, { sdk: true });
      }
      // A terminal command thrown in the body is issued in the same completion; the server defers it until the scope
      // settles. Any other exception waits for the scope's outcome.
      if (isTerminalError(error)) throw error;
      const settled = await outcome;
      if (settled === 'compensated') throw new SagaCompensated(error instanceof SagaAbort ? undefined : error);
      if (settled === 'partial') throw new SagaPartial(error instanceof SagaAbort ? undefined : error);
      throw new SagaCompensationFailed(error instanceof SagaAbort ? undefined : error);
    }
    // The scope's outstanding members are awaited before it is closed (Technical Plan 10.1).
    while (scope.members.size > 0) await Promise.allSettled([...scope.members]);
    if (scope.state === 'open') {
      const current = run('saga');
      countScopeCommand(current);
      scope.state = 'closed';
      current.issue('saga_close', scope.id, { scope: scope.id }, { sdk: true });
    }
    return value;
  })();
  if (parent) {
    // A nested scope is a member of its parent: the parent closes only after it settled.
    parent.members.add(result);
    const forget = (): void => { parent.members.delete(result); };
    result.then(forget, forget);
  }
  return result;
}

class SagaAbort extends Error {
  constructor() { super('the saga was aborted'); this.name = 'SagaAbort'; }
}

function sagaScopeOf(scope: Scope): SagaScope {
  return Object.freeze({
    scope: scope.id,
    activity: <T>(name: string, input?: unknown, options?: ActivityOptions) => scheduleActivity<T>(name, input, options, scope),
    script: <T>(digest: string, input?: unknown, options?: ScriptOptions) => scheduleScript<T>(digest, input, options, scope),
    saga: <T>(options: SagaOptions, body: (inner: SagaScope) => Promise<T>) => openSaga(options, body, scope),
    abort: (): never => { throw new SagaAbort(); },
  });
}

/** The durations of the code profile (Decision 044-247): 1 ms to 366 days; a wait's look-back at most 7 days. */
const MAX_CODE_DURATION_MS = 366 * 86_400_000;
const MAX_LOOKBACK_MS = 7 * 86_400_000;

function checkedDuration(value: unknown, what: string, max: number, min = 1): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < min || value > max) {
    throw new RangeError(`${what} is an integer from ${min} to ${max} milliseconds`);
  }
  return value;
}

/** Schedules an activity and resolves with its output, or rejects with `ActivityError`. */
export function activity<T = unknown>(name: string, input?: unknown, options?: ActivityOptions): CommandPromise<T> {
  return scheduleActivity<T>(name, input, options, undefined);
}

/** Runs a script artifact by digest. */
export function script<T = unknown>(artifactDigest: string, input?: unknown, options?: ScriptOptions): CommandPromise<T> {
  return scheduleScript<T>(artifactDigest, input, options, undefined);
}

/** A durable timer. */
export function sleep(durationMs: number): CommandPromise<void> {
  const r = run('sleep');
  let settle!: () => void;
  let cancel!: (error: unknown) => void;
  const handle = r.nextHandle++;
  const number = r.startTimer(durationMs, () => settle(), handle);
  r.timerOfHandle.delete(handle);
  const promise = new Promise<void>((resolve, reject) => { settle = resolve; cancel = reject; }) as CommandPromise<void>;
  r.timers.get(number)!.onCancel = () => cancel(new TimerCancelled());
  Object.defineProperty(promise, 'commandNumber', { value: number, enumerable: false });
  return promise;
}

/** Waits for an event of a declared type and a literal correlation key; `undefined` when the wait times out. */
export function waitForEvent<T = unknown>(eventType: string, correlationKey: string, options: WaitOptions): CommandPromise<T | undefined> {
  const r = run('waitForEvent');
  // The type requires it; a caller in plain JavaScript is told at the call.
  if (options?.timeoutMs === undefined) throw new TypeError('a wait for an event needs timeoutMs: the code profile requires it');
  const fragment: Record<string, unknown> = { type: 'waitEvent', eventType, correlationKey, timeoutMs: checkedDuration(options.timeoutMs, 'timeoutMs', MAX_CODE_DURATION_MS) };
  if (options?.lookbackMs !== undefined) fragment['lookbackMs'] = checkedDuration(options.lookbackMs, 'lookbackMs', MAX_LOOKBACK_MS, 0);
  // The correlation key is a literal of the fragment, which travels inline.
  const number = r.issue('wait_event', eventType, fragment, { values: [{ size: snapshot(correlationKey).size, inline: true, what: 'the correlation key' }] });
  return commandPromise<T | undefined>(number, (resolve, reject) => ({
    settle(event: HistoryEvent): void {
      if (event.kind === 'wait.outcome') resolve(event.status === 'satisfied' ? event.payload as T : undefined);
      else if (event.kind === 'step.outcome' && event.status === 'cancelled') reject(new CommandCancelled());
    },
  }));
}

/** Starts a child run of a listed workflow and resolves with its output. */
export function child<T = unknown>(workflowId: string, input?: unknown, options?: ChildOptions): CommandPromise<T> {
  const r = run('child');
  const { value, size } = snapshot(input);
  const fragment: Record<string, unknown> = { type: 'child', workflowId, input: value };
  // cancelPolicy and businessId are fragment keys; the input is the command's value.
  if (options?.cancelPolicy !== undefined) fragment['cancelPolicy'] = options.cancelPolicy;
  if (options?.businessId !== undefined) fragment['businessId'] = options.businessId;
  const values = [{ size, inline: false, what: `the input of the child ${workflowId}` }];
  // A business identifier is a literal of the fragment, which travels inline.
  if (options?.businessId !== undefined) values.push({ size: snapshot(options.businessId).size, inline: true, what: 'the business identifier' });
  const number = r.issue('child', workflowId, fragment, { values });
  return commandPromise<T>(number, (resolve, reject) => ({
    settle(event: HistoryEvent): void {
      if (event.kind !== 'child.finished') return;
      if (event.status === 'completed') resolve(event.output as T);
      else reject(new ChildWorkflowError(event.status, event.errorCode));
    },
  }));
}

/** Asks for a person's approval under a named policy of the envelope. */
export function approval(policyName: string, subject: unknown): CommandPromise<'approved' | 'rejected' | 'expired'> {
  const r = run('approval');
  const { value, size } = snapshot(subject);
  const number = r.issue('approval', policyName, { type: 'approval', policyName, subject: value },
    { alwaysInline: true, values: [{ size, inline: true, what: `the subject of the approval ${policyName}` }] });
  return commandPromise(number, (resolve, reject) => ({
    settle(event: HistoryEvent): void {
      if (event.kind === 'approval.decided') resolve(event.decision);
      else if (event.kind === 'step.outcome' && event.status === 'cancelled') reject(new CommandCancelled());
    },
  }));
}

/** Opens a saga scope: the engine plans and runs the compensations of its activities when it aborts. */
export function saga<T>(options: SagaOptions, body: (scope: SagaScope) => Promise<T>): Promise<T> {
  return openSaga(options, body, undefined);
}

/** Asks the platform to cancel one of the run's own steps or children; a step that already ended makes a no-op. */
export function requestCancellation(command: CommandPromise<unknown>): void {
  const r = run('requestCancellation');
  const number = command?.commandNumber;
  if (typeof number !== 'number') throw new TypeError('requestCancellation takes the promise of a workflow command');
  if (r.timerNumbers.has(number)) {
    // A timer, fired or not: the command is timer_cancel, a no-op on the server when it fired.
    const timer = r.timers.get(number);
    r.issue('timer_cancel', `c-${number}`, { target: `c-${number}` });
    if (timer) {
      r.timers.delete(number);
      r.timerOfHandle.delete(timer.handle);
      timer.onCancel?.();
    }
    return;
  }
  r.issue('step_cancel', `c-${number}`, { target: `c-${number}` });
}

/** Registers the handler of a declared signal; signals buffered before it are given to it at once. */
export function onSignal(name: string, handler: (payload: unknown) => void | Promise<void>): void {
  const r = run('onSignal');
  r.signalHandlers.set(name, handler);
  r.drainBuffered(name);
}

/** Waits for the next signal of a declared name, taking one already buffered first. */
export function signal<T = unknown>(name: string): Promise<T> {
  const r = run('signal');
  return new Promise<T>((resolve) => {
    r.signalWaiters.set(name, [...(r.signalWaiters.get(name) ?? []), resolve as (payload: unknown) => void]);
    r.drainBuffered(name);
  });
}

/** Registers a declared update: its validator runs when the update is admitted, its handler as a branch. */
export function onUpdate(name: string, validator: (args: unknown) => void, handler: (args: unknown) => Promise<unknown>): void {
  const r = run('onUpdate');
  r.updateHandlers.set(name, { validator, handler });
}

/** Sets the declared state; the last value of an activation is committed with its completion. */
export function setState(state: unknown): void {
  const r = run('setState');
  r.guardEffects();
  const { value, size } = snapshot(state);
  // The body the host hashes holds the state one level deeper: checked now, at the call.
  rfc8785({ state: value });
  if (size > r.stateMaxBytes) throw new InlineValuesExceeded(`the declared state of ${size} bytes is over its maxBytes of ${r.stateMaxBytes}`);
  r.stateValue = { value };
}

/** True when the run takes the new path of a patch (Technical Plan 7.2). */
export function patched(id: string): boolean {
  const r = run('patched');
  if (!MARKER_ID.test(id)) throw new TypeError('a patch identifier matches ^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$');
  const known = r.patches.get(id);
  if (known !== undefined) return known;
  if (r.replaying) {
    const record = r.expected.get(r.next);
    const taken = record !== undefined && record.kind === 'marker_patch' && record.target === id;
    if (taken) r.issue('marker_patch', id, { markerId: id });
    r.patches.set(id, taken);
    return taken;
  }
  r.issue('marker_patch', id, { markerId: id });
  r.patches.set(id, true);
  return true;
}

/** Records a patch whose old path was removed: the run always takes the new path. */
export function deprecatePatch(id: string): void {
  const r = run('deprecatePatch');
  if (!MARKER_ID.test(id)) throw new TypeError('a patch identifier matches ^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$');
  if (r.patches.has(id)) return;
  r.patches.set(id, true);
  if (r.replaying) {
    const record = r.expected.get(r.next);
    if (record !== undefined && record.kind === 'marker_patch' && record.target === id) r.issue('marker_patch', id, { markerId: id });
    return;
  }
  r.issue('marker_patch', id, { markerId: id });
}

/** Computes a value once and records it; a replay returns the recorded value instead of computing it again. */
export function sideEffect<T>(compute: () => T, options?: SideEffectOptions): T {
  const r = run('sideEffect');
  const markerId = options?.id ?? SIDE_EFFECT_MARKER_ID;
  if (typeof markerId !== 'string' || !MARKER_ID.test(markerId)) throw new TypeError('a side effect identifier matches ^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$');
  const record = r.replaying ? r.expected.get(r.next) : undefined;
  // A replay returns the recorded value instead of computing it again; the checks are the same live and replayed.
  let computed: T;
  if (record && record.kind === 'marker_side_effect') {
    if (record.redacted) r.sawRedacted = true;
    computed = record.value as T;
  } else {
    // The computation runs live only: it may draw from the generator, whose state is put back so that the run's stream
    // is where a replay, which returns the recorded value, finds it; and it may make no command.
    const saved = r.rng.save();
    r.inSideEffect = true;
    try {
      computed = compute();
    } finally {
      r.inSideEffect = false;
      r.rng.restore(saved);
    }
  }
  const { value: stored, size } = snapshot(computed);
  const value = stored as T;
  if (size > MAX_SIDE_EFFECT_BYTES) throw new InlineValuesExceeded(`a side-effect value is at most ${MAX_SIDE_EFFECT_BYTES} bytes, not ${size}`);
  r.issue('marker_side_effect', markerId, { markerId, value: stored },
    { values: [{ size, inline: false, what: 'the side-effect value' }] });
  return value;
}

/** The workflow time: the time of the activation's last `workflow_task.started`, constant during the activation. */
export function now(): Date {
  return new Date(run('now').nowMs);
}

/** A number in [0, 1) from the run's generator. */
export function random(): number {
  return run('random').rng.random();
}

/** An integer in [0, n) from the run's generator. */
export function randomInt(n: number): number {
  return run('randomInt').rng.randomInt(n);
}

/** A version 4 UUID from the run's generator. Not for secrets: anyone holding the seed can predict it. */
export function uuid(): string {
  return run('uuid').rng.uuid();
}

/** Ends this run and starts its successor with the input and, optionally, the carried state. */
export function continueAsNew(input: unknown, options?: ContinueAsNewOptions): never {
  const r = run('continueAsNew');
  const taken = snapshot(input);
  if (options?.version !== undefined && options.version !== 'same' && options.version !== 'active') throw new TypeError('version is same or active');
  const body: Record<string, unknown> = { input: taken.value, state: null, version: options?.version ?? 'same' };
  let size = taken.size;
  if (options && 'state' in options && options.state !== undefined) {
    // The carried state is bounded by the current version only; the server checks the resolved successor (Task 044-36).
    const state = snapshot(options.state);
    if (state.size > r.stateMaxBytes) {
      throw new InlineValuesExceeded(`the carried state of ${state.size} bytes is over the current version's maxBytes of ${r.stateMaxBytes}`);
    }
    body['state'] = state.value;
    size += state.size;
  }
  rfc8785(body);
  r.admitInlineValue(size, 'the input and carried state of continue-as-new');
  throw new ContinueAsNewRequest(body);
}

/** Whether the run has been asked to cancel, and a promise that settles when it is. */
export const cancellation: Cancellation = Object.freeze({
  get requested(): boolean { return run('cancellation').cancelRequested; },
  get promise(): Promise<void> { return run('cancellation').cancelPromise; },
});

/** Ends the run `CANCELLED` in answer to a cancellation request. */
export function acknowledgeCancellation(): never {
  run('acknowledgeCancellation');
  throw new CancellationAcknowledged();
}

/** Whether the history has grown enough that continuing as new is suggested. */
export function continueAsNewSuggested(): boolean {
  return run('continueAsNewSuggested').casSuggested;
}

/** The state a predecessor carried into this run. */
export function carriedState(): unknown {
  return run('carriedState').carried;
}

/** Whether the run is in its closing window after the history limit. */
export function historyLimitReached(): boolean {
  return run('historyLimitReached').historyLimit;
}

const api: WorkflowContext = Object.freeze({
  activity, script, sleep, waitForEvent, child, approval, saga, requestCancellation, onSignal, signal, onUpdate, setState,
  patched, deprecatePatch, sideEffect, now, random, randomInt, uuid, continueAsNew, cancellation, acknowledgeCancellation,
  continueAsNewSuggested, historyLimitReached, carriedState,
});

// ---------------------------------------------------------------------------------------------------------------------
// The context's globals and the bridge.

/**
 * Registers the bundle's workflow functions and installs the bridge. Called once by the bundle after its modules were
 * evaluated; refuses to run where the host did not mark the context (a bundle loaded anywhere else).
 */
export function __sagatInstall(workflows: Record<string, unknown>): void {
  const global = globalThis as Record<string, unknown>;
  if (global['__sagat_context'] !== BRIDGE_VERSION || !globalsInstalled || typeof global['__sagat_bridge'] !== 'undefined') {
    throw new Error('a workflow bundle runs only inside the SDK\'s workflow context');
  }
  for (const [name, value] of Object.entries(workflows)) {
    if (typeof value === 'function') registered.set(name, value as WorkflowFunction);
  }
  Object.defineProperty(global, '__sagat_bridge', {
    value: Object.freeze({ version: BRIDGE_VERSION, call: bridgeCall }),
    enumerable: false, configurable: false, writable: false,
  });
}

let globalsInstalled = false;

/** The globals of the context, replaced by the SDK before any workflow module is evaluated. */
function installGlobals(global: Record<string, unknown>): void {
  globalsInstalled = true;
  const OriginalDate = global['Date'] as DateConstructor;
  const timeNow = (): number => {
    if (!currentRun || !currentRun.activating) throw new NotInWorkflow('the current time');
    return currentRun.nowMs;
  };
  const WorkflowDate = function (this: unknown, ...args: unknown[]): unknown {
    if (new.target === undefined) return new OriginalDate(timeNow()).toString();
    if (args.length === 0) return Reflect.construct(OriginalDate, [timeNow()], new.target);
    return Reflect.construct(OriginalDate, args, new.target);
  } as unknown as DateConstructor;
  Object.defineProperty(WorkflowDate, 'prototype', { value: OriginalDate.prototype, writable: false });
  Object.defineProperty(OriginalDate.prototype, 'constructor', { value: WorkflowDate, writable: true, configurable: true });
  Object.defineProperty(WorkflowDate, 'now', { value: timeNow, writable: true, configurable: true });
  Object.defineProperty(WorkflowDate, 'UTC', { value: OriginalDate.UTC, writable: true, configurable: true });
  Object.defineProperty(WorkflowDate, 'parse', { value: OriginalDate.parse, writable: true, configurable: true });
  global['Date'] = WorkflowDate;

  const math = global['Math'] as Math;
  Object.defineProperty(math, 'random', {
    value: (): number => {
      if (!currentRun || !currentRun.activating) throw new NotInWorkflow('Math.random');
      return currentRun.rng.random();
    },
    writable: true, configurable: true,
  });

  global['setTimeout'] = (callback: unknown, ms?: number, ...args: unknown[]): number => {
    const r = run('setTimeout');
    if (typeof callback !== 'function') throw new TypeError('setTimeout takes a function in a workflow');
    const handle = r.nextHandle++;
    r.startTimer(Number(ms ?? 0), () => r.runHandler(() => (callback as (...a: unknown[]) => unknown)(...args)), handle);
    return handle;
  };
  global['setInterval'] = (callback: unknown, ms?: number, ...args: unknown[]): number => {
    const r = run('setInterval');
    if (typeof callback !== 'function') throw new TypeError('setInterval takes a function in a workflow');
    const handle = r.nextHandle++;
    const interval = Math.max(1, Math.ceil(Number(ms ?? 0)));
    r.startTimer(interval, () => r.runHandler(() => (callback as (...a: unknown[]) => unknown)(...args)), handle, interval);
    return handle;
  };
  global['clearTimeout'] = (handle: unknown): void => { run('clearTimeout').cancelTimer(handle); };
  global['clearInterval'] = (handle: unknown): void => { run('clearInterval').cancelTimer(handle); };
  global['queueMicrotask'] = (callback: unknown): void => {
    if (typeof callback !== 'function') throw new TypeError('queueMicrotask takes a function');
    void Promise.resolve().then(callback as () => void);
  };
  const logger = (level: LogLine['level']) => (...args: unknown[]): void => { currentRun?.log(level, args); };
  global['console'] = Object.freeze({
    log: logger('info'), info: logger('info'), debug: logger('debug'), warn: logger('warn'), error: logger('error'),
  });

  // WebAssembly cannot compile here (no code generation) and its shared memory would hand out a SharedArrayBuffer.
  for (const name of ['WeakRef', 'FinalizationRegistry', 'SharedArrayBuffer', 'Atomics', 'WebAssembly', 'crypto', 'performance', 'process']) {
    delete global[name];
  }
  installTemporal(global, timeNow);
  installStackTraces(global);
  installLocale(global, timeNow);
}

// Temporal.Now reads the real clock and the process's time zone: it answers the workflow time in UTC instead.
function installTemporal(global: Record<string, unknown>, timeNow: () => number): void {
  const temporal = global['Temporal'] as Record<string, unknown> | undefined;
  if (!temporal || typeof temporal !== 'object') return;
  const Instant = temporal['Instant'] as { fromEpochMilliseconds(ms: number): { toZonedDateTimeISO(zone: string): Record<string, () => unknown> } };
  const instant = (): ReturnType<typeof Instant.fromEpochMilliseconds> => Instant.fromEpochMilliseconds(timeNow());
  const zoned = (zone?: unknown): Record<string, () => unknown> => instant().toZonedDateTimeISO(zone === undefined ? 'UTC' : String(zone));
  Object.defineProperty(temporal, 'Now', {
    value: Object.freeze({
      instant,
      timeZoneId: (): string => 'UTC',
      zonedDateTimeISO: (zone?: unknown): unknown => zoned(zone),
      plainDateTimeISO: (zone?: unknown): unknown => zoned(zone)['toPlainDateTime']!(),
      plainDateISO: (zone?: unknown): unknown => zoned(zone)['toPlainDate']!(),
      plainTimeISO: (zone?: unknown): unknown => zoned(zone)['toPlainTime']!(),
    }),
    writable: true, configurable: true,
  });
}

// A stack trace lists only the frames of the bundle: the host's frames name its files, which differ between workers
// and are not the workflow's to see.
function installStackTraces(global: Record<string, unknown>): void {
  const ErrorConstructor = global['Error'] as ErrorConstructor & { prepareStackTrace?: unknown };
  ErrorConstructor.prepareStackTrace = (error: Error, frames: { getFileName(): string | null; getFunctionName(): string | null; getLineNumber(): number | null; getColumnNumber(): number | null }[]): string => {
    let header: string;
    try { header = `${error.name}: ${error.message}`; } catch { header = 'Error'; }
    const own = frames.filter((frame) => frame.getFileName() === 'workflow-bundle.js');
    return header + own.map((frame) => `\n    at ${frame.getFunctionName() ?? '<anonymous>'} (workflow-bundle.js:${frame.getLineNumber() ?? 0}:${frame.getColumnNumber() ?? 0})`).join('');
  };
}

function installLocale(global: Record<string, unknown>, timeNow: () => number): void {
  const intl = global['Intl'] as Record<string, unknown> | undefined;
  if (!intl) return;
  const fixed = (locales: unknown): unknown => (locales === undefined ? WORKFLOW_LOCALE : locales);
  for (const name of ['Collator', 'DateTimeFormat', 'DisplayNames', 'DurationFormat', 'ListFormat', 'NumberFormat', 'PluralRules', 'RelativeTimeFormat', 'Segmenter']) {
    const Original = intl[name] as (new (...a: unknown[]) => unknown) & Record<string, unknown> | undefined;
    if (typeof Original !== 'function') continue;
    const Wrapped = function (this: unknown, locales?: unknown, options?: unknown): unknown {
      let chosen = options;
      if (name === 'DateTimeFormat') {
        const given = (options ?? {}) as Record<string, unknown>;
        chosen = given['timeZone'] === undefined ? { ...given, timeZone: 'UTC' } : options;
      }
      return Reflect.construct(Original, [fixed(locales), chosen], new.target ?? Original);
    } as unknown as Record<string, unknown>;
    Object.defineProperty(Wrapped, 'prototype', { value: Original.prototype, writable: false });
    if (typeof Original['supportedLocalesOf'] === 'function') Wrapped['supportedLocalesOf'] = Original['supportedLocalesOf'];
    intl[name] = Wrapped;
    if (name === 'DateTimeFormat') {
      // format() and formatToParts() without a date read the real clock: they take the workflow time instead.
      const proto = Original.prototype as Record<string, unknown>;
      const formatGetter = Object.getOwnPropertyDescriptor(proto, 'format')?.get;
      if (formatGetter) {
        Object.defineProperty(proto, 'format', {
          get(this: unknown) {
            const bound = formatGetter.call(this) as (date?: unknown) => string;
            return (date?: unknown): string => bound(date === undefined ? timeNow() : date);
          },
          configurable: true,
        });
      }
      const formatToParts = proto['formatToParts'] as ((date?: unknown) => unknown) | undefined;
      if (typeof formatToParts === 'function') {
        Object.defineProperty(proto, 'formatToParts', {
          value: function (this: unknown, date?: unknown): unknown { return formatToParts.call(this, date === undefined ? timeNow() : date); },
          writable: true, configurable: true,
        });
      }
    }
  }
  const patch = (target: unknown, method: string): void => {
    const proto = target as Record<string, unknown>;
    const original = proto[method] as ((...a: unknown[]) => unknown) | undefined;
    if (typeof original !== 'function') return;
    Object.defineProperty(proto, method, {
      value: function (this: unknown, ...args: unknown[]): unknown {
        if (method === 'localeCompare') return original.call(this, args[0], fixed(args[1]), args[2]);
        const options = (args[1] ?? {}) as Record<string, unknown>;
        const zoned = target === (global['Date'] as DateConstructor).prototype && options['timeZone'] === undefined ? { ...options, timeZone: 'UTC' } : args[1];
        return original.call(this, fixed(args[0]), zoned);
      },
      writable: true, configurable: true,
    });
  };
  patch((global['Number'] as NumberConstructor).prototype, 'toLocaleString');
  patch((global['BigInt'] as BigIntConstructor).prototype, 'toLocaleString');
  patch((global['Date'] as DateConstructor).prototype, 'toLocaleString');
  patch((global['Date'] as DateConstructor).prototype, 'toLocaleDateString');
  patch((global['Date'] as DateConstructor).prototype, 'toLocaleTimeString');
  patch((global['String'] as StringConstructor).prototype, 'localeCompare');
  for (const method of ['toLocaleUpperCase', 'toLocaleLowerCase']) {
    const proto = (global['String'] as StringConstructor).prototype as unknown as Record<string, unknown>;
    const original = proto[method] as (this: string, locales?: unknown) => string;
    Object.defineProperty(proto, method, {
      value: function (this: string, locales?: unknown): string { return original.call(this, fixed(locales)); },
      writable: true, configurable: true,
    });
  }
}

type BridgeRequest =
  | { op: 'init'; init: RunInit }
  | { op: 'begin'; begin: BeginActivation }
  | { op: 'apply'; event: HistoryEvent }
  | { op: 'end' }
  | { op: 'commit'; deferred: boolean }
  | { op: 'probe' };

function bridgeCall(input: unknown): string {
  if (typeof input !== 'string') return JSON.stringify({ error: 'the bridge takes a JSON string' });
  let request: BridgeRequest;
  try { request = JSON.parse(input) as BridgeRequest; } catch { return JSON.stringify({ error: 'the bridge request is not JSON' }); }
  switch (request.op) {
    case 'probe':
      return JSON.stringify({ version: BRIDGE_VERSION, workflowTypes: [...registered.keys()].sort() });
    case 'init': {
      const fn = registered.get(request.init.workflowType);
      if (!fn) return JSON.stringify({ error: `the bundle has no workflow ${request.init.workflowType}` });
      currentRun = new Run(request.init, fn);
      return '{}';
    }
    case 'begin':
      currentRun!.begin(request.begin);
      return '{}';
    case 'apply':
      try { currentRun!.apply(request.event); } catch (error) { if (!(error instanceof ActivationStopped)) currentRun!.failTask(error); }
      return currentRun!.failure ? '{"stopped":true}' : '{}';
    case 'end':
      return JSON.stringify(currentRun!.end());
    case 'commit':
      currentRun!.commit(request.deferred);
      return '{}';
  }
}

// The host marks a workflow context with a number before it evaluates the bundle, and this module is the first one the
// bundle evaluates: so the globals are replaced before any workflow module runs, and never in the host process.
if ((globalThis as Record<string, unknown>)['__sagat_context'] === BRIDGE_VERSION) installGlobals(globalThis as Record<string, unknown>);
