/**
 * Step-log capture (design 041, `sdks/SPEC.md` §6.9): what an activity writes to its logger is numbered, redacted with
 * the process's secret set, cut to the line ceilings, buffered per attempt, sent through `AppendStepLogs` while the
 * attempt runs and, for its last records, carried on its report. Nothing here blocks a logger call on the network or
 * throws into the activity: a failure of capture is counted and dropped (Requirement 041-005).
 */
import { SagatError } from '../errors.js';

export type StepLogLevel = 'trace' | 'debug' | 'info' | 'warn' | 'error';

/** The worker option of §6.9: capture on, at `info`, unless the worker says otherwise. */
export interface StepLogOptions {
  capture?: boolean;
  level?: StepLogLevel;
}

/** Why the SDK dropped a record (data-model.md §7): counted locally, never sent. */
export type StepLogDropReason = 'report_deadline' | 'buffer_full' | 'send_failed' | 'after_ceiling' | 'process_bound' | 'secret_set_full';

// ---- the constants of §6.9 --------------------------------------------------------------------------------------

export const LIMITS = {
  attemptRecords: 1000,
  attemptBytes: 256 * 1024,
  processBytes: 16 * 1024 * 1024,
  flushRecords: 256,
  flushMs: 1000,
  batchRecords: 500,
  batchBytes: 192 * 1024,
  batchAgeMs: 10_000,
  retryDelaysMs: [200, 1000] as const,
  carryRecords: 64,
  carryBytes: 16 * 1024,
  reportWaitMs: 2000,
  messageBytes: 8192,
  fieldKeys: 32,
  fieldBytes: 2048,
  secretValues: 1024,
  secretBytes: 256 * 1024,
  secretMinBytes: 6,
} as const;

const LEVELS: readonly StepLogLevel[] = ['trace', 'debug', 'info', 'warn', 'error'];
const REDACTED = '[REDACTED]';

export function levelRank(level: StepLogLevel): number {
  return LEVELS.indexOf(level);
}

export function isStepLogLevel(value: unknown): value is StepLogLevel {
  return typeof value === 'string' && (LEVELS as readonly string[]).includes(value);
}

// ---- the wire shapes (worker.proto, protocol 1.7) ---------------------------------------------------------------

export interface StepLogField {
  key: string;
  string_value?: string;
  int_value?: number | string;
  double_value?: number;
  bool_value?: boolean;
}
export interface StepLogRecord {
  sequence: number;
  worker_time_epoch_ms: number;
  level: string;
  // A value equal to its proto3 default is left out, as protobuf leaves it out of the wire (protobufjs writes a field
  // that is present even when it holds its default), so that the sizes counted are the sizes sent.
  message?: string;
  fields?: StepLogField[];
  exception?: string;
  truncated?: boolean;
}
export interface StepLogCarry { records: StepLogRecord[]; highest_sequence: number }

const WIRE_LEVEL: Record<StepLogLevel, string> = {
  trace: 'STEP_LOG_LEVEL_TRACE',
  debug: 'STEP_LOG_LEVEL_DEBUG',
  info: 'STEP_LOG_LEVEL_INFO',
  warn: 'STEP_LOG_LEVEL_WARN',
  error: 'STEP_LOG_LEVEL_ERROR',
};
const LEVEL_NUMBER: Record<string, number> = {
  STEP_LOG_LEVEL_TRACE: 1, STEP_LOG_LEVEL_DEBUG: 2, STEP_LOG_LEVEL_INFO: 3, STEP_LOG_LEVEL_WARN: 4, STEP_LOG_LEVEL_ERROR: 5,
};

// ---- protobuf sizes, by hand: exact, and checked against protobufjs by the tests ---------------------------------
// protobufjs writes every field the object holds, a default value included; so a field the SDK always sets is counted
// whatever its value, and one that holds its default is left out of the object (`buildRecord`).

function varintSize(value: number | bigint): number {
  let n = typeof value === 'bigint' ? value : BigInt(Math.trunc(value));
  if (n < 0n) return 10; // a negative int32 or int64 is sign-extended to ten bytes
  let size = 1;
  while (n >= 128n) {
    n >>= 7n;
    size += 1;
  }
  return size;
}

function stringSize(text: string): number {
  return Buffer.byteLength(text, 'utf8');
}

/** The bytes of one length-delimited field of `size` bytes, its tag included (every tag here fits one byte). */
function delimited(size: number): number {
  return 1 + varintSize(size) + size;
}

function fieldSize(field: StepLogField): number {
  let size = delimited(stringSize(field.key));
  if (field.string_value !== undefined) size += delimited(stringSize(field.string_value));
  else if (field.int_value !== undefined) size += 1 + varintSize(BigInt(field.int_value));
  else if (field.double_value !== undefined) size += 1 + 8;
  else if (field.bool_value !== undefined) size += 2;
  return size;
}

/** The encoded size of a `StepLogRecord`, proto3 defaults omitted. */
export function recordSize(record: StepLogRecord): number {
  let size = 1 + varintSize(record.sequence) + 1 + varintSize(record.worker_time_epoch_ms)
    + 1 + varintSize(LEVEL_NUMBER[record.level] ?? 0);
  if (record.message) size += delimited(stringSize(record.message));
  for (const field of record.fields ?? []) size += delimited(fieldSize(field));
  if (record.exception) size += delimited(stringSize(record.exception));
  if (record.truncated) size += 2;
  return size;
}

/** The encoded size of a `StepLogCarry` holding records of these sizes. */
export function carrySize(recordSizes: readonly number[], highest: number): number {
  let size = 1 + varintSize(highest);
  for (const record of recordSizes) size += delimited(record);
  return size;
}

export interface AttemptIdentity {
  task_id: string; run_id: string; step_id: string; attempt: number; generation: number; lease_token: string;
  instance?: { instance_path: Array<{ scope_node_id: string; iteration_index: number }>; definition_node_id: string };
}

/** The encoded size of an `AppendStepLogsRequest` of this attempt and session, without its records. */
export function requestOverhead(identity: AttemptIdentity, workerId: string, sessionToken: string): number {
  const text = (value: string): number => delimited(stringSize(value));
  let size = text(workerId) + text(sessionToken) + text(identity.task_id) + text(identity.lease_token)
    + text(identity.run_id) + text(identity.step_id) + 1 + varintSize(identity.generation) + 1 + varintSize(identity.attempt);
  if (identity.instance) {
    let instance = text(identity.instance.definition_node_id);
    for (const segment of identity.instance.instance_path) {
      instance += delimited(text(segment.scope_node_id) + 1 + varintSize(segment.iteration_index));
    }
    size += delimited(instance);
  }
  return size;
}

// ---- text: well-formed, cut at a character boundary ------------------------------------------------------------

/** NUL and every lone surrogate become U+FFFD, so that what is sent is valid UTF-8 without a NUL (§6.9, step 3). */
export function wellFormed(text: string): string {
  return text.replace(/\u0000|[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/g, '�');
}

/** The longest prefix of `text` of at most `max` UTF-8 bytes that ends at a character boundary, and whether it cut. */
export function cutUtf8(text: string, max: number): [string, boolean] {
  if (text.length * 3 <= max || Buffer.byteLength(text, 'utf8') <= max) return [text, false];
  let bytes = 0;
  let end = 0;
  for (const char of text) {
    const code = char.codePointAt(0)!;
    const size = code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
    if (bytes + size > max) break;
    bytes += size;
    end += char.length;
  }
  return [text.slice(0, end), true];
}

// ---- the process's secret set and its redaction ----------------------------------------------------------------

/**
 * An Aho–Corasick automaton over UTF-16 code units: it tells whether any value occurs in a text in one pass, so that
 * a text holding none (the usual case) costs one scan whatever the size of the set.
 */
class Matcher {
  private readonly next: Array<Map<number, number>> = [new Map()];
  private readonly fail: number[] = [0];
  private readonly out: boolean[] = [false];

  constructor(values: readonly string[]) {
    for (const value of values) {
      let state = 0;
      for (let i = 0; i < value.length; i++) {
        const unit = value.charCodeAt(i);
        let to = this.next[state]!.get(unit);
        if (to === undefined) {
          to = this.next.length;
          this.next.push(new Map());
          this.fail.push(0);
          this.out.push(false);
          this.next[state]!.set(unit, to);
        }
        state = to;
      }
      this.out[state] = true;
    }
    const queue: number[] = [];
    for (const to of this.next[0]!.values()) queue.push(to);
    for (let head = 0; head < queue.length; head++) {
      const state = queue[head]!;
      for (const [unit, to] of this.next[state]!) {
        let f = this.fail[state]!;
        while (f !== 0 && !this.next[f]!.has(unit)) f = this.fail[f]!;
        const target = this.next[f]!.get(unit);
        this.fail[to] = target !== undefined && target !== to ? target : 0;
        if (this.out[this.fail[to]!]) this.out[to] = true;
        queue.push(to);
      }
    }
  }

  occursIn(text: string): boolean {
    let state = 0;
    for (let i = 0; i < text.length; i++) {
      const unit = text.charCodeAt(i);
      while (state !== 0 && !this.next[state]!.has(unit)) state = this.fail[state]!;
      state = this.next[state]!.get(unit) ?? 0;
      if (this.out[state]) return true;
    }
    return false;
  }
}

/**
 * The values `ResolveSecrets` returned in this process (§6.7): distinct values of at least 6 UTF-8 bytes, at most
 * 1,024 of them or 256 KiB. Module-private: never exported from the package, never printed, never handed to activity
 * code. When a value does not fit, capture stops for the process.
 */
export class SecretSet {
  #values: string[] = [];
  #bytes = 0;
  #full = false;
  #matcher: Matcher | undefined;
  #ordered: string[] | undefined;
  #warned = false;

  get full(): boolean {
    return this.#full;
  }

  /** Adds what one resolution returned; true when the set stopped capture with it (once). */
  add(values: readonly Uint8Array[]): boolean {
    const decoder = new TextDecoder('utf-8', { fatal: true });
    let stopped = false;
    for (const bytes of values) {
      if (bytes.length < LIMITS.secretMinBytes) continue;
      let text: string;
      try {
        text = decoder.decode(bytes);
      } catch {
        // Not UTF-8: the text an activity can log is the one the accessor hands out (§6.7).
        text = Buffer.from(bytes).toString('utf8');
      }
      if (this.#values.includes(text)) continue;
      if (this.#full || this.#values.length + 1 > LIMITS.secretValues || this.#bytes + bytes.length > LIMITS.secretBytes) {
        if (!this.#full) stopped = true;
        this.#full = true;
        continue;
      }
      this.#values.push(text);
      this.#bytes += bytes.length;
      this.#matcher = undefined;
      this.#ordered = undefined;
    }
    return stopped;
  }

  /** True the first time it is asked after the set filled: the one local line per process. */
  shouldWarn(): boolean {
    if (!this.#full || this.#warned) return false;
    this.#warned = true;
    return true;
  }

  /** The rule of `sagat_domain::scripts::redact` without its shapes, equal to it on every text (§6.9). */
  redact(text: string): string {
    if (this.#values.length === 0 || text.length < 2) return text;
    this.#matcher ??= new Matcher(this.#values);
    if (!this.#matcher.occursIn(text)) return text;
    // Longest first by UTF-8 bytes; equal lengths keep the order they joined the set (a stable sort).
    this.#ordered ??= [...this.#values].sort((a, b) => Buffer.byteLength(b, 'utf8') - Buffer.byteLength(a, 'utf8'));
    let out = text;
    for (const value of this.#ordered) if (out.includes(value)) out = out.split(value).join(REDACTED);
    return out;
  }

  wipe(): void {
    this.#values = [];
    this.#bytes = 0;
    this.#full = false;
    this.#warned = false;
    this.#matcher = undefined;
    this.#ordered = undefined;
  }

  toString(): string {
    return `SecretSet(${this.#values.length} values)`;
  }

  [Symbol.for('nodejs.util.inspect.custom')](): string {
    return this.toString();
  }
}

/**
 * What the whole process shares: the secret set, which lives for the process (§6.7, decision D041-164), and the bytes
 * every attempt holds. Module-private: the module exports only a way to add to the set, so nothing that imports it,
 * even by its file path, can read the set or redact with it.
 */
const processSecrets = new SecretSet();
let heldBytes = 0;

/** Adds the values one resolution returned; true when the set stopped capture for the process with them (once). */
export function addProcessSecrets(values: readonly Uint8Array[]): boolean {
  return processSecrets.add(values);
}

/** Whether capture stopped for the process because its secret set is full. */
export function processCaptureStopped(): boolean {
  return processSecrets.full;
}

// ---- formatting one call -----------------------------------------------------------------------------------------

/** ECMAScript prints a number as the canonical text of data-model.md §5. */
function numberText(value: number | bigint): string {
  return typeof value === 'bigint' ? value.toString() : String(value);
}

const INT64_MIN = -(2n ** 63n);
const INT64_MAX = 2n ** 63n - 1n;

/** A byte buffer, any view of bytes, as its UTF-8 text (§6.9): never as numbers or base64. */
function bytesText(value: unknown): string | undefined {
  if (Buffer.isBuffer(value)) return value.toString('utf8');
  if (ArrayBuffer.isView(value)) return Buffer.from(value.buffer, value.byteOffset, value.byteLength).toString('utf8');
  if (value instanceof ArrayBuffer) return Buffer.from(value).toString('utf8');
  return undefined;
}

/**
 * A copy of a value whose every string and every key is redacted, so that encoding it cannot escape a secret out of
 * redaction's sight (§6.9); a byte buffer inside it becomes its redacted text.
 */
function redactedCopy(value: unknown, secrets: { redact(text: string): string }, seen: Set<unknown>): unknown {
  if (typeof value === 'string') return secrets.redact(value);
  const bytes = bytesText(value);
  if (bytes !== undefined) return secrets.redact(bytes);
  if (value === null || typeof value !== 'object') return value;
  if (seen.has(value)) throw new TypeError('circular');
  seen.add(value);
  const json = (value as { toJSON?: unknown }).toJSON;
  if (typeof json === 'function') return redactedCopy(json.call(value), secrets, seen);
  if (Array.isArray(value)) return value.map((item) => redactedCopy(item, secrets, seen));
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(value)) out[secrets.redact(key)] = redactedCopy((value as Record<string, unknown>)[key], secrets, seen);
  return out;
}

/** One field value as the wire carries it; a string is redacted later, any other text is redacted here. */
function fieldValue(value: unknown, secrets: { redact(text: string): string }): Omit<StepLogField, 'key'> | undefined {
  if (value === null || value === undefined) return undefined;
  if (typeof value === 'string') return { string_value: value };
  if (typeof value === 'boolean') return { bool_value: value };
  if (typeof value === 'bigint') {
    return value >= INT64_MIN && value <= INT64_MAX ? { int_value: value.toString() } : { string_value: value.toString() };
  }
  if (typeof value === 'number') {
    if (Number.isNaN(value)) return { string_value: 'NaN' };
    if (!Number.isFinite(value)) return { string_value: value > 0 ? 'Infinity' : '-Infinity' };
    // A JavaScript integer is exact up to 2^53 - 1; past it the number is a double, as it was given.
    if (Number.isSafeInteger(value)) return { int_value: value };
    return { double_value: value };
  }
  const bytes = bytesText(value);
  if (bytes !== undefined) return { string_value: bytes };
  let text: string | undefined;
  try {
    text = JSON.stringify(redactedCopy(value, secrets, new Set()));
  } catch {
    text = undefined;
  }
  if (typeof text !== 'string') {
    try {
      text = String(value);
    } catch {
      text = Object.prototype.toString.call(value);
    }
  }
  return { string_value: text };
}

function jsonString(text: string): string {
  let out = '"';
  for (const char of text) {
    const code = char.codePointAt(0)!;
    if (char === '"') out += '\\"';
    else if (char === '\\') out += '\\\\';
    else if (code < 0x20) out += `\\u${code.toString(16).padStart(4, '0')}`;
    else out += char;
  }
  return `${out}"`;
}

function jsonValue(field: StepLogField): string {
  if (field.string_value !== undefined) return jsonString(field.string_value);
  if (field.int_value !== undefined) return String(field.int_value);
  if (field.double_value !== undefined) return numberText(field.double_value);
  return field.bool_value ? 'true' : 'false';
}

function compareUtf8(a: string, b: string): number {
  return Buffer.compare(Buffer.from(a, 'utf8'), Buffer.from(b, 'utf8'));
}

/** The fields of a call by data-model.md §5: redacted, keys made unique, kept in key order up to 32 keys and 2 KiB. */
export function buildFields(input: Record<string, unknown> | undefined, secrets: { redact(text: string): string }): {
  fields: StepLogField[];
  truncated: boolean;
} {
  if (!input) return { fields: [], truncated: false };
  const given: Array<[string, Omit<StepLogField, 'key'>]> = [];
  for (const key of Object.keys(input)) {
    let value: Omit<StepLogField, 'key'> | undefined;
    try {
      value = fieldValue(input[key], secrets);
    } catch {
      value = undefined; // a getter that throws costs its field
    }
    if (value) given.push([key, value]);
  }
  // The order sent: the original keys' UTF-8 bytes, the same in every language.
  given.sort((a, b) => compareUtf8(a[0], b[0]));
  const used = new Set<string>();
  const made: StepLogField[] = [];
  for (const [original, value] of given) {
    // Redacted on the text as given, then made well formed (§6.9, steps 4 and 5).
    const redactedKey = wellFormed(secrets.redact(original));
    let key = redactedKey;
    for (let n = 2; used.has(key); n++) key = `${redactedKey}~${n}`;
    used.add(key);
    const field: StepLogField = { key, ...value };
    if (field.string_value !== undefined) field.string_value = wellFormed(secrets.redact(field.string_value));
    else if (field.int_value !== undefined || field.double_value !== undefined) {
      const text = field.int_value !== undefined ? String(field.int_value) : numberText(field.double_value!);
      if (secrets.redact(text) !== text) {
        delete field.int_value;
        delete field.double_value;
        field.string_value = REDACTED;
      }
    }
    made.push(field);
  }
  made.sort((a, b) => compareUtf8(a.key, b.key));
  const kept: StepLogField[] = [];
  let bytes = 2; // the braces
  for (const field of made) {
    const entry = Buffer.byteLength(`${jsonString(field.key)}:${jsonValue(field)}`, 'utf8') + (kept.length > 0 ? 1 : 0);
    if (kept.length + 1 > LIMITS.fieldKeys || bytes + entry > LIMITS.fieldBytes) {
      return { fields: kept, truncated: true };
    }
    kept.push(field);
    bytes += entry;
  }
  return { fields: kept, truncated: false };
}

/** The exception text of §6.9 for TypeScript: an `Error` as `name: message` and its stack; anything else as a string. */
export function exceptionText(error: unknown): string {
  if (error === undefined || error === null) return '';
  try {
    if (error instanceof Error) {
      const head = `${error.name}: ${error.message}`;
      const stack = typeof error.stack === 'string' ? error.stack : '';
      return stack.startsWith(head) ? stack : stack === '' ? head : `${head}\n${stack}`;
    }
    return String(error);
  } catch {
    return Object.prototype.toString.call(error);
  }
}

/** Steps 3 to 5 of a call (§6.9): format, make well formed, redact, cut. */
export function buildRecord(
  sequence: number,
  time: number,
  level: StepLogLevel,
  message: unknown,
  fields: Record<string, unknown> | undefined,
  error: unknown,
  secrets: { redact(text: string): string },
): StepLogRecord {
  let text: string;
  try {
    text = typeof message === 'string' ? message : String(message);
  } catch {
    text = '';
  }
  const [cutMessage, messageCut] = cutUtf8(wellFormed(secrets.redact(text)), LIMITS.messageBytes);
  const [cutException, exceptionCut] = cutUtf8(wellFormed(secrets.redact(exceptionText(error))), LIMITS.messageBytes);
  const built = buildFields(fields, secrets);
  const record: StepLogRecord = { sequence, worker_time_epoch_ms: time, level: WIRE_LEVEL[level] };
  if (cutMessage !== '') record.message = cutMessage;
  if (built.fields.length > 0) record.fields = built.fields;
  if (cutException !== '') record.exception = cutException;
  if (messageCut || exceptionCut || built.truncated) record.truncated = true;
  return record;
}

// ---- one attempt -------------------------------------------------------------------------------------------------

/** The capture state a registration gives a session: shared by its attempts, stopped by `STOP_CAPTURE`. */
export class CaptureSession {
  stopped = false;
  constructor(readonly enabled: boolean) {}
}

/** The answer of one `AppendStepLogs`, or the failure the transport or the service gave. */
export type AppendOutcome =
  | { kind: 'answer'; verdict: string; stopReason: string; retryAfterMs: number }
  | { kind: 'error'; status: string; retryAfterMs: number | null };

/** What an attempt needs of its host. */
export interface CaptureHost {
  /** The session id and token the host holds now. */
  session(): { workerId: string; token: string };
  /** One `AppendStepLogs`, with its own deadline; never throws. */
  append(request: Record<string, unknown>, deadlineMs: number): Promise<AppendOutcome>;
  /** The configured call timeout. */
  timeoutMs: number;
  /** Counted drops, once per attempt and reason when it ends. */
  dropped(reason: StepLogDropReason, records: number): void;
  /** The one local line per process when the server stops capture or the secret set is full. */
  warnOnce(message: string): void;
}

interface Held { record: StepLogRecord; size: number }

interface Batch {
  held: Held[];
  done: Promise<void>;
  firstSentAt: number;
}

type AttemptState = 'capturing' | 'finalOnly' | 'stopped';

const sleep = (ms: number): Promise<void> => new Promise((resolve) => {
  const timer = setTimeout(resolve, ms);
  timer.unref?.();
});

function jittered(ms: number): number {
  return Math.round(ms * (0.8 + Math.random() * 0.4));
}

/** The capture of one attempt: its sequences, its buffer, its sender and, at its end, its carry. */
export class AttemptCapture {
  #state: AttemptState = 'capturing';
  #lastSequence = 0;
  #buffer: Held[] = [];
  #bufferBytes = 0;
  #inFlight: Batch | undefined;
  #batchesStopped = false;
  #ended = false;
  #closed = false;
  #tick: NodeJS.Timeout | undefined;
  readonly #drops = new Map<StepLogDropReason, number>();

  constructor(
    private readonly host: CaptureHost,
    private readonly session: CaptureSession,
    private readonly identity: AttemptIdentity,
    private readonly level: StepLogLevel,
  ) {}

  /** The last sequence taken: the carry's `highest_sequence`. */
  get highestSequence(): number {
    return this.#lastSequence;
  }

  #count(reason: StepLogDropReason, records = 1): void {
    if (records > 0) this.#drops.set(reason, (this.#drops.get(reason) ?? 0) + records);
  }

  #stopped(): boolean {
    return this.#state === 'stopped' || this.session.stopped;
  }

  /** A logger call (§6.9, "A call, in order"). Never throws. */
  log(level: StepLogLevel, message: unknown, fields: Record<string, unknown> | undefined, error: unknown): void {
    try {
      if (this.#ended || this.#stopped() || levelRank(level) < levelRank(this.level)) return;
      const sequence = ++this.#lastSequence;
      const time = Date.now();
      if (this.#state === 'finalOnly') return this.#count('after_ceiling');
      if (processSecrets.full) return this.#count('secret_set_full');
      if (this.#buffer.length >= LIMITS.attemptRecords || this.#bufferBytes >= LIMITS.attemptBytes) return this.#count('buffer_full');
      if (heldBytes >= LIMITS.processBytes) return this.#count('process_bound');
      const record = buildRecord(sequence, time, level, message, fields, error, processSecrets);
      const size = recordSize(record);
      this.#buffer.push({ record, size });
      this.#bufferBytes += size;
      heldBytes += size;
      if (this.#tick === undefined) {
        this.#tick = setInterval(() => this.#onTick(), LIMITS.flushMs);
        this.#tick.unref?.();
      }
      if (this.#buffer.length >= LIMITS.flushRecords && !this.#inFlight && !this.#batchesStopped) this.#send(this.#cut(this.#buffer.length));
    } catch {
      // A bug of the capture path costs the record, never the activity.
    }
  }

  #onTick(): void {
    try {
      if (this.#closed || this.#ended || this.#inFlight || this.#buffer.length === 0 || this.#stopped()) return;
      if (processSecrets.full) return this.#dropBuffer('secret_set_full');
      if (this.#state !== 'capturing' || this.#batchesStopped) return;
      this.#send(this.#cut(this.#buffer.length));
    } catch {
      /* the next tick tries again */
    }
  }

  /** Takes the oldest of the first `limit` buffered records that one batch holds (§6.9, "Sending"). */
  #cut(limit: number): Held[] {
    const { workerId, token } = this.host.session();
    let bytes = requestOverhead(this.identity, workerId, token);
    let count = 0;
    while (count < Math.min(limit, LIMITS.batchRecords)) {
      const next = delimited(this.#buffer[count]!.size);
      if (count > 0 && bytes + next > LIMITS.batchBytes) break;
      bytes += next;
      count += 1;
    }
    const held = this.#buffer.splice(0, count);
    for (const one of held) this.#bufferBytes -= one.size;
    return held;
  }

  #release(held: readonly Held[]): void {
    for (const one of held) heldBytes -= one.size;
  }

  #send(held: Held[]): void {
    if (held.length === 0) return;
    const batch: Batch = { held, done: Promise.resolve(), firstSentAt: Date.now() };
    batch.done = this.#deliver(batch).catch(() => undefined).finally(() => {
      this.#release(batch.held);
      if (this.#inFlight === batch) this.#inFlight = undefined;
      if (!this.#closed && !this.#ended && this.#state === 'capturing' && !this.#stopped() && !this.#batchesStopped
        && !processSecrets.full && this.#buffer.length >= LIMITS.flushRecords && !this.#inFlight) {
        this.#send(this.#cut(this.#buffer.length));
      }
    });
    this.#inFlight = batch;
  }

  #request(batch: Batch, session: { workerId: string; token: string }): Record<string, unknown> {
    return {
      worker_id: session.workerId,
      session_token: session.token,
      task_id: this.identity.task_id,
      generation: this.identity.generation,
      lease_token: this.identity.lease_token,
      run_id: this.identity.run_id,
      step_id: this.identity.step_id,
      attempt: this.identity.attempt,
      instance: this.identity.instance,
      records: batch.held.map((one) => one.record),
    };
  }

  /** Sends one batch until an answer settles it (contracts.md §4). */
  async #deliver(batch: Batch): Promise<void> {
    let session = this.host.session();
    let request = this.#request(batch, session);
    let retries = 0;
    let sessionResent = false;
    for (;;) {
      if (this.#closed) return;
      const left = LIMITS.batchAgeMs - (Date.now() - batch.firstSentAt);
      if (left <= 0) return this.#count('send_failed', batch.held.length);
      const outcome = await this.host.append(request, Math.min(this.host.timeoutMs, left));
      if (this.#closed) return;
      if (outcome.kind === 'answer') {
        switch (outcome.verdict) {
          case 'STEP_LOG_VERDICT_CONTINUE':
            return;
          case 'STEP_LOG_VERDICT_FINAL_ONLY':
            this.#finalOnly();
            return;
          case 'STEP_LOG_VERDICT_STOP_ATTEMPT':
            this.#stop();
            return;
          case 'STEP_LOG_VERDICT_STOP_CAPTURE': {
            const now = this.host.session();
            if (outcome.stopReason === 'STEP_LOG_STOP_REASON_SESSION' && !sessionResent && now.token !== session.token) {
              sessionResent = true;
              session = now;
              request = this.#request(batch, session);
              continue;
            }
            this.#stopSession('the platform stopped step-log capture for this session');
            return;
          }
          case 'STEP_LOG_VERDICT_RETRY_LATER':
            await sleep(Math.min(5000, Math.max(50, outcome.retryAfterMs)));
            continue;
          default:
            // An answer this SDK does not know is treated as a failure of the call.
            break;
        }
      } else {
        switch (outcome.status) {
          case 'RESOURCE_EXHAUSTED':
            await sleep(Math.min(5000, Math.max(50, outcome.retryAfterMs ?? 200)));
            continue;
          case 'INVALID_ARGUMENT':
            return this.#count('send_failed', batch.held.length);
          case 'UNAUTHENTICATED':
          case 'PERMISSION_DENIED':
            this.#stopSession('the platform refused step-log capture for this session');
            return;
          case 'UNIMPLEMENTED':
            this.#batchesStopped = true;
            return this.#count('send_failed', batch.held.length);
          case 'CANCELLED':
            // The host stopped: nothing is sent again.
            return this.#count('send_failed', batch.held.length);
          default:
            break;
        }
      }
      if (retries >= LIMITS.retryDelaysMs.length) return this.#count('send_failed', batch.held.length);
      await sleep(jittered(LIMITS.retryDelaysMs[retries]!));
      retries += 1;
    }
  }

  #dropBuffer(reason: StepLogDropReason): void {
    this.#count(reason, this.#buffer.length);
    this.#release(this.#buffer);
    this.#buffer = [];
    this.#bufferBytes = 0;
  }

  #finalOnly(): void {
    if (this.#state !== 'capturing') return;
    this.#state = 'finalOnly';
    this.#dropBuffer('after_ceiling');
  }

  #stop(): void {
    this.#state = 'stopped';
    // Records the server will not take: they are counted as records whose sending failed.
    this.#dropBuffer('send_failed');
  }

  #stopSession(line: string): void {
    this.session.stopped = true;
    this.host.warnOnce(line);
    this.#stop();
  }

  /** The newest buffered records that fit one carry: how many, counted from the end. */
  #carryFits(): number {
    let size = 1 + varintSize(this.#lastSequence);
    let count = 0;
    for (let i = this.#buffer.length - 1; i >= 0 && count < LIMITS.carryRecords; i--) {
      const next = delimited(this.#buffer[i]!.size);
      if (size + next > LIMITS.carryBytes) break;
      size += next;
      count += 1;
    }
    return count;
  }

  /** The handler ended: what its logger captures from now on is nothing. */
  end(): void {
    this.#ended = true;
    if (this.#tick) clearInterval(this.#tick);
    this.#tick = undefined;
  }

  /**
   * The carry of the attempt's report (§6.9, "The carry"), after at most 2 seconds of waiting for its batch in flight
   * and sending what does not fit one carry; undefined when the attempt sends none. Never throws.
   */
  async carry(): Promise<StepLogCarry | undefined> {
    this.end();
    try {
      return await this.#carry();
    } catch {
      return undefined;
    } finally {
      this.#close();
    }
  }

  async #carry(): Promise<StepLogCarry | undefined> {
    if (this.#lastSequence === 0 || this.#stopped()) return undefined;
    const markOnly = (): StepLogCarry => {
      this.#dropBuffer(this.#state === 'finalOnly' ? 'after_ceiling' : 'secret_set_full');
      return { records: [], highest_sequence: this.#lastSequence };
    };
    if (this.#state === 'finalOnly' || processSecrets.full) return markOnly();
    const deadline = Date.now() + LIMITS.reportWaitMs;
    for (;;) {
      if (this.#inFlight) {
        const left = deadline - Date.now();
        if (left <= 0) break;
        let timer: NodeJS.Timeout | undefined;
        const late = await Promise.race([
          this.#inFlight.done.then(() => false),
          new Promise<boolean>((resolve) => {
            timer = setTimeout(() => resolve(true), left);
          }),
        ]);
        clearTimeout(timer);
        if (late) break;
        if (this.#stopped()) return undefined;
        // The answer may have changed the state while the report waited.
        if ((this.#state as AttemptState) === 'finalOnly' || processSecrets.full) return markOnly();
        continue;
      }
      const fits = this.#carryFits();
      if (fits === this.#buffer.length || this.#batchesStopped || Date.now() >= deadline) break;
      this.#send(this.#cut(this.#buffer.length - fits));
    }
    const fits = this.#carryFits();
    const beyond = this.#buffer.length - fits;
    if (beyond > 0) {
      this.#count('report_deadline', beyond);
      this.#release(this.#buffer.slice(0, beyond));
      this.#buffer = this.#buffer.slice(beyond);
    }
    return { records: this.#buffer.map((one) => one.record), highest_sequence: this.#lastSequence };
  }

  /** The attempt is over: nothing more is sent, its records are let go, and its drops are counted once. */
  #close(): void {
    if (this.#closed) return;
    this.#closed = true;
    this.#release(this.#buffer);
    this.#buffer = [];
    this.#bufferBytes = 0;
    for (const [reason, records] of this.#drops) this.host.dropped(reason, records);
    this.#drops.clear();
  }

  /** The attempt makes no report (its lease was lost, or it lapses at shutdown): nothing more is sent. */
  discard(): void {
    this.end();
    this.#close();
  }

  /** The host stops with this attempt still running: its buffered records go once, as one batch, with no resend. */
  async flushOnShutdown(deadlineMs: number): Promise<void> {
    try {
      this.end();
      if (processSecrets.full) return this.#dropBuffer('secret_set_full');
      if (this.#stopped() || this.#state !== 'capturing' || this.#batchesStopped || this.#inFlight || this.#buffer.length === 0) return;
      const held = this.#cut(this.#buffer.length);
      const { workerId, token } = this.host.session();
      const request = this.#request({ held, done: Promise.resolve(), firstSentAt: Date.now() }, { workerId, token });
      await this.host.append(request, Math.max(1, deadlineMs));
      this.#release(held);
    } catch {
      /* the lease lapses with what it held */
    } finally {
      this.#close();
    }
  }

  /** For the tests: what the attempt holds. */
  get buffered(): number {
    return this.#buffer.length;
  }
}

/** A failure of `AppendStepLogs` as the attempt reads it. */
export function outcomeOfError(error: unknown): AppendOutcome {
  if (error instanceof SagatError) return { kind: 'error', status: String(error.status ?? 'UNAVAILABLE'), retryAfterMs: error.retryAfterMs ?? null };
  return { kind: 'error', status: 'UNAVAILABLE', retryAfterMs: null };
}
