#!/usr/bin/env node
// The golden corpus of workflow as code (design 044, Task 044-30). README.md beside this file is normative for the
// notation, the command chain, the body of each command kind and the deterministic primitives; this script is the one
// writer of every other file of the corpus.
//
//   node generate.mjs           writes vectors/ and scenarios/
//   node generate.mjs --check   regenerates in memory and exits 1 naming the first file that differs
//
// Node 20 or later, no dependency: node:crypto for SHA-256, BigInt for the generator, its own RFC 8785 serializer. The
// goldens never come from the Rust code they check: `contract_workflow_as_code::goldens` recomputes them with serde_json
// and sha2. A golden changes only after its diff has been read (AGENTS.md rule 18).
//
// The files of a scenario run (scenarios/<scenario>/ and, for a scenario with more than one run, a subdirectory per
// further run, which holds the same three files and is described by the scenario's function.md):
//
// - commands.json: the run (identifier, workflow type, code digest and version, seed, input) and, per activation k,
//   `window` {afterSeq: S(k - 1), throughSeq: S(k)} (the activation applies the visible events of (S(k - 1), S(k)]),
//   `nowUs` (the time of the workflow_task.started at S(k), which ctx.now() returns), the failed attempts before it,
//   `replayedChainHex` (the chain the completion carries), and the commands. Each command names its number, its kind
//   (the chain's lower-case kind), its target and the step or scope it creates or targets (`node`), and then the fields
//   of the `Command` message of worker.proto, readable: `fragment` (Command.fragment_json, as JSON), `value`
//   (Command.value, as JSON) or `valueBlob` (a value that travels as a blob: a generator that rebuilds it and its
//   descriptor), `sagaScope` (Command.saga_scope), and `typed`, keyed by the member of the `typed` oneof in the proto3
//   JSON mapping, with every CommandValue and the fail command's safe message shown as the JSON value it carries;
//   `body` is B(k) of README.md, `bodySha256` hex(SHA(C(B(k)))), `journalKind` the journal event that records it. A
//   deferral (`deferred`) is a terminal or continue-as-new command that took no number; it is re-emitted later, last.
//   `invisibleJournal` lists the journal rows that are not visible (the gaps in the history's sequences; their kinds
//   are indicative), `observedGraph` the steps with the S(k) that issued them and their outcome's sequence.
// - history.jsonl: the run's visible history, one `HistoryEvent` per line in the proto3 JSON mapping (lowerCamelCase
//   names, enum values by name, int64 as decimal strings, bytes as standard base64, a payload's `json` the RFC 8785
//   bytes of the value, fields at their default value omitted).
// - chain.json: H(k) after each command and after each activation.
//
// Conventions the corpus fixes where design 044 leaves a choice (each function.md repeats the ones it uses):
// - Commands of an activation are in the order the function makes them; an update's accept or reject is made when its
//   update.admitted event is applied, its completion when its handler returns. The SDK then appends, in this order,
//   the consumption of the signals the activation delivered to a handler or a wait, the declared state (the last
//   setState of the activation), and the terminal or continue-as-new command (a re-emitted deferral last).
// - A command's record is one visible event: HISTORY_MARKER_RECORDED for a marker, HISTORY_COMMAND_RECORDED for every
//   other kind, a terminal command included; the stop events that follow a terminal command are not visible.
// - Origins: run.started, workflow_task.started, step, timer, child, approval and saga outcomes are ENGINE; a signal, an
//   update's admission, a cancellation request and the event that satisfies a wait are EXTERNAL; a completion's
//   workflow_task.completed and records are COMPLETION; a cancellation of a step whose attempt has not begun (READY, or
//   waiting on an event, an approval or a gate) is EFFECT, a STEP_OUTCOME_CANCELLED (Decision 044-245); a leased step's
//   cancelled outcome comes later and is ENGINE.
// - Creating transactions journal run.started first: a with-start signal or update follows it (Decision 044-246).

import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const MAJOR = Number(process.versions.node.split('.')[0]);
if (!(MAJOR >= 20)) {
  process.stderr.write(`generate.mjs needs Node 20 or later; this is ${process.version}\n`);
  process.exit(2);
}

const HERE = dirname(fileURLToPath(import.meta.url));
const REPOSITORY = resolve(HERE, '../../../..');
const IR_SCHEMA = JSON.parse(
  readFileSync(join(REPOSITORY, 'specs/001-platform-foundation/contracts/workflow.schema.json'), 'utf8'),
);

// ---------------------------------------------------------------------------------------------------------------
// RFC 8785

function canonicalText(value) {
  if (value === null) return 'null';
  if (value === true) return 'true';
  if (value === false) return 'false';
  switch (typeof value) {
    case 'number':
      if (!Number.isFinite(value)) throw new Error(`RFC 8785 refuses the number ${value}`);
      return String(value); // ECMAScript Number-to-String; -0 prints 0
    case 'string':
      return quote(value);
    case 'object': {
      if (Array.isArray(value)) return `[${value.map(canonicalText).join(',')}]`;
      const keys = Object.keys(value).sort(); // the default sort compares UTF-16 code units
      return `{${keys
        .map((key) => {
          if (value[key] === undefined) throw new Error(`the key ${key} has no value`);
          return `${quote(key)}:${canonicalText(value[key])}`;
        })
        .join(',')}}`;
    }
    default:
      throw new Error(`RFC 8785 has no form for a ${typeof value}`);
  }
}

function quote(text) {
  let out = '"';
  for (let i = 0; i < text.length; i += 1) {
    const unit = text.charCodeAt(i);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const low = text.charCodeAt(i + 1);
      if (!(low >= 0xdc00 && low <= 0xdfff)) throw new Error('RFC 8785 refuses a lone surrogate');
      out += text[i] + text[i + 1];
      i += 1;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) {
      throw new Error('RFC 8785 refuses a lone surrogate');
    } else if (unit === 0x22) out += '\\"';
    else if (unit === 0x5c) out += '\\\\';
    else if (unit === 0x08) out += '\\b';
    else if (unit === 0x0c) out += '\\f';
    else if (unit === 0x0a) out += '\\n';
    else if (unit === 0x0d) out += '\\r';
    else if (unit === 0x09) out += '\\t';
    else if (unit < 0x20) out += `\\u${unit.toString(16).padStart(4, '0')}`;
    else out += text[i];
  }
  return `${out}"`;
}

const C = (value) => Buffer.from(canonicalText(value), 'utf8');
const SHA = (bytes) => createHash('sha256').update(bytes).digest();
const hex = (bytes) => Buffer.from(bytes).toString('hex');
const D = (value) => {
  const bytes = C(value);
  return { sha256: hex(SHA(bytes)), size: bytes.length };
};

// ---------------------------------------------------------------------------------------------------------------
// The command chain and the body of each kind (README.md)

const WORK_KINDS = new Set(['activity', 'script', 'timer_start', 'wait_event', 'child', 'approval']);
const JOURNAL_KIND = {
  activity: 'step.released',
  script: 'step.released',
  child: 'step.released',
  timer_start: 'timer.started',
  wait_event: 'wait.started',
  approval: 'approval.opened',
  timer_cancel: 'step.cancelled',
  step_cancel: 'step.cancel_requested',
  saga_open: 'saga.entered',
  saga_close: 'saga.completed',
  saga_abort: 'saga.aborting',
  marker_patch: 'marker.recorded',
  marker_side_effect: 'marker.recorded',
  set_state: 'state.updated',
  update_accept: 'update.accepted',
  update_reject: 'update.rejected',
  update_complete: 'update.completed',
  consume_signals: 'signal.consumed',
  complete: 'run.stop_requested',
  fail: 'run.stop_requested',
  continue_as_new: 'run.continue_requested',
  acknowledge_cancellation: 'run.stop_requested',
};
const PROFILE = {
  activity: 'codeProfileActivity',
  script: 'codeProfileScript',
  timer_start: 'codeProfileTimer',
  wait_event: 'codeProfileWaitEvent',
  child: 'codeProfileChild',
  approval: 'codeProfileApproval',
};

const utf16Compare = (a, b) => (a < b ? -1 : a > b ? 1 : 0); // JavaScript compares strings by UTF-16 code units

function valueDescriptor(command, value) {
  if (command.valueBlob) return command.valueBlob.descriptor;
  if (value === undefined) throw new Error(`${command.kind} has no value`);
  return D(value);
}

function bodyOf(command) {
  const typed = command.typed ?? {};
  switch (command.kind) {
    case 'activity':
    case 'script':
      return {
        ...command.fragment,
        input: valueDescriptor(command, command.value),
        ...(command.sagaScope ? { scope: command.sagaScope } : {}),
      };
    case 'child':
      return { ...command.fragment, input: valueDescriptor(command, command.value) };
    case 'approval':
      return { ...command.fragment, subject: valueDescriptor(command, command.value) };
    case 'timer_start':
    case 'wait_event':
      return command.fragment;
    case 'timer_cancel':
    case 'step_cancel':
      return { target: typed.target.target };
    case 'saga_open': {
      const open = typed.sagaOpen;
      return {
        scope: open.scope,
        parentScope: open.parentScope ? open.parentScope : null,
        onCompensationFailure: open.onCompensationFailure,
        cancelPolicy: open.cancelPolicy,
      };
    }
    case 'saga_close':
      return { scope: typed.sagaClose.target };
    case 'saga_abort':
      return { scope: typed.sagaAbort.target };
    case 'marker_patch':
      return { markerId: typed.marker.markerId };
    case 'marker_side_effect':
      return { markerId: typed.marker.markerId, value: D(typed.marker.sideEffectValue) };
    case 'set_state':
      return { state: typed.setState };
    case 'update_accept':
      return { admittedSeq: Number(typed.updateDecision.admittedSeq), accepted: true };
    case 'update_reject':
      return {
        admittedSeq: Number(typed.updateDecision.admittedSeq),
        accepted: false,
        reason: typed.updateDecision.rejectionReason,
      };
    case 'update_complete': {
      const result = typed.updateResult;
      if ('result' in result === Boolean(result.failureCode)) {
        throw new Error('update_complete carries exactly one of a result and a failure code');
      }
      if (result.failureCode) return { admittedSeq: Number(result.admittedSeq), failureCode: result.failureCode };
      return { admittedSeq: Number(result.admittedSeq), result: D(result.result) };
    }
    case 'consume_signals':
      return {
        signals: typed.consumeSignals.signals
          .map((s) => ({ name: s.signalName, seqs: s.seqs.map(Number).sort((a, b) => a - b) }))
          .sort((a, b) => utf16Compare(a.name, b.name)),
      };
    case 'complete':
      return { output: valueDescriptor(command, typed.complete) };
    case 'fail':
      return { code: typed.fail.code, message: D(typed.fail.safeMessage) };
    case 'continue_as_new': {
      const next = typed.continueAsNew;
      return {
        input: next.input,
        state: next.state === undefined ? null : next.state,
        version: next.activeVersion ? 'active' : 'same',
      };
    }
    case 'acknowledge_cancellation':
      return {};
    default:
      throw new Error(`no body for the kind ${command.kind}`);
  }
}

const dOf = (seq, kind, target, body) => C({ seq, kind, target, body: hex(SHA(C(body))) });
function extend(chain, d) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(d.length);
  return SHA(Buffer.concat([chain, length, d]));
}

function checkFragment(command) {
  const profile = PROFILE[command.kind];
  if (!profile) {
    if (command.fragment !== undefined) throw new Error(`${command.kind} carries no fragment`);
    return;
  }
  const schema = IR_SCHEMA.$defs[profile];
  for (const key of Object.keys(command.fragment)) {
    if (!(key in schema.properties)) throw new Error(`${profile} has no field ${key}`);
  }
  for (const key of schema.required) {
    if (!(key in command.fragment)) throw new Error(`${profile} requires ${key}`);
  }
  if (command.fragment.type !== schema.properties.type.const) throw new Error(`${profile} has the wrong type`);
}

// ---------------------------------------------------------------------------------------------------------------
// The deterministic primitives (README.md, Technical Plan 1.8)

const MASK = (1n << 64n) - 1n;
const rotl = (x, k) => ((x << BigInt(k)) | (x >> BigInt(64 - k))) & MASK;

function splitMix64(state) {
  let x = state;
  const out = [];
  for (let i = 0; i < 4; i += 1) {
    x = (x + 0x9e3779b97f4a7c15n) & MASK;
    let z = x;
    z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & MASK;
    z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & MASK;
    out.push(z ^ (z >> 31n));
  }
  return out;
}

class Generator {
  constructor(seedHex) {
    const seed = Buffer.from(seedHex, 'hex');
    if (seed.length !== 32) throw new Error('a seed has 32 bytes');
    this.s = [0, 1, 2, 3].map((i) => seed.readBigUInt64LE(i * 8));
    if (this.s.every((w) => w === 0n)) this.s = splitMix64(0n);
    this.calls = 0;
  }

  words() {
    return this.s.map((w) => w.toString(16).padStart(16, '0'));
  }

  next() {
    const [s0, s1, s2, s3] = this.s;
    const result = (rotl((s1 * 5n) & MASK, 7) * 9n) & MASK;
    const t = (s1 << 17n) & MASK;
    let n2 = s2 ^ s0;
    let n3 = s3 ^ s1;
    const n1 = s1 ^ n2;
    const n0 = s0 ^ n3;
    n2 ^= t;
    n3 = rotl(n3, 45);
    this.s = [n0, n1, n2, n3];
    this.calls += 1;
    return result;
  }

  random() {
    return Number(this.next() >> 11n) * 2 ** -53;
  }

  randomInt(n) {
    const bound = BigInt(n);
    if (bound <= 0n || bound > 1n << 32n) throw new Error('0 < n <= 2^32');
    const threshold = ((1n << 64n) - bound) % bound;
    const before = this.calls;
    for (;;) {
      const x = this.next();
      if (x >= threshold) return { value: Number(x % bound), nextCalls: this.calls - before };
    }
  }

  uuid() {
    const bytes = Buffer.alloc(16);
    bytes.writeBigUInt64LE(this.next(), 0);
    bytes.writeBigUInt64LE(this.next(), 8);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    const h = bytes.toString('hex');
    return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
  }
}

const RANDOM_INT_BOUNDS = [1, 2, 3, 6, 10, 1000, 2 ** 31, 2 ** 32];
const SEQUENTIAL_SEED = Buffer.from(Array.from({ length: 32 }, (_, i) => i)).toString('hex');

// The stream every primitives vector and the scenario code-deterministic-primitives draw, from a fresh generator.
function primitiveStream(seedHex) {
  const generator = new Generator(seedHex);
  const random = Array.from({ length: 64 }, () => generator.random());
  const randomInt = RANDOM_INT_BOUNDS.map((n) => {
    const draws = Array.from({ length: 8 }, () => generator.randomInt(n));
    return { n, values: draws.map((d) => d.value), nextCalls: draws.map((d) => d.nextCalls) };
  });
  const uuid = Array.from({ length: 8 }, () => generator.uuid());
  return { random, randomInt, uuid, nextCalls: generator.calls };
}

// ---------------------------------------------------------------------------------------------------------------
// Identifiers and values

const T0 = 1767225600000000n; // 2026-01-01T00:00:00Z, microseconds since the epoch
function uuidFrom(label) {
  const bytes = SHA(Buffer.from(`sagat-golden:${label}`));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const h = bytes.subarray(0, 16).toString('hex');
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}
const digestFor = (workflowType, version) => `sha256:${hex(SHA(Buffer.from(`code:${workflowType}:v${version}`)))}`;
const scriptDigest = (name) => `sha256:${hex(SHA(Buffer.from(`script:${name}`)))}`;
const payload = (value) => {
  checkIntegers(value, 'a history payload');
  return { json: C(value).toString('base64') };
};

// A value too large for the message (Non-functional requirement 044-006), rebuilt from its generator.
function rows(count, label) {
  return { rows: Array.from({ length: count }, (_, i) => ({ i, label: `${label}${i}` })) };
}
const blobValue = (count, label, carrier) => ({
  generator: { rows: count, label },
  carrier,
  descriptor: D(rows(count, label)),
});

// ---------------------------------------------------------------------------------------------------------------
// Commands as the function makes them

const activity = (name, input, options = {}) => ({
  kind: 'activity',
  target: name,
  fragment: { type: 'activity', activity: name, ...options },
  value: input,
});
const runScript = (digest, input, options = {}) => ({
  kind: 'script',
  target: digest,
  fragment: { type: 'script', artifactDigest: digest, ...options },
  value: input,
});
const timer = (durationMs) => ({ kind: 'timer_start', fragment: { type: 'timer', durationMs } });
const waitForEvent = (eventType, correlationKey, timeoutMs, lookbackMs) => ({
  kind: 'wait_event',
  target: eventType,
  fragment: {
    type: 'waitEvent',
    eventType,
    correlationKey,
    timeoutMs,
    ...(lookbackMs === undefined ? {} : { lookbackMs }),
  },
});
const child = (workflowId, input, options = {}) => ({
  kind: 'child',
  target: workflowId,
  fragment: { type: 'child', workflowId, ...options },
  value: input,
});
const approval = (policyName, subject) => ({
  kind: 'approval',
  target: policyName,
  fragment: { type: 'approval', policyName },
  value: subject,
});
const timerCancel = (node) => ({ kind: 'timer_cancel', target: node, typed: { target: { target: node } } });
const stepCancel = (node) => ({ kind: 'step_cancel', target: node, typed: { target: { target: node } } });
const sagaOpen = (options, parentScope) => ({
  kind: 'saga_open',
  typed: {
    sagaOpen: {
      ...(parentScope ? { parentScope } : {}),
      onCompensationFailure: options.onCompensationFailure ?? 'manual_review',
      cancelPolicy: options.cancelPolicy ?? 'compensate_completed',
    },
  },
});
const sagaClose = (scope) => ({ kind: 'saga_close', target: scope, typed: { sagaClose: { target: scope } } });
const sagaAbort = (scope) => ({ kind: 'saga_abort', target: scope, typed: { sagaAbort: { target: scope } } });
const patch = (id) => ({ kind: 'marker_patch', target: id, typed: { marker: { markerId: id } } });
const sideEffect = (id, value) => ({
  kind: 'marker_side_effect',
  target: id,
  typed: { marker: { markerId: id, sideEffectValue: value } },
});
const setState = (state) => ({ kind: 'set_state', target: '', typed: { setState: state } });
const acceptUpdate = (seq) => ({
  kind: 'update_accept',
  target: String(seq),
  typed: { updateDecision: { admittedSeq: String(seq), accepted: true } },
});
const rejectUpdate = (seq, reason) => ({
  kind: 'update_reject',
  target: String(seq),
  typed: { updateDecision: { admittedSeq: String(seq), rejectionReason: reason } },
});
const completeUpdate = (seq, result) => ({
  kind: 'update_complete',
  target: String(seq),
  typed: { updateResult: { admittedSeq: String(seq), result } },
});
const failUpdate = (seq, failureCode) => ({
  kind: 'update_complete',
  target: String(seq),
  typed: { updateResult: { admittedSeq: String(seq), failureCode } },
});
const consumeSignals = (bySignal) => ({
  kind: 'consume_signals',
  target: '',
  typed: {
    consumeSignals: {
      signals: Object.keys(bySignal)
        .sort(utf16Compare)
        .map((name) => ({ signalName: name, seqs: [...bySignal[name]].sort((a, b) => a - b).map(String) })),
    },
  },
});
const complete = (output) => ({ kind: 'complete', target: '', typed: { complete: output } });
const completeBlob = (blob) => ({ kind: 'complete', target: '', valueBlob: blob });
const fail = (code, safeMessage) => ({ kind: 'fail', target: '', typed: { fail: { code, safeMessage } } });
const continueAsNew = (input, state, active) => ({
  kind: 'continue_as_new',
  target: '',
  typed: {
    continueAsNew: { input, ...(state === undefined ? {} : { state }), ...(active ? { activeVersion: true } : {}) },
  },
});
const acknowledgeCancellation = () => ({
  kind: 'acknowledge_cancellation',
  target: '',
  typed: { acknowledgeCancellation: {} },
});

// I-JSON (RFC 7493), which RFC 8785 assumes: a command refuses an integer outside +-(2^53 - 1) anywhere (COMMAND_INVALID).
// Every double of magnitude 2^53 or more is an integer, so no command or scenario value may reach it.
const SAFE = 2 ** 53 - 1;
// A closed object schema of the IR (`$defs/resultSchema`): every declared schema of the corpus is one.
const closed = (properties, required) => ({
  type: 'object',
  properties,
  additionalProperties: false,
  ...(required ? { required } : {}),
});
// A code version's `state.maxBytes` when it declares state without one (Decision 044-257).
const STATE_MAX_BYTES_DEFAULT = 65536;
function checkIntegers(value, where) {
  if (typeof value === 'number') {
    if (Number.isInteger(value) && Math.abs(value) > SAFE) throw new Error(`${where}: ${value} is outside +-(2^53 - 1)`);
  } else if (Array.isArray(value)) {
    value.forEach((item) => checkIntegers(item, where));
  } else if (value && typeof value === 'object') {
    Object.values(value).forEach((item) => checkIntegers(item, where));
  }
}

// The readable form of one command of commands.json and chain.json.
function commandEntry(number, command) {
  checkFragment(command);
  checkIntegers(command, `command ${number} (${command.kind})`);
  const body = bodyOf(command);
  const entry = {};
  if (number) entry.commandNumber = number;
  entry.kind = command.kind;
  entry.target = command.target;
  if (command.node) entry.node = command.node;
  if (command.sagaScope) entry.sagaScope = command.sagaScope;
  if (command.fragment) entry.fragment = command.fragment;
  if (command.value !== undefined) entry.value = command.value;
  if (command.valueBlob) entry.valueBlob = command.valueBlob;
  if (command.typed) entry.typed = command.typed;
  if (command.noop) entry.noop = true;
  if (command.reEmitted) entry.reEmitted = true;
  entry.body = body;
  entry.bodySha256 = hex(SHA(C(body)));
  entry.journalKind = JOURNAL_KIND[command.kind];
  return entry;
}

// ---------------------------------------------------------------------------------------------------------------
// A run: its journal, its activations and its chain

class Run {
  constructor(scenario, label, options) {
    this.scenario = scenario;
    this.label = label;
    this.runId = options.runId ?? uuidFrom(`run:${scenario}/${label}`);
    this.workflowType = options.workflowType;
    this.codeVersion = options.codeVersion ?? 1;
    this.seedHex = options.seedHex ?? hex(SHA(Buffer.from(`seed:${scenario}/${label}`)));
    this.input = options.input;
    this.carriedState = options.carriedState;
    this.predecessorRunId = options.predecessorRunId;
    this.lease = options.lease;
    this.now = options.startUs ?? T0;
    this.seq = 0;
    this.events = [];
    this.hidden = [];
    this.count = 0;
    this.chain = Buffer.alloc(32);
    this.chainCommands = [];
    this.activations = [];
    this.lastStart = 0;
    this.current = null;
    this.failed = [];
    this.nodes = new Map();
    this.requests = [];
    this.expectations = undefined;
    this.declarations = undefined;
    this.digests = new Map();
    const started = { input: payload(options.input) };
    if (options.carriedState !== undefined) started.carriedState = payload(options.carriedState);
    if (options.predecessorRunId) started.predecessorRunId = options.predecessorRunId;
    this.visible('RUN_STARTED', 'ENGINE', 'runStarted', started);
  }

  digest(version) {
    return digestFor(this.workflowType, version);
  }

  advanceMs(ms) {
    this.now += BigInt(ms) * 1000n;
  }

  visible(kind, origin, key, body, commandNumber = 0) {
    this.seq += 1;
    this.now += 137n;
    const event = { seq: String(this.seq), kind: `HISTORY_${kind}`, origin: `HISTORY_ORIGIN_${origin}` };
    if (commandNumber) event.commandNumber = String(commandNumber);
    event.atUs = String(this.now);
    event[key] = body;
    this.events.push(event);
    return this.seq;
  }

  hide(journalKind, detail = {}) {
    this.seq += 1;
    this.now += 137n;
    this.hidden.push({ seq: this.seq, journalKind, ...detail });
    return this.seq;
  }

  // A workflow task is leased: workflow_task.started with its time and its two flags.
  begin(options = {}) {
    if (this.current) throw new Error('a workflow task is already started');
    const flags = {};
    if (options.continueAsNewSuggested) flags.continueAsNewSuggested = true;
    if (options.historyLimitReached) flags.historyLimitReached = true;
    const startedSeq = this.visible('WORKFLOW_TASK_STARTED', 'ENGINE', 'workflowTaskStarted', flags);
    if (options.codeVersion) this.codeVersion = options.codeVersion;
    this.current = {
      startedSeq,
      nowUs: Number(this.now),
      options,
      codeVersion: this.codeVersion,
      replayedChainHex: hex(this.chain),
      commands: [],
      records: [],
    };
    return this.current;
  }

  // One command the function makes, numbered and chained at once.
  cmd(spec) {
    const activation = this.current;
    if (!activation) throw new Error('a command needs an activation');
    const number = ++this.count;
    const command = structuredClone(spec);
    if (WORK_KINDS.has(command.kind)) command.node = `c-${number}`;
    if (command.kind === 'timer_start') command.target = command.node;
    if (command.kind === 'saga_open') {
      const scope = `s-${number}`;
      command.target = scope;
      command.node = scope;
      command.typed.sagaOpen = { scope, ...command.typed.sagaOpen };
    }
    if (['timer_cancel', 'step_cancel', 'saga_close', 'saga_abort'].includes(command.kind)) {
      command.node = command.target;
    }
    const entry = commandEntry(number, command);
    const d = dOf(number, command.kind, command.target, entry.body);
    this.chain = extend(this.chain, d);
    this.chainCommands.push({
      commandNumber: number,
      kind: command.kind,
      target: command.target,
      bodySha256: entry.bodySha256,
      dHex: hex(d),
      chainHex: hex(this.chain),
    });
    activation.commands.push(entry);
    activation.records.push({ number, command });
    if (WORK_KINDS.has(command.kind)) {
      this.nodes.set(command.node, {
        node: command.node,
        kind: command.kind,
        target: command.target,
        commandNumber: number,
        issuedSeq: activation.startedSeq,
      });
    }
    return command.node ?? number;
  }

  // The completion: workflow_task.completed, then the record of each command and its immediate effects.
  finish(options = {}) {
    const activation = this.current;
    if (!activation) throw new Error('no workflow task to complete');
    this.current = null;
    const completed = {};
    if (options.defer) {
      const command = options.defer.command;
      const entry = commandEntry(0, { ...command, ...(options.defer.reEmitted ? { reEmitted: true } : {}) });
      completed.deferred = true;
      completed.deferredKind = `COMMAND_${command.kind.toUpperCase()}`;
      if (command.target) completed.deferredTarget = command.target;
      completed.deferralCause = `CAUSE_${options.defer.cause}`;
      activation.deferred = { ...entry, cause: `CAUSE_${options.defer.cause}` };
    }
    activation.completedSeq = this.visible(
      'WORKFLOW_TASK_COMPLETED',
      'COMPLETION',
      'workflowTaskCompleted',
      completed,
    );
    for (const { number, command } of activation.records) this.record(number, command);
    const out = { k: this.activations.length + 1, codeVersion: activation.codeVersion };
    out.window = { afterSeq: this.lastStart, throughSeq: activation.startedSeq };
    out.nowUs = activation.nowUs;
    for (const key of ['worker', 'cache', 'replayFromSeq', 'notes', 'thrown', 'historyPages']) {
      if (activation.options[key] !== undefined) out[key] = activation.options[key];
    }
    if (this.failed.length) out.failedAttempts = this.failed;
    this.failed = [];
    out.replayedChainHex = activation.replayedChainHex;
    out.appliedThroughSeq = activation.startedSeq;
    out.commands = activation.commands;
    if (activation.deferred) out.deferred = activation.deferred;
    out.outcome = options.defer ? 'WORKFLOW_TASK_ACCEPTED_DEFERRED' : 'WORKFLOW_TASK_ACCEPTED';
    if (options.defer) out.cause = `CAUSE_${options.defer.cause}`;
    out.completedSeq = activation.completedSeq;
    out.chainAfterHex = hex(this.chain);
    out.commandCountAfter = this.count;
    this.activations.push(out);
    this.lastStart = activation.startedSeq;
  }

  record(number, command) {
    if (command.kind === 'marker_patch' || command.kind === 'marker_side_effect') {
      const marker = { markerId: command.typed.marker.markerId };
      if (command.kind === 'marker_side_effect') {
        marker.sideEffect = true;
        marker.value = payload(command.typed.marker.sideEffectValue);
      }
      this.visible('MARKER_RECORDED', 'COMPLETION', 'markerRecorded', marker, number);
    } else {
      const record = { kind: `COMMAND_${command.kind.toUpperCase()}` };
      if (command.target) record.target = command.target;
      if (command.node) record.node = command.node;
      if (command.noop) record.noop = true;
      // The scope of an activity or script, which the SDK compares at replay with the kind and the target (Decision
      // 044-244).
      if (command.sagaScope) record.sagaScope = command.sagaScope;
      this.visible('COMMAND_RECORDED', 'COMPLETION', 'commandRecord', record, number);
    }
    if (command.effect === 'cancelled') {
      this.outcome(
        command.target,
        this.visible('STEP_OUTCOME', 'EFFECT', 'stepOutcome', {
          node: command.target,
          outcome: 'STEP_OUTCOME_CANCELLED',
          errorCode: 'CANCELLED',
        }),
      );
    }
  }

  // A workflow task that fails (the SDK's FailWorkflowTask or the server's timeout).
  failAttempt(cause, detail = {}) {
    const activation = this.current;
    if (!activation) throw new Error('no workflow task to fail');
    this.current = null;
    this.failed.push({
      startedSeq: activation.startedSeq,
      nowUs: activation.nowUs,
      ...(activation.options.worker ? { worker: activation.options.worker } : {}),
      cause: `CAUSE_${cause}`,
      ...detail,
    });
    this.hide('workflow_task.failed', { cause: `CAUSE_${cause}` });
  }

  // The run goes under review after failed attempts; no completion follows in this golden.
  review(cause) {
    this.hide('step.needs_review', { node: '~workflow', code: cause });
    this.activations.push({
      k: this.activations.length + 1,
      codeVersion: this.codeVersion,
      window: { afterSeq: this.lastStart, throughSeq: null },
      failedAttempts: this.failed,
      review: cause,
    });
    this.failed = [];
  }

  outcome(node, seq) {
    const known = this.nodes.get(node);
    if (known) known.outcomeSeq = seq;
  }

  completed(node, output, { lease = true } = {}) {
    if (lease) this.hide('step.leased', { node });
    const seq = this.visible('STEP_OUTCOME', 'ENGINE', 'stepOutcome', {
      node,
      outcome: 'STEP_OUTCOME_COMPLETED',
      output: payload(output),
    });
    this.outcome(node, seq);
    return seq;
  }

  failed_(node, errorCode, { lease = true, retryable = false } = {}) {
    if (lease) this.hide('step.leased', { node });
    const body = { node, outcome: 'STEP_OUTCOME_FAILED', errorCode };
    if (retryable) body.retryable = true;
    const seq = this.visible('STEP_OUTCOME', 'ENGINE', 'stepOutcome', body);
    this.outcome(node, seq);
    return seq;
  }

  cancelled(node) {
    const seq = this.visible('STEP_OUTCOME', 'ENGINE', 'stepOutcome', {
      node,
      outcome: 'STEP_OUTCOME_CANCELLED',
      errorCode: 'CANCELLED',
    });
    this.outcome(node, seq);
    return seq;
  }

  timerFired(node) {
    const seq = this.visible('TIMER_FIRED', 'ENGINE', 'timerFired', { node });
    this.outcome(node, seq);
    return seq;
  }

  waitSatisfied(node, value) {
    const seq = this.visible('WAIT_OUTCOME', 'EXTERNAL', 'waitOutcome', { node, payload: payload(value) });
    this.outcome(node, seq);
    return seq;
  }

  childFinished(node, childRunId, status, output) {
    const seq = this.visible('CHILD_FINISHED', 'ENGINE', 'childFinished', {
      node,
      childRunId,
      status,
      output: payload(output),
    });
    this.outcome(node, seq);
    return seq;
  }

  approvalDecided(node, decision) {
    const seq = this.visible('APPROVAL_DECIDED', 'ENGINE', 'approvalDecided', { node, decision });
    this.outcome(node, seq);
    return seq;
  }

  sagaOutcome(scope, outcome) {
    return this.visible('SAGA_OUTCOME', 'ENGINE', 'sagaOutcome', { scope, outcome });
  }

  signal(name, value) {
    return this.visible('SIGNAL_BUFFERED', 'EXTERNAL', 'signalBuffered', { signalName: name, payload: payload(value) });
  }

  updateAdmitted(name, args) {
    return this.visible('UPDATE_ADMITTED', 'EXTERNAL', 'updateAdmitted', { updateName: name, args: payload(args) });
  }

  cancelRequested(graceMs, origin = 'REQUEST') {
    return this.visible('CANCELLATION_REQUESTED', 'EXTERNAL', 'cancellationRequested', {
      ...(graceMs ? { graceMs: String(graceMs) } : {}),
      cancelOrigin: `CANCEL_ORIGIN_${origin}`,
    });
  }

  files(dir) {
    const commands = {
      scenario: this.scenario,
      run: this.label,
      runId: this.runId,
      workflowType: this.workflowType,
      seedHex: this.seedHex,
      input: this.input,
    };
    if (this.carriedState !== undefined) commands.carriedState = this.carriedState;
    if (this.predecessorRunId) commands.predecessorRunId = this.predecessorRunId;
    commands.codeVersions = Object.fromEntries(
      [...new Set(this.activations.map((a) => a.codeVersion).concat([this.codeVersion]))]
        .sort((a, b) => a - b)
        .map((v) => [`v${v}`, this.digest(v)]),
    );
    // The lease's `state_max_bytes` is the version's `state.maxBytes` (Decision 044-257): 64 KiB when the version declares
    // state without it, 0 when it declares none.
    const declared = this.declarations?.state;
    const stateMaxBytes = declared ? (declared.maxBytes ?? STATE_MAX_BYTES_DEFAULT) : 0;
    if (this.lease?.stateMaxBytes !== undefined) {
      throw new Error(`${this.scenario}/${this.label}: the lease's stateMaxBytes comes from declarations.state.maxBytes`);
    }
    if (this.carriedState !== undefined && C(this.carriedState).length > stateMaxBytes) {
      throw new Error(`${this.scenario}/${this.label}: the carried state is over the version's state.maxBytes`);
    }
    for (const activation of this.activations) {
      for (const command of activation.commands ?? []) {
        if (command.kind === 'set_state' && C(command.typed.setState).length > stateMaxBytes) {
          throw new Error(`${this.scenario}/${this.label}: a set-state over the version's state.maxBytes (${stateMaxBytes})`);
        }
        const carried = command.kind === 'continue_as_new' ? command.typed.continueAsNew.state : undefined;
        if (carried !== undefined && carried !== null && C(carried).length > stateMaxBytes) {
          throw new Error(`${this.scenario}/${this.label}: a carried state over the version's state.maxBytes (${stateMaxBytes})`);
        }
      }
    }
    const lease = { ...(this.lease ?? {}), ...(stateMaxBytes ? { stateMaxBytes } : {}) };
    if (Object.keys(lease).length) commands.lease = lease;
    if (this.declarations) commands.declarations = this.declarations;
    commands.activations = this.activations;
    commands.invisibleJournal = this.hidden;
    commands.observedGraph = [...this.nodes.values()];
    if (this.requests.length) commands.requests = this.requests;
    if (this.expectations) commands.expectations = this.expectations;
    checkIntegers(commands, `${this.scenario}/${this.label}/commands.json`);
    const chain = {
      runId: this.runId,
      activations: this.activations
        .filter((a) => a.chainAfterHex)
        .map((a) => ({
          k: a.k,
          commandCountAfter: a.commandCountAfter,
          chainHex: a.chainAfterHex,
          ...(a.deferred ? { deferredBodySha256: a.deferred.bodySha256 } : {}),
        })),
      commands: this.chainCommands.map(({ commandNumber, chainHex }) => ({ commandNumber, chainHex })),
    };
    return {
      [`${dir}/commands.json`]: pretty(commands),
      [`${dir}/history.jsonl`]: this.events.map((e) => JSON.stringify(e)).join('\n') + '\n',
      [`${dir}/chain.json`]: pretty(chain),
    };
  }
}

const pretty = (value) => `${JSON.stringify(value, null, 2)}\n`;

// ---------------------------------------------------------------------------------------------------------------
// function.md

// A line that starts with two spaces continues the bullet above it.
const bullet = (line) => (line.startsWith('  ') ? line : `- ${line}`);

function functionDoc({ title, proves, code, runs, shows, assumptions }) {
  const lines = [`# ${title}`, '', `What it proves (design 044, Verification Plan): ${proves}`, ''];
  lines.push('## Reference function', '', 'Pseudo-code with the API names of Technical Plan 1.12.', '', '```text');
  lines.push(code.trim(), '```', '');
  if (runs) {
    lines.push('## Runs', '');
    for (const [dir, text] of runs) lines.push(dir === 'main' ? `- this directory: ${text}` : `- \`${dir}/\`: ${text}`);
    lines.push('');
  }
  lines.push('## What the golden shows', '');
  for (const line of shows) lines.push(bullet(line));
  lines.push('');
  lines.push('## Assumptions', '');
  lines.push(
    '- The corpus conventions of `generate.mjs` (command order inside an activation, one visible record per command,',
    '  the origins). Sequences missing from `history.jsonl` are journal rows that are not visible, listed with',
    '  indicative kinds in `commands.json` under `invisibleJournal`.',
  );
  for (const line of assumptions ?? []) lines.push(bullet(line));
  lines.push('');
  return lines.join('\n');
}

// ---------------------------------------------------------------------------------------------------------------
// The scenarios of the Verification Plan

const SCENARIOS = [];
const scenario = (name, build) => SCENARIOS.push({ name, build });

scenario('code-run-lifecycle', (name) => {
  const r = new Run(name, 'main', {
    workflowType: 'order',
    input: { orderId: 'ord-1001', sku: 'SKU-7', quantity: 2, amountCents: 4590 },
  });
  r.begin();
  r.cmd(
    activity(
      'inventory.reserve',
      { sku: 'SKU-7', quantity: 2 },
      { retry: { maxAttempts: 3, initialDelayMs: 1000, strategy: 'exponential' }, timeoutMs: 30000 },
    ),
  );
  r.cmd(activity('payments.authorize', { orderId: 'ord-1001', amountCents: 4590 }, { timeoutMs: 30000 }));
  r.finish();
  r.hide('step.leased', { node: 'c-1' });
  r.advanceMs(40);
  r.completed('c-2', { authorizationId: 'auth-77' });
  r.begin({ notes: 'Only c-2 has completed: all() still waits for c-1, so the completion has no command.' });
  r.advanceMs(3);
  r.completed('c-1', { reservationId: 'res-31' }, { lease: false });
  r.finish();
  r.begin();
  r.cmd(timer(60000));
  r.finish();
  r.advanceMs(60000);
  r.timerFired('c-3');
  r.begin();
  r.cmd(complete({ orderId: 'ord-1001', reservationId: 'res-31', authorizationId: 'auth-77' }));
  r.finish();
  r.hide('run.finished', { status: 'COMPLETED' });
  const doc = functionDoc({
    title: 'code-run-lifecycle',
    proves: 'start, two activities with all(), a timer, completion; the journal and the observed graph.',
    code: `
workflow order(input):
  [reservation, authorization] = await all(
      activity("inventory.reserve", {sku: input.sku, quantity: input.quantity},
               {retry: {maxAttempts: 3, initialDelayMs: 1000, strategy: "exponential"}, timeoutMs: 30000}),
      activity("payments.authorize", {orderId: input.orderId, amountCents: input.amountCents}, {timeoutMs: 30000}))
  await sleep(60000)
  return {orderId: input.orderId, reservationId: reservation.reservationId,
          authorizationId: authorization.authorizationId}`,
    shows: [
      'Activation 1 makes commands 1 and 2. c-2 completes first; activation 2 sees only that outcome and makes no command',
      '  (an empty completion still moves S); c-1 completes while task 2 is started, so it belongs to activation 3.',
      'Activation 3 starts the timer c-3 (a timer\'s target is its own step); activation 4 completes the run.',
      '`observedGraph`: c-1 and c-2 issued at S(1) with no dependency, c-3 issued after both outcomes (its issuedSeq is',
      '  at or after their outcomeSeq), which is the observed graph the Studio draws.',
    ],
  });
  return { doc, runs: [['', r]] };
});

scenario('code-deterministic-primitives', (name) => {
  const r = new Run(name, 'main', { workflowType: 'primitives', seedHex: SEQUENTIAL_SEED, input: { draw: true } });
  const stream = primitiveStream(SEQUENTIAL_SEED);
  const first = r.begin();
  r.cmd(timer(1000));
  r.finish();
  r.advanceMs(1000);
  r.timerFired('c-1');
  const second = r.begin();
  r.cmd(
    complete({
      nowMs: [Math.floor(first.nowUs / 1000), Math.floor(second.nowUs / 1000)],
      random: stream.random,
      randomInt: stream.randomInt.map(({ n, values }) => ({ n, values })),
      uuid: stream.uuid,
    }),
  );
  r.finish();
  r.hide('run.finished', { status: 'COMPLETED' });
  r.expectations = {
    primitivesVector: 'vectors/primitives.json, the seed 000102...1f: the output equals its random, randomInt and uuid',
    nowUs: [first.nowUs, second.nowUs],
  };
  const doc = functionDoc({
    title: 'code-deterministic-primitives',
    proves: 'time, 64 numbers, bounded integers and 8 UUIDs of a fixed seed equal the golden.',
    code: `
workflow primitives(input):
  firstNow = now()
  numbers = [random() repeated 64 times]
  bounded = for n in [1, 2, 3, 6, 10, 1000, 2^31, 2^32]: {n, values: [randomInt(n) repeated 8 times]}
  ids = [uuid() repeated 8 times]
  await sleep(1000)
  secondNow = now()
  return {nowMs: [epochMillis(firstNow), epochMillis(secondNow)], random: numbers, randomInt: bounded, uuid: ids}`,
    shows: [
      'The run\'s seed is 000102...1f (`seedHex`); the output is exactly the stream of that seed in',
      '  `vectors/primitives.json` (the contract test compares them).',
      'now() is the time of the workflow_task.started at S(k): `nowUs` of each activation; the output holds it in',
      '  milliseconds, truncated, because TypeScript\'s Date has milliseconds (Technical Plan 1.8). The second activation\'s',
      '  time differs from the first, and a replay returns the same two values.',
      'Drawing numbers makes no command: only the timer and the completion are commands.',
    ],
    assumptions: [
      'The table asks for 8 bounded integers; the function draws 8 for each of the 8 bounds of the primitives vectors so',
      '  that the scenario and the vectors are one stream.',
    ],
  });
  return { doc, runs: [['', r]] };
});

scenario('code-signals', (name) => {
  const r = new Run(name, 'main', { workflowType: 'cart', input: { cartId: 'cart-9' } });
  r.declarations = { signals: { 'item.added': { maxBytes: 65536 }, checkout: { maxBytes: 65536 } } };
  const early = r.signal('item.added', { sku: 'SKU-1' });
  r.requests.push({
    request: 'POST /runs/{runId}/signals/item.added',
    idempotencyKey: 'sig-1',
    payload: { sku: 'SKU-1' },
    answer: '202',
    journaledSeq: early,
    note: 'sent before the function waits (before the first workflow task): buffered in the inbox',
  });
  r.begin();
  r.cmd(activity('cart.open', { cartId: 'cart-9' }));
  r.cmd(consumeSignals({ 'item.added': [early] }));
  r.finish();
  r.completed('c-1', { opened: true });
  const second = r.signal('item.added', { sku: 'SKU-2' });
  r.requests.push(
    {
      request: 'POST /runs/{runId}/signals/item.added',
      idempotencyKey: 'sig-2',
      payload: { sku: 'SKU-2' },
      answer: '202',
      journaledSeq: second,
    },
    {
      request: 'POST /runs/{runId}/signals/item.added',
      idempotencyKey: 'sig-2',
      payload: { sku: 'SKU-2' },
      answer: '202 with the receipt of the first request with this key',
      journaledSeq: null,
      note: 'a repeated key: journaled once, nothing new in the history',
    },
    {
      request: 'POST /runs/{runId}/signals/coupon.applied',
      idempotencyKey: 'sig-3',
      payload: { code: 'WELCOME' },
      answer: '409 CONFLICT, details.reason SIGNAL_NOT_DECLARED',
      journaledSeq: null,
      note: 'an undeclared name is refused (Requirement 044-014)',
    },
  );
  r.begin();
  r.cmd(consumeSignals({ 'item.added': [second] }));
  r.finish();
  const third = r.signal('item.added', { sku: 'SKU-3' });
  const checkout = r.signal('checkout', { paymentMethod: 'card' });
  r.requests.push(
    { request: 'POST /runs/{runId}/signals/item.added', idempotencyKey: 'sig-4', answer: '202', journaledSeq: third },
    { request: 'POST /runs/{runId}/signals/checkout', idempotencyKey: 'sig-5', answer: '202', journaledSeq: checkout },
  );
  r.begin();
  r.cmd(consumeSignals({ 'item.added': [third], checkout: [checkout] }));
  r.cmd(complete({ cartId: 'cart-9', items: ['SKU-1', 'SKU-2', 'SKU-3'], checkout: { paymentMethod: 'card' } }));
  r.finish();
  r.hide('run.finished', { status: 'COMPLETED' });
  const doc = functionDoc({
    title: 'code-signals',
    proves: 'a signal before the function waits, a repeated key, an undeclared name refused.',
    code: `
workflow cart(input):
  items = []
  onSignal("item.added", (p) => items.push(p.sku))
  await activity("cart.open", {cartId: input.cartId})
  payment = await signal("checkout")
  return {cartId: input.cartId, items: items, checkout: payment}`,
    shows: [
      'item.added is buffered at sequence 2, before the first workflow task: activation 1 delivers it to the handler and',
      '  the SDK consumes it (`consume_signals`, after the function\'s own command).',
      'The second item.added is sent twice with one idempotency key: one signal.buffered (`requests`).',
      'coupon.applied is not declared: refused 409 with SIGNAL_NOT_DECLARED, nothing journaled.',
      'Activation 3 consumes two names in one command, names in ascending UTF-16 order (checkout before item.added).',
    ],
    assumptions: [
      'The HTTP status of an accepted signal is today\'s 202 of `POST /runs/{runId}/signals/{signalName}`.',
    ],
  });
  return { doc, runs: [['', r]] };
});

scenario('code-signal-at-return', (name) => {
  const r = new Run(name, 'main', { workflowType: 'report', input: { reportId: 'rep-5' } });
  r.declarations = { signals: { note: { maxBytes: 4096 } } };
  r.begin();
  r.cmd(activity('report.build', { reportId: 'rep-5' }));
  r.finish();
  r.hide('step.leased', { node: 'c-1' });
  const a = r.signal('note', { text: 'a' });
  r.completed('c-1', { url: 'https://reports.example/rep-5' }, { lease: false });
  const b = r.signal('note', { text: 'b' });
  r.begin({
    notes:
      'The window holds note a, the outcome of c-1, then note b. The function returns after the outcome with the notes it' +
      ' has (a); the SDK applies note b before it sends the terminal command, so the consumption comes first.',
  });
  r.cmd(consumeSignals({ note: [a, b] }));
  const done = complete({ reportId: 'rep-5', url: 'https://reports.example/rep-5', notes: ['a'] });
  const c = r.signal('note', { text: 'c' });
  r.finish({ defer: { command: done, cause: 'UNHANDLED_EVENTS' } });
  r.begin({ notes: 'The SDK applies note c, consumes it, and re-emits the deferred completion last, unchanged.' });
  r.cmd(consumeSignals({ note: [c] }));
  r.cmd({ ...done, reEmitted: true });
  r.finish();
  r.hide('run.finished', { status: 'COMPLETED' });
  const doc = functionDoc({
    title: 'code-signal-at-return',
    proves:
      'a signal that arrives in the window of the activation in which the function returns is applied before the' +
      ' terminal command.',
    code: `
workflow report(input):
  notes = []
  onSignal("note", (p) => notes.push(p.text))
  built = await activity("report.build", {reportId: input.reportId})
  return {reportId: input.reportId, url: built.url, notes: copy(notes)}`,
    shows: [
      'Activation 2: the handler takes note a, the function returns, the handler takes note b, then the SDK sends',
      '  `consume_signals` (both sequences) and, last, `complete`.',
      'Note c commits after the workflow_task.started of task 2: the completion defers `complete` with UNHANDLED_EVENTS',
      '  (its workflow_task.completed carries the deferral; the command takes no number).',
      'Activation 3 consumes note c and re-emits the same `complete` (the same body digest), which takes number 4.',
    ],
  });
  return { doc, runs: [['', r]] };
});

scenario('code-updates', (name) => {
  const r = new Run(name, 'main', { workflowType: 'credit-line', input: { accountId: 'acc-3', limit: 100 } });
  r.declarations = {
    state: {
      schema: closed({}),
      initial: '{}',
      updates: {
        raiseLimit: {
          argsSchema: closed({ amount: { type: 'integer' } }, ['amount']),
          disclosure: 'declared-business-result-v1',
          resultSchema: closed({ limit: { type: 'integer' } }, ['limit']),
        },
      },
    },
  };
  r.begin();
  r.cmd(timer(3600000));
  r.finish();
  const u1 = r.updateAdmitted('raiseLimit', { amount: 50 });
  const u2 = r.updateAdmitted('raiseLimit', { amount: -5 });
  r.begin();
  r.cmd(acceptUpdate(u1));
  r.cmd(activity('limits.apply', { accountId: 'acc-3', amount: 50 }));
  r.cmd(rejectUpdate(u2, 'VALIDATION_FAILED'));
  r.finish();
  r.hide('step.leased', { node: 'c-3' });
  const u3 = r.updateAdmitted('raiseLimit', { amount: 25 });
  r.begin();
  r.cmd(acceptUpdate(u3));
  r.cmd(activity('limits.apply', { accountId: 'acc-3', amount: 25 }));
  r.finish();
  r.completed('c-3', { limit: 150 }, { lease: false });
  r.begin();
  r.cmd(completeUpdate(u1, { limit: 150 }));
  r.finish();
  r.completed('c-6', { limit: 175 });
  r.begin();
  r.cmd(completeUpdate(u3, { limit: 175 }));
  r.finish();
  r.advanceMs(3600000);
  r.timerFired('c-1');
  r.begin();
  r.cmd(complete({ accountId: 'acc-3', limit: 175 }));
  r.finish();
  r.hide('run.finished', { status: 'COMPLETED' });
  r.requests.push(
    {
      request: 'POST /runs/{runId}/updates/raiseLimit',
      body: { updateKey: 'u-1', args: { amount: 50 }, waitMs: 30000 },
      admittedSeq: u1,
      answer: '200 with the result {"limit": 150}',
    },
    {
      request: 'POST /runs/{runId}/updates/raiseLimit',
      body: { updateKey: 'u-2', args: { amount: -5 }, waitMs: 30000 },
      admittedSeq: u2,
      answer: 'REJECTED with the reason VALIDATION_FAILED',
    },
    {
      request: 'POST /runs/{runId}/updates/raiseLimit',
      body: { updateKey: 'u-3', args: { amount: 25 }, waitMs: 100, acceptPending: true },
      admittedSeq: u3,
      answer: '202 with the status ACCEPTED (its handler waits for c-6 past waitMs); later 200 {"limit": 175}',
    },
  );
  const doc = functionDoc({
    title: 'code-updates',
    proves: 'validator, accept, reject, an asynchronous handler with an activity, 200, 202 with acceptPending.',
    code: `
workflow creditLine(input):
  limit = input.limit
  onUpdate("raiseLimit",
      validator: (args) => if args.amount <= 0: throw ValidationFailed,
      handler: async (args) =>
          applied = await activity("limits.apply", {accountId: input.accountId, amount: args.amount})
          limit = applied.limit
          return {limit: applied.limit})
  await sleep(3600000)
  return {accountId: input.accountId, limit: limit}`,
    shows: [
      'Two updates admitted while the function sleeps: the first passes its validator (`update_accept`, target the',
      '  decimal sequence of its update.admitted) and its handler schedules limits.apply; the second fails it',
      '  (`update_reject`). Each decision is made when its update.admitted is applied, before the next event.',
      'The third update is sent with acceptPending: the client gets 202 ACCEPTED while its handler waits for c-6.',
      'Each handler\'s return is `update_complete` with the result as a descriptor; the first client gets 200.',
    ],
    assumptions: [
      'A validator\'s rejection uses VALIDATION_FAILED, of the closed list of rejection reasons (VALIDATION_FAILED,',
      '  HANDLER_NOT_REGISTERED, and the engine\'s RUN_ENDED and RUN_CONTINUED).',
      'The REST answer of a rejected update (status and body) belongs to the REST contract; `requests` states only the',
      '  outcome.',
    ],
  });
  return { doc, runs: [['', r]] };
});

scenario('code-update-with-start', (name) => {
  const r = new Run(name, 'main', { workflowType: 'basket', input: { basketId: 'b-12' } });
  r.declarations = {
    state: {
      schema: closed({}),
      initial: '{}',
      updates: {
        addItem: {
          argsSchema: closed({ sku: { type: 'string', maxLength: 64 } }, ['sku']),
          disclosure: 'declared-business-result-v1',
          resultSchema: closed({ items: { type: 'integer' } }, ['items']),
        },
      },
    },
  };
  const admitted = r.updateAdmitted('addItem', { sku: 'SKU-1' });
  r.begin();
  r.cmd(timer(600000));
  r.cmd(acceptUpdate(admitted));
  r.cmd(completeUpdate(admitted, { items: 1 }));
  r.finish();
  r.advanceMs(600000);
  r.timerFired('c-1');
  r.begin();
  r.cmd(complete({ basketId: 'b-12', items: ['SKU-1'] }));
  r.finish();
  r.hide('run.finished', { status: 'COMPLETED' });
  r.requests.push({
    request: 'Update-with-Start (feature 018) of the code version of basket',
    body: { input: { basketId: 'b-12' }, update: 'addItem', updateKey: 'u-1', args: { sku: 'SKU-1' } },
    admittedSeq: admitted,
    answer: '200 with the result {"items": 1}; the run and the update.admitted commit in one transaction',
  });
  const doc = functionDoc({
    title: 'code-update-with-start',
    proves: 'Update-with-Start on a code version.',
    code: `
workflow basket(input):
  items = []
  onUpdate("addItem",
      validator: (args) => if items contains args.sku: throw ValidationFailed,
      handler: (args) => items.push(args.sku); return {items: length(items)})
  await sleep(600000)
  return {basketId: input.basketId, items: items}`,
    shows: [
      'run.started (1) and update.admitted (2) are written by the creating transaction; activation 1 applies both.',
      'run.started starts the function, which registers its handler and sleeps (command 1); update.admitted then',
      '  runs the validator (command 2, accept) and the synchronous handler (command 3, its result).',
    ],
  });
  return { doc, runs: [['', r]] };
});

scenario('code-declared-state', (name) => {
  const r = new Run(name, 'main', { workflowType: 'fulfilment', input: { orderId: 'ord-77' } });
  r.declarations = {
    state: {
      schema: closed({ status: { type: 'string', maxLength: 64 }, items: { type: 'integer' } }),
      initial: '{}',
      maxBytes: 4096,
      queries: {
        status: {
          argsSchema: closed({}),
          resultSchema: { type: 'string', maxLength: 64 },
          expression: 'state.status',
          disclosure: 'declared-business-result-v1',
        },
      },
    },
  };
  r.begin();
  r.cmd(activity('inventory.reserve', { orderId: 'ord-77' }));
  r.cmd(setState({ status: 'reserving', items: 0 }));
  r.finish();
  r.completed('c-1', { items: 3 });
  r.begin();
  r.cmd(activity('payments.charge', { orderId: 'ord-77' }));
  r.cmd(setState({ status: 'charging', items: 3 }));
  r.finish();
  r.completed('c-3', { receiptId: 'rcpt-4' });
  r.begin();
  r.cmd(setState({ status: 'done', items: 3, receiptId: 'rcpt-4' }));
  r.cmd(complete({ orderId: 'ord-77', receiptId: 'rcpt-4' }));
  r.finish();
  r.hide('run.finished', { status: 'COMPLETED' });
  r.expectations = {
    declaredQueries: [
      { after: 'activation 1 committed, every worker stopped', query: 'status', answer: 'reserving' },
      { after: 'activation 2 committed, every worker stopped', query: 'status', answer: 'charging' },
      { after: 'the run ended', query: 'status', answer: 'done' },
    ],
  };
  const doc = functionDoc({
    title: 'code-declared-state',
    proves: 'declared queries with every worker stopped, and on the ended run.',
    code: `
workflow fulfilment(input):
  setState({status: "reserving", items: 0})
  reserved = await activity("inventory.reserve", {orderId: input.orderId})
  setState({status: "charging", items: reserved.items})
  charged = await activity("payments.charge", {orderId: input.orderId})
  setState({status: "done", items: reserved.items, receiptId: charged.receiptId})
  return {orderId: input.orderId, receiptId: charged.receiptId}`,
    shows: [
      'set_state is chained with the state itself as its body (always inline).',
      'The engine answers the declared query `status` (expression `state.status`) from the committed state with no',
      '  worker running, and after the run ended (`expectations`).',
    ],
    assumptions: [
      'The SDK sends the declared state once per completion, after the function\'s other commands and before a terminal',
      '  command (the last setState of the activation wins, Requirement 044-016).',
    ],
  });
  return { doc, runs: [['', r]] };
});

scenario('code-worker-queries', () => {
  const doc = functionDoc({
    title: 'code-worker-queries',
    proves: 'a worker query from the committed state (pending until Task 044-29).',
    code: `
workflow inventoryCount(input):
  counted = 0
  workerQuery("progress", (args) => {counted: counted})   // the worker-query registration of Task 044-29
  for sku in input.skus:
    await activity("inventory.count", {sku: sku})
    counted = counted + 1
  return {counted: counted}`,
    shows: [
      'Pending: the design keeps this golden `.json.pending` until Task 044-29 builds worker queries; the `PENDING`',
      '  file marks the scenario and the contract test skips it.',
    ],
    assumptions: [
      'Technical Plan 1.12 names no registration call for a worker query; `workerQuery` is a placeholder name.',
    ],
  });
  return {
    doc,
    runs: [],
    extra: {
      PENDING:
        'Pending until Task 044-29 (worker queries). Design 044, Verification Plan: code-worker-queries stays\n' +
        '`.json.pending` until that task lands; this scenario has no commands, history or chain yet.\n',
    },
  };
});

scenario('code-graph-mixed', (name) => {
  const childRunId = uuidFrom(`run:${name}/graph-child`);
  const parent = new Run(name, 'main', { workflowType: 'shipment', input: { batchId: 'batch-42' } });
  parent.begin();
  parent.cmd(
    child(
      'bulk-labels',
      { orders: ['ord-1', 'ord-2', 'ord-3'] },
      { cancelPolicy: 'propagate', businessId: 'labels-batch-42' },
    ),
  );
  parent.finish();
  parent.hide('child.started', { node: 'c-1', childRunId });
  parent.advanceMs(500);
  parent.childFinished('c-1', childRunId, 'COMPLETED', { printed: 3 });
  parent.begin();
  parent.cmd(complete({ batchId: 'batch-42', printed: 3 }));
  parent.finish();
  parent.hide('run.finished', { status: 'COMPLETED' });

  const grandchild = new Run(name, 'grandchild', { workflowType: 'label-print', input: { orderId: 'ord-1' } });
  grandchild.begin();
  grandchild.cmd(activity('labels.render', { orderId: 'ord-1', format: 'zpl' }));
  grandchild.finish();
  grandchild.completed('c-1', { labelId: 'lbl-ord-1' });
  grandchild.begin();
  grandchild.cmd(complete({ orderId: 'ord-1', labelId: 'lbl-ord-1' }));
  grandchild.finish();
  grandchild.hide('run.finished', { status: 'COMPLETED' });
  grandchild.expectations = { parentRunId: childRunId, startedBy: 'the child node `print` of the graph child' };
  parent.expectations = {
    graphChild: {
      runId: childRunId,
      workflowId: 'bulk-labels',
      version: 3,
      document: {
        entrypoint: 'each',
        nodes: [
          { id: 'each', type: 'forEach', note: 'over input.orders, body `print`' },
          { id: 'print', type: 'child', workflowId: 'label-print', input: '{"orderId": item}' },
        ],
      },
    },
  };
  const doc = functionDoc({
    title: 'code-graph-mixed',
    proves: 'a code parent, a graph child, a code grandchild.',
    code: `
workflow shipment(input):                                   // code version of shipment
  result = await child("bulk-labels", {orders: ["ord-1", "ord-2", "ord-3"]},
                       {cancelPolicy: "propagate", businessId: "labels-batch-42"})
  return {batchId: input.batchId, printed: result.printed}

graph bulk-labels (version 3):                              // a graph workflow, no code
  forEach item in input.orders: child node "print" starts label-print with {orderId: item}
  returns {printed: count}

workflow labelPrint(input):                                 // code version of label-print
  label = await activity("labels.render", {orderId: input.orderId, format: "zpl"})
  return {orderId: input.orderId, labelId: label.labelId}`,
    runs: [
      ['main', 'the code parent: one child command, then its child.finished.'],
      ['grandchild', 'the code grandchild of ord-1, started by the graph child\'s child node (one of three).'],
    ],
    shows: [
      'The parent sees only the graph child\'s child.finished; the graph child\'s own journal is not code history.',
      'The grandchild is an ordinary code run whose start version is a code version named by a graph child node',
      '  (Requirement 044-018).',
    ],
    assumptions: [
      'The graph child is described (`expectations.graphChild`), not given as a full document.',
      'A child\'s terminal status in HistoryChildFinished is the string COMPLETED.',
    ],
  });
  return { doc, runs: [['', parent], ['grandchild', grandchild]] };
});

scenario('code-continue-as-new', (name) => {
  const r = new Run(name, 'main', { workflowType: 'subscription', input: { cycle: 1, amountCents: 990 } });
  const subscriptionState = {
    state: { schema: closed({ charged: { type: 'array', items: { type: 'integer' }, maxItems: 64 } }), initial: '{}' },
  };
  r.declarations = subscriptionState;
  r.begin();
  r.cmd(activity('audit.write', { cycle: 1 }));
  r.cmd(activity('billing.charge', { cycle: 1, amountCents: 990 }));
  r.cmd(timer(30000));
  r.finish();
  r.hide('step.leased', { node: 'c-1' });
  r.completed('c-2', { charged: 990 });
  r.begin({ notes: 'billing.charge won the race: the function cancels the losing timer and continues as new.' });
  r.cmd(timerCancel('c-3'));
  const next = continueAsNew({ cycle: 2, amountCents: 990 }, { charged: [990] }, true);
  r.finish({ defer: { command: next, cause: 'OUTSTANDING_OBLIGATIONS' } });
  r.advanceMs(200);
  r.completed('c-1', { written: true }, { lease: false });
  r.begin({ notes: 'audit.write is no longer open: the re-emitted continue-as-new is accepted.' });
  r.cmd({ ...next, reEmitted: true });
  r.finish();
  r.hide('run.continued', { successorRunId: uuidFrom(`run:${name}/successor`) });

  const successor = new Run(name, 'successor', {
    workflowType: 'subscription',
    codeVersion: 2,
    input: { cycle: 2, amountCents: 990 },
    carriedState: { charged: [990] },
    predecessorRunId: r.runId,
  });
  successor.declarations = subscriptionState;
  successor.begin();
  successor.cmd(complete({ cycles: 2, charged: [990] }));
  successor.finish();
  successor.hide('run.finished', { status: 'COMPLETED' });
  successor.expectations = { pinnedVersion: 2, why: 'version "active" resolved to the active code version 2' };

  const moved = new Run(name, 'moved', { workflowType: 'subscription', input: { cycle: 1, amountCents: 990, wait: true } });
  moved.declarations = subscriptionState;
  moved.begin();
  moved.cmd(timer(1000));
  moved.finish();
  moved.hide('run.code_version_moved', { from: 1, to: 2, note: 'a verified move (Technical Plan 7.3), not visible' });
  moved.advanceMs(1000);
  moved.timerFired('c-1');
  moved.begin({ codeVersion: 2 });
  moved.cmd(continueAsNew({ cycle: 2, amountCents: 990 }, undefined, false));
  moved.finish();
  moved.hide('run.continued', {});
  moved.expectations = {
    successorPinnedVersion: 2,
    why: 'version "same" pins the run\'s current code version, the moved-to version 2, not its start version 1',
  };
  const doc = functionDoc({
    title: 'code-continue-as-new',
    proves:
      'a deferred continue-as-new while an activity is open, a losing timer cancelled, transferred state, version' +
      ' active, a moved run continuing on its current version.',
    code: `
workflow subscription(input, carried):
  if input.cycle >= 2: return {cycles: input.cycle, charged: carried.charged}
  if input.wait: await sleep(1000); continueAsNew({cycle: 2, amountCents: input.amountCents})   // run "moved"
  activity("audit.write", {cycle: input.cycle})              // not awaited
  charge = activity("billing.charge", {cycle: input.cycle, amountCents: input.amountCents})
  deadline = sleep(30000)
  winner = await race(charge, deadline)
  cancel(deadline)                                           // the losing timer
  continueAsNew({cycle: input.cycle + 1, amountCents: input.amountCents},
                {version: "active", state: {charged: [winner.charged]}})`,
    runs: [
      ['main', 'cycle 1: the deferred continue-as-new and the cancelled timer.'],
      ['successor', 'cycle 2 on the active version (2): run.started carries the input, the state and the predecessor.'],
      ['moved', 'a run moved from version 1 to 2 that continues as new with version "same".'],
    ],
    shows: [
      'Activation 2 cancels the timer c-3 (command 4, `timer_cancel`) and makes continue-as-new while audit.write (c-1)',
      '  is open: deferred with OUTSTANDING_OBLIGATIONS, no number; the deferral is in its workflow_task.completed.',
      'When c-1 completes, activation 3 re-emits it unchanged; it takes number 5 and is chained then.',
      'The continue-as-new body holds its input and state inline and `version` "active"; the successor\'s',
      '  run.started carries `carriedState` and `predecessorRunId`; the successor has a new seed.',
      'The moved run\'s continue-as-new has `version` "same" and no state (null in the body).',
    ],
    assumptions: [
      'Technical Plan 1.12 names no call that cancels a timer or a step; `cancel(handle)` stands for the SDK\'s',
      '  cancellation of its own step (the commands timer_cancel and step_cancel exist).',
      'A timer cancellation is recorded by one visible event, its command record (journal step.cancelled); no',
      '  separate step outcome is written for it.',
      'run.code_version_moved is not visible (it is not in the visible list of Technical Plan 4.2).',
    ],
  });
  return { doc, runs: [['', r], ['successor', successor], ['moved', moved]] };
});

scenario('code-saga', (name) => {
  const r = new Run(name, 'main', { workflowType: 'booking', input: { orderId: 'ord-9', tripId: 'trip-4' } });
  const options = { onCompensationFailure: 'manual_review', cancelPolicy: 'compensate_completed' };
  r.begin();
  const s1 = r.cmd(sagaOpen(options));
  r.cmd({ ...activity('inventory.reserve', { orderId: 'ord-9' }), sagaScope: s1 });
  r.finish();
  r.completed('c-2', { reservationId: 'res-1' });
  r.begin();
  r.cmd({ ...activity('payments.charge', { orderId: 'ord-9', amountCents: 12000 }), sagaScope: s1 });
  r.finish();
  r.completed('c-3', { chargeId: 'ch-1' });
  r.begin();
  r.cmd(sagaClose(s1));
  const s5 = r.cmd(sagaOpen(options));
  r.cmd({ ...activity('hotel.book', { tripId: 'trip-4' }), sagaScope: s5 });
  r.finish();
  r.completed('c-6', { hotelBookingId: 'h-1' });
  r.begin();
  r.cmd({ ...activity('car.rent', { tripId: 'trip-4', hotelBookingId: 'h-1' }), sagaScope: s5 });
  r.finish();
  r.completed('c-7', { carRentalId: 'car-1' });
  r.begin();
  r.cmd({ ...activity('flight.book', { tripId: 'trip-4' }), sagaScope: s5 });
  r.finish();
  r.failed_('c-8', 'NO_SEATS');
  r.begin({ notes: 'flight.book failed: the body throws, the SDK aborts the scope s-5.' });
  r.cmd(sagaAbort(s5));
  r.finish();
  r.hide('compensation.planned', { scope: s5, entries: ['c-6', 'c-7'] });
  r.hide('step.leased', { node: 'c-7~c', activity: 'car.return' });
  r.hide('step.completed', { node: 'c-7~c' });
  r.hide('step.leased', { node: 'c-6~c', activity: 'hotel.cancel' });
  r.hide('step.completed', { node: 'c-6~c' });
  r.sagaOutcome(s5, 'compensated');
  r.begin({ notes: 'saga() throws SagaCompensated, which the function catches.' });
  r.cmd(complete({ orderId: 'ord-9', order: 'charged', trip: 'compensated' }));
  r.finish();
  r.hide('run.finished', { status: 'COMPLETED' });
  const pair = (node) => ({ node, issuedSeq: r.nodes.get(node).issuedSeq, outcomeSeq: r.nodes.get(node).outcomeSeq });
  r.expectations = {
    compensationOrder: ['c-7~c', 'c-6~c'],
    plannedEntries: [pair('c-6'), pair('c-7')],
    rule:
      'c-7 depends on c-6 because outcomeSeq(c-6) <= issuedSeq(c-7); c-8 failed and is not compensated; c-2 and c-3' +
      ' belong to the closed top-level scope s-1, whose compensations were dropped when it closed',
  };
  const doc = functionDoc({
    title: 'code-saga',
    proves: 'a saga that completes, one that aborts and compensates in reverse dependency order.',
    code: `
workflow booking(input):
  await saga({onCompensationFailure: "manual_review", cancelPolicy: "compensate_completed"}, async (s) =>
      await s.activity("inventory.reserve", {orderId: input.orderId})
      await s.activity("payments.charge", {orderId: input.orderId, amountCents: 12000}))
  try:
    await saga({onCompensationFailure: "manual_review", cancelPolicy: "compensate_completed"}, async (s) =>
        hotel = await s.activity("hotel.book", {tripId: input.tripId})       // envelope: compensateWith hotel.cancel
        await s.activity("car.rent", {tripId: input.tripId, hotelBookingId: hotel.hotelBookingId})
                                                                             // envelope: compensateWith car.return
        await s.activity("flight.book", {tripId: input.tripId}))            // fails NO_SEATS
  catch SagaCompensated:
    return {orderId: input.orderId, order: "charged", trip: "compensated"}`,
    shows: [
      'open saga takes `s-<n>`, n its own command number; activities scheduled in it carry `sagaScope`',
      '  (Command.saga_scope), which their body hashes as `scope`.',
      'The first scope closes (command 4) once its members settled; the second aborts (command 9) after flight.book',
      '  failed. The compensations are derived steps, not visible; the function sees the saga outcome `compensated`.',
      '`expectations.compensationOrder`: car.return before hotel.cancel, from the (issuedSeq, outcomeSeq) pairs.',
    ],
    assumptions: [
      'Closing a scope writes only the close command\'s record (journal saga.completed); a HISTORY_SAGA_OUTCOME',
      '  exists only after an abort (compensated, partial or compensation_failed).',
    ],
  });
  return { doc, runs: [['', r]] };
});

scenario('code-saga-uncertain', (name) => {
  const r = new Run(name, 'main', { workflowType: 'settlement', input: { invoiceId: 'inv-8', amountCents: 5000 } });
  r.begin();
  const s1 = r.cmd(sagaOpen({ onCompensationFailure: 'manual_review', cancelPolicy: 'compensate_completed' }));
  r.cmd({ ...activity('payments.charge', { invoiceId: 'inv-8', amountCents: 5000 }), sagaScope: s1 });
  r.finish();
  r.hide('step.leased', { node: 'c-2' });
  r.hide('step.outcome_unknown', { node: 'c-2', note: 'the worker reported OUTCOME_UNKNOWN (feature 037)' });
  r.hide('step.leased', { node: 'c-2~r', activity: 'payments.lookup' });
  r.hide('step.completed', { node: 'c-2~r', verdict: 'applied' });
  r.completed('c-2', { chargeId: 'ch-77', reconciled: true }, { lease: false });
  r.begin({ notes: 'c-2 is completed with the output of its lookup; the function sees only that outcome.' });
  r.cmd({ ...activity('ledger.post', { invoiceId: 'inv-8', chargeId: 'ch-77' }), sagaScope: s1 });
  r.finish();
  r.hide('step.leased', { node: 'c-3' });
  r.hide('step.outcome_unknown', { node: 'c-3' });
  r.hide('step.leased', { node: 'c-3~r', activity: 'ledger.lookup' });
  r.hide('step.completed', { node: 'c-3~r', verdict: 'not_found' });
  r.hide('step.needs_review', { node: 'c-3', code: 'OUTCOME_UNKNOWN' });
  r.expectations = {
    'c-2': 'unknown outcome reconciled applied: completed with the lookup output',
    'c-3': 'unknown outcome left for an authorized review: no visible event; the function keeps waiting',
    workflowTaskStep: 'WAITING with kind DECIDER; a terminal command would be deferred while c-3 is under review',
  };
  const doc = functionDoc({
    title: 'code-saga-uncertain',
    proves: 'an unknown outcome reconciled applied, and one left for review.',
    code: `
workflow settlement(input):
  await saga({onCompensationFailure: "manual_review", cancelPolicy: "compensate_completed"}, async (s) =>
      charge = await s.activity("payments.charge", {invoiceId: input.invoiceId, amountCents: input.amountCents})
                                                         // envelope: reconcileWith payments.lookup
      await s.activity("ledger.post", {invoiceId: input.invoiceId, chargeId: charge.chargeId}))
                                                         // envelope: reconcileWith ledger.lookup
  return {invoiceId: input.invoiceId}`,
    shows: [
      'payments.charge reports an unknown outcome; its lookup says applied, so c-2 completes with the lookup\'s',
      '  output: one visible STEP_OUTCOME, nothing of the reconciliation.',
      'ledger.post\'s lookup is not conclusive: c-3 waits for a review, nothing is visible, and the run stays open',
      '  with the function waiting (the history ends after activation 2\'s records).',
    ],
    assumptions: ['The kinds in `invisibleJournal` for feature 037\'s reconciliation are indicative.'],
  });
  return { doc, runs: [['', r]] };
});

scenario('code-gated-activity', (name) => {
  const r = new Run(name, 'main', {
    workflowType: 'payouts',
    input: { batchId: 'pay-3' },
    lease: { gatedEntries: ['payouts.send'] },
  });
  r.declarations = { state: { schema: closed({}), initial: '{}', maxBytes: 131072 } };
  r.begin();
  r.cmd(activity('payouts.send', { payoutId: 'p-1', amountCents: 250000 }));
  r.cmd(activity('payouts.send', { payoutId: 'p-2', amountCents: 990000 }));
  r.finish();
  r.hide('approval.opened', { node: 'c-1', policy: 'large-payouts' });
  r.hide('approval.opened', { node: 'c-2', policy: 'large-payouts' });
  r.hide('approval.decided', { node: 'c-1', decision: 'approved' });
  r.completed('c-1', { transferId: 't-1' });
  r.hide('approval.decided', { node: 'c-2', decision: 'rejected' });
  r.failed_('c-2', 'APPROVAL_REJECTED', { lease: false });
  r.begin();
  r.cmd(approval('refund-review', { refundId: 'r-9', amountCents: 1200 }));
  r.finish();
  r.approvalDecided('c-3', 'approved');
  r.begin();
  r.cmd(complete({ batchId: 'pay-3', sent: ['p-1'], rejected: ['p-2'], refund: 'approved' }));
  r.finish();
  r.hide('run.finished', { status: 'COMPLETED' });
  const doc = functionDoc({
    title: 'code-gated-activity',
    proves: 'an activity whose entry requires approval, approved and rejected.',
    code: `
workflow payouts(input):                       // envelope: payouts.send has requiresApproval "large-payouts"
  first = activity("payouts.send", {payoutId: "p-1", amountCents: 250000})
  second = activity("payouts.send", {payoutId: "p-2", amountCents: 990000})
  [a, b] = await allSettled(first, second)
  refund = await approval("refund-review", {refundId: "r-9", amountCents: 1200})
  return {batchId: input.batchId, sent: ["p-1"], rejected: ["p-2"], refund: refund}`,
    shows: [
      'The lease lists `gatedEntries` ["payouts.send"]: the SDK keeps those inputs inline.',
      'The engine holds both steps for approval (not visible); c-1 is approved and completes, c-2 is rejected and',
      '  fails with APPROVAL_REJECTED, which is all the function sees.',
      'An approval command (command 3, target the policy name, node c-3) and its HISTORY_APPROVAL_DECIDED.',
    ],
  });
  return { doc, runs: [['', r]] };
});

scenario('code-cancellation', (name) => {
  const r = new Run(name, 'main', { workflowType: 'report-render', input: { reportId: 'rep-1' } });
  r.begin();
  r.cmd(activity('report.render', { reportId: 'rep-1' }));
  r.finish();
  r.hide('step.leased', { node: 'c-1' });
  r.advanceMs(1000);
  r.cancelRequested(60000);
  r.begin({ notes: 'cancellation.requested is true: the function cancels its activity and runs a cleanup.' });
  r.cmd(stepCancel('c-1'));
  r.cmd(activity('report.cleanup', { reportId: 'rep-1' }));
  r.finish();
  r.cancelled('c-1');
  r.completed('c-3', { removed: true });
  r.begin();
  r.cmd(acknowledgeCancellation());
  r.finish();
  r.hide('run.finished', { status: 'CANCELLED' });

  const grace = new Run(name, 'grace-expires', { workflowType: 'report-render', input: { reportId: 'rep-2', idle: true } });
  grace.begin();
  grace.cmd(timer(604800000));
  grace.finish();
  grace.advanceMs(1000);
  const requested = grace.cancelRequested(5000);
  grace.begin({ notes: 'The function does not look at cancellation: an empty completion.' });
  grace.finish();
  grace.advanceMs(5000);
  grace.hide('run.stop_requested', { stopReason: 'Cancelled', by: 'the cancellation deadline pass' });
  grace.cancelled('c-1');
  grace.hide('run.finished', { status: 'CANCELLED' });
  grace.expectations = {
    cancellationGraceMs: 5000,
    deadlineUs: Number(grace.events.find((e) => Number(e.seq) === requested).atUs) + 5000000,
    end: 'CANCELLED by the deadline pass through the stop drain, which cancels the timer c-1',
  };

  const saga = new Run(name, 'acknowledge-in-saga', { workflowType: 'trip', input: { tripId: 'trip-8' } });
  saga.begin();
  const s1 = saga.cmd(sagaOpen({ onCompensationFailure: 'manual_review', cancelPolicy: 'compensate_completed' }));
  saga.cmd({ ...activity('trip.hold', { tripId: 'trip-8' }), sagaScope: s1 });
  saga.finish();
  saga.completed('c-2', { holdId: 'hold-1' });
  saga.begin();
  saga.cmd(timer(86400000));
  saga.finish();
  saga.advanceMs(1000);
  saga.cancelRequested(60000);
  saga.begin({
    notes:
      'acknowledgeCancellation() inside the open scope: the SDK aborts s-1 and sends the acknowledgement, which the' +
      ' server defers while the scope aborts.',
  });
  saga.cmd(sagaAbort(s1));
  const ack = acknowledgeCancellation();
  saga.finish({ defer: { command: ack, cause: 'OUTSTANDING_OBLIGATIONS' } });
  saga.hide('compensation.planned', { scope: s1, entries: ['c-2'] });
  saga.hide('step.leased', { node: 'c-2~c', activity: 'trip.release' });
  saga.hide('step.completed', { node: 'c-2~c' });
  saga.sagaOutcome(s1, 'compensated');
  saga.begin();
  saga.cmd({ ...ack, reEmitted: true });
  saga.finish();
  saga.cancelled('c-3');
  saga.hide('run.finished', { status: 'CANCELLED' });

  const own = new Run(name, 'own-steps', { workflowType: 'refund-review', input: { refundId: 'r-31' } });
  own.begin();
  own.cmd(waitForEvent('refund.confirmed', 'r-31', 86400000));
  own.cmd(approval('refund-review', { refundId: 'r-31', amountCents: 4200 }));
  own.finish();
  own.advanceMs(1000);
  own.cancelRequested(60000);
  own.begin({ notes: 'The function cancels its own wait and approval; neither is held by a worker.' });
  own.cmd({ ...stepCancel('c-1'), effect: 'cancelled' });
  own.cmd({ ...stepCancel('c-2'), effect: 'cancelled' });
  own.finish();
  own.begin({ notes: 'Both commands reject with CommandCancelled; the function acknowledges.' });
  own.cmd(acknowledgeCancellation());
  own.finish();
  own.hide('run.finished', { status: 'CANCELLED' });
  const doc = functionDoc({
    title: 'code-cancellation',
    proves:
      'cooperative cancellation, the grace that ends the run, an acknowledgement while a saga scope is open, and the' +
      ' cancellation of the function\'s own wait and approval.',
    code: `
workflow reportRender(input):
  if input.idle: await sleep(604800000); return {}            // run "grace-expires": never looks at cancellation
  render = activity("report.render", {reportId: input.reportId})
  await race(render, cancellation.promise)
  if cancellation.requested:
    cancel(render)
    await activity("report.cleanup", {reportId: input.reportId})
    acknowledgeCancellation()

workflow trip(input):                                         // run "acknowledge-in-saga"
  await saga({onCompensationFailure: "manual_review", cancelPolicy: "compensate_completed"}, async (s) =>
      await s.activity("trip.hold", {tripId: input.tripId})   // envelope: compensateWith trip.release
      await race(sleep(86400000), cancellation.promise)
      if cancellation.requested: acknowledgeCancellation())

workflow refundReview(input):                                 // run "own-steps"
  confirmed = waitForEvent("refund.confirmed", input.refundId, {timeoutMs: 86400000})
  review = approval("refund-review", {refundId: input.refundId, amountCents: 4200})
  await race(confirmed, review, cancellation.promise)
  if cancellation.requested:
    cancel(confirmed); cancel(review)
    await allSettled(confirmed, review)                       // each rejects with CommandCancelled
    acknowledgeCancellation()`,
    runs: [
      ['main', 'a cooperative cancellation: the step cancel, a cleanup activity, then the acknowledgement.'],
      ['grace-expires', 'the function ignores the request; the deadline (grace 5,000 ms) ends the run CANCELLED.'],
      ['acknowledge-in-saga', 'the acknowledgement while a scope is open: abort, deferral, compensation, acceptance.'],
      ['own-steps', 'the function cancels its own wait and approval, then acknowledges.'],
    ],
    shows: [
      'run.cancel_requested is visible and waking (EXTERNAL) with its grace and the origin REQUEST.',
      'main: `step_cancel` of the leased c-1 (journal step.cancel_requested); the activity worker reports the',
      '  cancellation and its CANCELLED outcome follows with the origin ENGINE (the command alone never cancels it).',
      'grace-expires: no answer; the deadline pass stops the run; the stop drain\'s cancellation of the timer is',
      '  journaled after the stop began, and no workflow task follows.',
      'acknowledge-in-saga: `saga_abort` then `acknowledge_cancellation`, deferred with OUTSTANDING_OBLIGATIONS while',
      '  the scope aborts; once it is compensated the acknowledgement is re-emitted and accepted (number 5).',
      'acknowledge-in-saga: the record of the scope\'s activity carries its scope (`sagaScope` s-1, Decision 044-244).',
      'own-steps: a cancelled wait and a cancelled approval are each a HISTORY_STEP_OUTCOME with STEP_OUTCOME_CANCELLED',
      '  and the origin EFFECT, appended by the completion that cancelled them (Decision 044-245), which also withdraws',
      '  the open approval request. The effects wake the next workflow task.',
    ],
    assumptions: [
      'The acknowledgement\'s record is visible (HISTORY_COMMAND_RECORDED) although its journal event is',
      '  run.stop_requested, which worker.proto calls not visible for that origin (see the report on the corpus).',
      '`cancel(handle)` stands for the SDK\'s cancellation of its own step (`requestCancellation` in the TypeScript',
      '  runtime).',
    ],
  });
  return {
    doc,
    runs: [['', r], ['grace-expires', grace], ['acknowledge-in-saga', saga], ['own-steps', own]],
  };
});

scenario('code-failures', (name) => {
  const r = new Run(name, 'main', { workflowType: 'checkout', input: { orderId: 'ord-5', amountCents: 2000 } });
  r.begin();
  r.cmd(activity('payments.charge', { orderId: 'ord-5', amountCents: 2000 }));
  r.finish();
  r.failed_('c-1', 'CARD_DECLINED');
  r.begin();
  r.cmd(fail('PAYMENT_DECLINED', 'The card was declined'));
  r.finish();
  r.hide('run.finished', { status: 'FAILED', code: 'PAYMENT_DECLINED' });

  const crash = new Run(name, 'uncaught-exception', { workflowType: 'invoice', input: { invoiceId: 'inv-404' } });
  crash.begin();
  crash.cmd(activity('invoice.load', { invoiceId: 'inv-404' }));
  crash.finish();
  crash.completed('c-1', { invoice: null });
  const backoff = [1000, 2000, 4000, 8000];
  for (let attempt = 0; attempt < 5; attempt += 1) {
    crash.begin();
    crash.advanceMs(12);
    crash.failAttempt('WORKFLOW_TASK_FAILED', { exceptionType: 'InvoiceMissing', consecutiveFailures: attempt + 1 });
    if (attempt < 4) crash.advanceMs(backoff[attempt]);
  }
  crash.review('WORKFLOW_TASK_FAILED');
  const doc = functionDoc({
    title: 'code-failures',
    proves:
      'a workflow failure ends the run; an uncaught exception puts it under review after five consecutive failed' +
      ' workflow tasks.',
    code: `
workflow checkout(input):
  try: await activity("payments.charge", {orderId: input.orderId, amountCents: input.amountCents})
  catch ActivityFailure: throw WorkflowFailure("PAYMENT_DECLINED", "The card was declined")
  return {orderId: input.orderId}

workflow invoice(input):                                     // run "uncaught-exception"
  loaded = await activity("invoice.load", {invoiceId: input.invoiceId})
  if loaded.invoice == null: throw InvoiceMissing()          // not the SDK's WorkflowFailure
  return loaded.invoice`,
    runs: [
      ['main', 'WorkflowFailure: the `fail` command, the run ends FAILED with its code.'],
      ['uncaught-exception', 'five failed workflow tasks, then the review.'],
    ],
    shows: [
      '`fail` hashes its code and the descriptor of the message as a JSON string; the message is never in the history.',
      'uncaught-exception: each attempt is a visible workflow_task.started followed by an invisible',
      '  workflow_task.failed (cause WORKFLOW_TASK_FAILED, exception type InvoiceMissing); the next attempt starts after',
      '  1, 2, 4 and 8 s; after the fifth the run is under review (no completion, the chain stays at command 1).',
    ],
  });
  return { doc, runs: [['', r], ['uncaught-exception', crash]] };
});

scenario('code-nondeterminism', (name) => {
  const r = new Run(name, 'main', { workflowType: 'notify-order', input: { orderId: 'ord-31' } });
  r.begin({ worker: 'A' });
  r.cmd(activity('order.load', { orderId: 'ord-31' }));
  r.finish();
  r.completed('c-1', { email: 'buyer@example.com' });
  r.begin({ worker: 'A' });
  r.cmd(activity('email.send', { orderId: 'ord-31', to: 'buyer@example.com' }));
  r.finish();
  r.completed('c-2', { sent: true });
  r.begin({ worker: 'B' });
  r.advanceMs(5);
  r.failAttempt('NONDETERMINISM', {
    report: {
      commandNumber: 2,
      actualKind: 'COMMAND_ACTIVITY',
      actualTarget: 'sms.send',
      codeDigest: r.digest(1),
    },
    expected: { kind: 'COMMAND_ACTIVITY', target: 'email.send', from: 'the server\'s own record of command 2' },
    replayFromSeq: 1,
  });
  r.review('NONDETERMINISM');

  // The same activity in another saga scope (Decision 044-244): the kind and the target agree, the scope does not.
  const moved = new Run(name, 'scope-moved', { workflowType: 'book-trip', input: { tripId: 'trip-12' } });
  const options = { onCompensationFailure: 'manual_review', cancelPolicy: 'compensate_completed' };
  moved.begin({ worker: 'A' });
  const scope = moved.cmd(sagaOpen(options));
  moved.cmd(activity('hotel.book', { tripId: 'trip-12' }));
  moved.finish();
  moved.completed('c-2', { hotelBookingId: 'h-12' });
  moved.begin({ worker: 'B' });
  moved.advanceMs(5);
  moved.failAttempt('NONDETERMINISM', {
    report: {
      commandNumber: 2,
      actualKind: 'COMMAND_ACTIVITY',
      actualTarget: 'hotel.book',
      actualSagaScope: scope,
      codeDigest: moved.digest(1),
    },
    expected: { kind: 'COMMAND_ACTIVITY', target: 'hotel.book', sagaScope: '', from: 'the server\'s own record of command 2' },
    replayFromSeq: 1,
  });
  moved.review('NONDETERMINISM');
  const doc = functionDoc({
    title: 'code-nondeterminism',
    proves: 'a changed function on replay: the report and the review.',
    code: `
workflow notifyOrder(input):                                 // version 1, which wrote the history
  order = await activity("order.load", {orderId: input.orderId})
  await activity("email.send", {orderId: input.orderId, to: order.email})
  return {orderId: input.orderId}

workflow notifyOrder(input):                                 // the second version, replayed by worker B
  order = await activity("order.load", {orderId: input.orderId})
  await activity("sms.send", {orderId: input.orderId, to: order.email})
  return {orderId: input.orderId}

workflow bookTrip(input):                                    // run "scope-moved", version 1
  await saga({}, async (s) =>                                // s-1
      await activity("hotel.book", {tripId: input.tripId}))  // the run's activity, not the scope's: outside any scope

workflow bookTrip(input):                                    // its second version, replayed by worker B
  await saga({}, async (s) =>                                // s-1
      await s.activity("hotel.book", {tripId: input.tripId}))           // now in s-1`,
    runs: [
      ['main', 'a changed target: email.send became sms.send.'],
      ['scope-moved', 'a changed saga scope: hotel.book moved into the saga s-1, its kind and target unchanged.'],
    ],
    shows: [
      'Worker A ran activations 1 and 2 with the first version. Worker B, cold, takes task 3, replays from journal',
      '  sequence 1 and makes sms.send where the history has the record of email.send at command 2.',
      'The report (FailWorkflowTask, cause NONDETERMINISM): command number 2, actual kind COMMAND_ACTIVITY, actual',
      '  target sms.send, the code digest; no payload. The run goes under review at once.',
      'scope-moved: the record of command 2 has no saga_scope (HistoryCommandRecord.saga_scope empty) and worker B makes',
      '  hotel.book in s-1. The report names command 2, kind COMMAND_ACTIVITY, target hotel.book and actual_saga_scope',
      '  s-1 (Decision 044-244); a report whose scope is outside `^s-[1-9][0-9]{0,9}$` has it dropped by the server.',
    ],
    assumptions: [
      'The code digest names the code, so the second version is registered under the run\'s digest by the conformance',
      '  harness only: it stands for a function that is not deterministic. The report carries the run\'s digest.',
    ],
  });
  return { doc, runs: [['', r], ['scope-moved', moved]] };
});

scenario('code-patch-and-move', (name) => {
  const v1 = 1;
  const v2 = 2;
  const moved = new Run(name, 'main', { workflowType: 'purchase', input: { orderId: 'ord-61' } });
  moved.begin();
  moved.cmd(activity('quote.fetch', { orderId: 'ord-61' }));
  moved.finish();
  const movedChain = hex(moved.chain);
  moved.hide('run.code_version_moved', { from: v1, to: v2, sK: moved.lastStart });
  moved.completed('c-1', { quoteId: 'q-1', amountCents: 8800 });
  moved.begin({ codeVersion: v2 });
  moved.cmd(patch('use-fraud-check'));
  moved.cmd(activity('fraud.check', { orderId: 'ord-61', quoteId: 'q-1' }));
  moved.finish();
  moved.completed('c-3', { risk: 'low' });
  moved.begin();
  moved.cmd(activity('order.place', { orderId: 'ord-61', quoteId: 'q-1' }));
  moved.finish();
  moved.completed('c-4', { placed: true });
  moved.begin();
  moved.cmd(activity('notify.send', { orderId: 'ord-61' }));
  moved.finish();
  moved.completed('c-5', { sent: true });
  moved.begin();
  moved.cmd(timer(86400000));
  moved.finish();
  moved.advanceMs(86400000);
  moved.timerFired('c-6');
  moved.begin();
  moved.cmd(complete({ orderId: 'ord-61', status: 'placed' }));
  moved.finish();
  moved.hide('run.finished', { status: 'COMPLETED' });
  moved.expectations = {
    move: {
      from: v1,
      to: v2,
      replayedThroughActivation: 1,
      chainHex: movedChain,
      verdict: 'SUCCEEDED (applied)',
      note: 'the second version replays activation 1 with the same chain; the run continues on version 2',
    },
  };

  const stays = new Run(name, 'unmatched', { workflowType: 'purchase', input: { orderId: 'ord-62' } });
  stays.begin();
  stays.cmd(activity('quote.fetch', { orderId: 'ord-62' }));
  stays.finish();
  stays.completed('c-1', { quoteId: 'q-2', amountCents: 9100 });
  stays.begin();
  stays.cmd(activity('order.place', { orderId: 'ord-62', quoteId: 'q-2' }));
  stays.finish();
  stays.completed('c-2', { placed: true });
  stays.begin();
  stays.cmd(timer(86400000));
  stays.finish();
  stays.expectations = {
    move: {
      from: v1,
      to: v2,
      verdict: 'SKIPPED REPLAY_MISMATCH',
      firstDifference: 3,
      note:
        'patched("use-fraud-check") is false at command 2 (the history has order.place there), so order.place matches;' +
        ' at command 3 the second version makes notify.send where the history has the timer c-3. The run stays on' +
        ' version 1.',
    },
  };
  const doc = functionDoc({
    title: 'code-patch-and-move',
    proves: 'a patched version moved; a run that does not match stays.',
    code: `
workflow purchase(input):                                    // version 1
  quote = await activity("quote.fetch", {orderId: input.orderId})
  await activity("order.place", {orderId: input.orderId, quoteId: quote.quoteId})
  await sleep(86400000)
  return {orderId: input.orderId, status: "placed"}

workflow purchase(input):                                    // version 2
  quote = await activity("quote.fetch", {orderId: input.orderId})
  if patched("use-fraud-check"):
    await activity("fraud.check", {orderId: input.orderId, quoteId: quote.quoteId})
  await activity("order.place", {orderId: input.orderId, quoteId: quote.quoteId})
  await activity("notify.send", {orderId: input.orderId})    // a change without a patch
  await sleep(86400000)
  return {orderId: input.orderId, status: "placed"}`,
    runs: [
      ['main', 'moved after activation 1, then run on version 2: the patch marker is recorded the first time.'],
      ['unmatched', 'at activation 3 on version 1: the replay check finds the first difference at command 3.'],
    ],
    shows: [
      'main: the move is applied (`expectations.move`); activation 2 on version 2 records the patch marker (command 2,',
      '  HISTORY_MARKER_RECORDED with its command number) and schedules fraud.check.',
      'unmatched: the dry-run or move verdict is REPLAY_MISMATCH with first difference 3; nothing moves.',
    ],
    assumptions: [
      'run.code_version_moved is not visible; `codeVersion` of each activation says which version ran it.',
    ],
  });
  return { doc, runs: [['', moved], ['unmatched', stays]] };
});

scenario('code-cache-eviction', (name) => {
  const r = new Run(name, 'main', { workflowType: 'payment-flow', input: { orderId: 'ord-88' } });
  const generator = new Generator(r.seedHex);
  const requestId = generator.uuid();
  r.begin({ worker: 'A', cache: 'cold', replayFromSeq: 1 });
  r.cmd(activity('payments.charge', { orderId: 'ord-88', requestId }));
  r.finish();
  r.completed('c-1', { chargeId: 'ch-88' });
  r.begin({ worker: 'A', cache: 'hit' });
  r.cmd(sideEffect('fx-rate', { pair: 'EURUSD', rate: 1.0842 }));
  r.cmd(waitForEvent('payment.settled', 'ord-88', 3600000, 60000));
  r.finish();
  r.advanceMs(2000);
  r.waitSatisfied('c-3', { settlementId: 'set-5', amountCents: 4200 });
  const normalize = scriptDigest('settlement-normalize');
  r.begin({
    worker: 'B',
    cache: 'cold',
    replayFromSeq: 1,
    notes:
      'Worker B holds nothing at S(2): it reads the history from sequence 1, replays activations 1 and 2 (the same' +
      ' uuid from the seed, the side effect\'s value from its marker) and carries the replayed chain.',
  });
  r.cmd(runScript(normalize, { settlementId: 'set-5', rate: 1.0842 }));
  r.finish();
  r.completed('c-4', { normalized: true });
  r.begin({ worker: 'B', cache: 'hit' });
  r.cmd(complete({ orderId: 'ord-88', requestId, settlementId: 'set-5' }));
  r.finish();
  r.hide('run.finished', { status: 'COMPLETED' });
  r.expectations = { requestId, from: 'the first uuid() of the run\'s seed' };
  const doc = functionDoc({
    title: 'code-cache-eviction',
    proves: 'a cache eviction and a replay from the start on another worker.',
    code: `
workflow paymentFlow(input):
  requestId = uuid()
  charge = await activity("payments.charge", {orderId: input.orderId, requestId: requestId})
  fx = sideEffect(() => readRate("EURUSD"), {id: "fx-rate"})  // {pair, rate}, recorded once
  settled = await waitForEvent("payment.settled", input.orderId, {timeoutMs: 3600000, lookbackMs: 60000})
  await script("${normalize}",
               {settlementId: settled.settlementId, rate: fx.rate})
  return {orderId: input.orderId, requestId: requestId, settlementId: settled.settlementId}`,
    shows: [
      'Worker A runs activations 1 and 2 (`cache` hit for 2: the lease\'s window start equals what it holds), then',
      '  evicts the run. Worker B takes activation 3 cold, replays from sequence 1, and produces the same chain:',
      '  the uuid comes from the seed, the side effect from its HISTORY_MARKER_RECORDED value.',
      'The wait is satisfied by an event (HISTORY_WAIT_OUTCOME with its payload, EXTERNAL); the script command\'s',
      '  target is the artifact digest.',
    ],
  });
  return { doc, runs: [['', r]] };
});

scenario('code-activation-deadline', (name) => {
  const r = new Run(name, 'main', { workflowType: 'feed', input: { feedId: 'feed-2' } });
  r.begin();
  r.cmd(activity('feed.poll', { feedId: 'feed-2' }));
  r.finish();
  r.completed('c-1', { items: 3 });
  const backoff = [1000, 2000, 4000, 8000];
  for (let attempt = 0; attempt < 5; attempt += 1) {
    r.begin();
    r.advanceMs(2000);
    r.failAttempt('ACTIVATION_DEADLINE', { stoppedAfterMs: 2000, consecutiveFailures: attempt + 1 });
    if (attempt < 4) r.advanceMs(backoff[attempt]);
  }
  r.review('WORKFLOW_TASK_FAILED');
  const doc = functionDoc({
    title: 'code-activation-deadline',
    proves: 'an infinite loop stopped with its cause.',
    code: `
workflow feed(input):
  polled = await activity("feed.poll", {feedId: input.feedId})
  while true: pass                                           // never yields
  return polled`,
    shows: [
      'Each attempt of activation 2 is stopped after 2 s of wall time and failed with ACTIVATION_DEADLINE, counted',
      '  with WORKFLOW_TASK_FAILED: retried after 1, 2, 4 and 8 s, then the run is under review (code',
      '  WORKFLOW_TASK_FAILED) after the fifth consecutive failure.',
    ],
    assumptions: [
      'The review raised by counted failures carries the code WORKFLOW_TASK_FAILED whatever the counted cause; each',
      '  attempt\'s own cause is ACTIVATION_DEADLINE.',
    ],
  });
  return { doc, runs: [['', r]] };
});

scenario('code-history-paging', (name) => {
  const r = new Run(name, 'main', { workflowType: 'meter', input: { meterId: 'm-12' } });
  r.declarations = { signals: { reading: { maxBytes: 1024 }, close: { maxBytes: 1024 } } };
  r.begin({ notes: 'The function registers its handler and waits for close: no command.' });
  r.finish();
  const first = [];
  for (let i = 0; i < 520; i += 1) first.push(r.signal('reading', { value: i % 7 }));
  r.begin();
  r.cmd(consumeSignals({ reading: first }));
  r.finish();
  const second = [];
  for (let i = 520; i < 1040; i += 1) second.push(r.signal('reading', { value: i % 7 }));
  const close = r.signal('close', { reason: 'end of period' });
  const visibleAtLease = r.events.length + 1; // the workflow_task.started below is the last event at the lease
  r.begin({
    worker: 'B',
    cache: 'cold',
    replayFromSeq: 1,
    notes: 'A cold worker reads the visible history in pages of at most 1,024 events and replays each as it arrives.',
  });
  const pages = [];
  for (let i = 0; i < visibleAtLease; i += 1024) {
    const last = Math.min(i + 1024, visibleAtLease) - 1;
    pages.push({
      fromSeq: Number(r.events[i].seq),
      throughSeq: Number(r.events[last].seq),
      events: last - i + 1,
      lastPage: last === visibleAtLease - 1,
    });
  }
  let total = 0;
  for (let i = 0; i < 1040; i += 1) total += i % 7;
  r.cmd(consumeSignals({ reading: second, close: [close] }));
  r.cmd(complete({ meterId: 'm-12', readings: 1040, total }));
  r.finish();
  r.hide('run.finished', { status: 'COMPLETED' });
  r.activations[r.activations.length - 1].historyPages = pages;
  r.expectations = { visibleEvents: r.events.length, visibleAtLastLease: visibleAtLease, readerActivation: 3 };
  const doc = functionDoc({
    title: 'code-history-paging',
    proves: 'a history of more than 1,024 events read in pages.',
    code: `
workflow meter(input):
  readings = 0
  total = 0
  onSignal("reading", (p) => readings = readings + 1; total = total + p.value)
  await signal("close")
  return {meterId: input.meterId, readings: readings, total: total}`,
    shows: [
      `${r.events.length} visible events, 1,041 of them signal.buffered (1,040 readings and close), in three`,
      '  activations; each consumption lists at most 1,000 sequences.',
      `Activation 3 is taken by a cold worker: ReadHistory from sequence 1 returns a page of 1,024 events, then the`,
      `  remaining ${visibleAtLease - 1024} (\`historyPages\`).`,
    ],
    assumptions: [
      'The pages are bounded by the 1,024-event count (the events are small, so the 3 MiB bound does not bind); the',
      '  lease\'s first page (at most 256 KiB of the encoding) is not computed by the corpus.',
    ],
  });
  return { doc, runs: [['', r]] };
});

scenario('code-command-batch-limit', (name) => {
  const r = new Run(name, 'main', { workflowType: 'notify-all', input: { recipients: 300 } });
  r.begin({
    thrown: {
      error: 'CommandBatchLimitExceeded',
      atCall: 257,
      caught: true,
      note: 'the 257th call of the function in this activation; the same point live and in replay',
    },
  });
  const nodes = [];
  for (let i = 1; i <= 256; i += 1) nodes.push(r.cmd(activity('notify.send', { recipient: i })));
  r.finish();
  for (const node of nodes) r.completed(node, { sent: true }, { lease: false });
  r.begin();
  r.cmd(child('notify-fanout', { from: 257, to: 300 }, { cancelPolicy: 'propagate' }));
  r.finish();
  r.childFinished('c-257', uuidFrom(`run:${name}/fanout`), 'COMPLETED', { sent: 44 });
  r.begin();
  r.cmd(complete({ sent: 256, delegated: 44 }));
  r.finish();
  r.hide('run.finished', { status: 'COMPLETED' });
  const doc = functionDoc({
    title: 'code-command-batch-limit',
    proves: 'an activation that would make a 257th command catches CommandBatchLimitExceeded, live and in replay.',
    code: `
workflow notifyAll(input):
  started = []
  overflowFrom = null
  try:
    for i in 1 .. input.recipients: started.push(activity("notify.send", {recipient: i}))
  catch CommandBatchLimitExceeded:
    overflowFrom = length(started) + 1                       // 257
  await all(started)
  await child("notify-fanout", {from: overflowFrom, to: input.recipients}, {cancelPolicy: "propagate"})
  return {sent: length(started), delegated: input.recipients - overflowFrom + 1}`,
    shows: [
      'Activation 1 makes 256 commands; the 257th call throws CommandBatchLimitExceeded into the function, which',
      '  catches it (`thrown`). A replay throws at the same call, so the chain is the same.',
      'The rest moves to a graph child in the next activation (command 257, a new activation\'s share).',
    ],
  });
  return { doc, runs: [['', r]] };
});

scenario('code-blob-values', (name) => {
  const r = new Run(name, 'main', { workflowType: 'warehouse-import', input: { rowCount: 14000 } });
  r.declarations = { signals: { 'audit.note': { maxBytes: 4096 } } };
  r.begin({ worker: 'A', cache: 'cold', replayFromSeq: 1 });
  r.cmd({
    kind: 'activity',
    target: 'warehouse.import',
    fragment: { type: 'activity', activity: 'warehouse.import', timeoutMs: 600000 },
    valueBlob: blobValue(14000, 'row-', 'blob written with WriteBlob under the workflow lease'),
  });
  r.finish();
  r.completed('c-1', { imported: 14000 });
  r.begin({ worker: 'A', cache: 'hit' });
  const output = completeBlob(blobValue(14000, 'out-', 'blob written under the lease, held by the deferral'));
  const note = r.signal('audit.note', { text: 'imported' });
  r.finish({ defer: { command: output, cause: 'UNHANDLED_EVENTS' } });
  r.begin({
    worker: 'B',
    cache: 'cold',
    replayFromSeq: 1,
    notes:
      'Worker B replays activations 1 and 2 without uploading anything: it rebuilds both values and computes their' +
      ' descriptors. It re-emits the deferred completion naming the held blob by its descriptor alone.',
  });
  r.cmd(consumeSignals({ 'audit.note': [note] }));
  r.cmd({
    ...completeBlob(blobValue(14000, 'out-', 'descriptor only: the blob the deferral holds')),
    reEmitted: true,
  });
  r.finish();
  r.hide('run.finished', { status: 'COMPLETED' });
  const doc = functionDoc({
    title: 'code-blob-values',
    proves: 'a blob-carried activity input replayed cold, and a deferred blob output re-emitted by another worker.',
    code: `
workflow warehouseImport(input):
  notes = []
  onSignal("audit.note", (p) => notes.push(p.text))
  rows = {rows: [{i: i, label: "row-" + i} for i in 0 .. input.rowCount - 1]}
  imported = await activity("warehouse.import", rows, {timeoutMs: 600000})
  return {rows: [{i: i, label: "out-" + i} for i in 0 .. input.rowCount - 1]}`,
    shows: [
      'Both values are larger than the 320 KiB message, so they travel as blobs; `valueBlob` gives the generator',
      '  that rebuilds each and its descriptor, which is all the chain hashes.',
      'The completion of activation 2 is deferred (UNHANDLED_EVENTS: audit.note arrived after its start) with its',
      '  output blob held; worker B re-emits it naming that blob by its descriptor and it is accepted as command 3.',
    ],
    assumptions: [
      'The values never appear in the corpus; the contract test rebuilds them from `valueBlob.generator`',
      '  (`{"rows": [{"i": i, "label": label + i}, ...]}`) and checks the descriptors.',
    ],
  });
  return { doc, runs: [['', r]] };
});

// ---------------------------------------------------------------------------------------------------------------
// Vectors

function rfc8785Vectors() {
  const cases = [
    ['key-order-rfc-example', String.raw`{"€":"Euro","\r":"CR","דּ":"Hebrew","1":"One","😀":"Smiley","\u0080":"Control","ö":"Latin"}`],
    ['key-order-utf16-not-code-points', String.raw`{"":1,"𐀀":2,"a":0}`],
    ['key-order-prefixes', '{"ab":1,"a":2,"b":3,"aa":4,"":5}'],
    ['key-order-escaped-keys', String.raw`{"a\nb":1,"a":2,"a\u0000":3}`],
    ['nested-objects-and-arrays', '{"z":[3,{"y":1,"x":[{"b":null,"a":true}]}],"a":{"d":{"c":[]}}}'],
    ['whitespace-removed', '  { "b" : [ 1 , 2 ] ,\n\t"a" : "x" }  '],
    ['empty-object', '{}'],
    ['empty-array', '[]'],
    ['empty-containers-nested', '{"a":[],"b":{},"c":[{}],"d":[[]]}'],
    ['deep-nesting', '[[[[{"a":[[]]}]]]]'],
    ['literals', '[true,false,null]'],
    ['escapes-short-forms', String.raw`"\b\f\n\r\t"`],
    ['escapes-control-characters', String.raw`"\u0000\u0001\u0007\u000b\u000e\u001f"`],
    ['escapes-quote-backslash-solidus', String.raw`"\"\\\/"`],
    ['line-separators-stay-raw', String.raw`"  "`],
    ['delete-and-latin1-stay-raw', String.raw`"\u007f\u0080ÿ"`],
    ['unicode-strings', String.raw`"été 😀 דּ 日本"`],
    ['ascii-printable', JSON.stringify(Array.from({ length: 95 }, (_, i) => String.fromCharCode(32 + i)).join(''))],
    ['numbers-zero-and-minus-zero', '[0,-0,0.0,-0.0,0e10]'],
    ['numbers-integers-near-2-53', '[9007199254740991,9007199254740992,9007199254740993,-9007199254740993]'],
    ['numbers-large-integers', '[1e21,1E+21,999999999999999999999,123456789012345678901,100000000000000000000]'],
    ['numbers-small', '[1e-7,0.000001,1.5e-7,0.1,0.2,0.30000000000000004]'],
    ['numbers-extremes', '[5e-324,-5e-324,1.7976931348623157e308,-1.7976931348623157e308,2.2250738585072014e-308]'],
    [
      'numbers-rfc-8785-samples',
      '[295147905179352830000,9.999999999999997e22,1e23,1.0000000000000001e23,999999999999999700000,' +
        '999999999999999900000,9.999999999999997e-7,333333333.3333332,333333333.33333325,333333333.3333333,' +
        '333333333.3333334,333333333.33333343,-0.0000033333333333333333,1424953923781206.2]',
    ],
    ['numbers-integral-floats', '[1.0,1e2,-2.50,100e-2,12e-1]'],
    ['numbers-exponent-boundaries', '[1e20,1e21,1e-6,1e-7,123e-20,1.5e300,-4.25e-10]'],
    ['mixed-document', '{"orderId":"ord-1","amount":12.50,"tags":["a","é"],"meta":{"😀":true,"z":null}}'],
  ];
  return {
    $comment:
      'RFC 8785 vectors (README.md, Notation): `input` is JSON text, `canonical` its canonical form as text and' +
      ' `canonicalHex` the UTF-8 bytes of that form. Numbers are IEEE 754 doubles printed as ECMAScript prints them.',
    cases: cases.map(([name, input]) => {
      const bytes = C(JSON.parse(input));
      return { name, input, canonical: bytes.toString('utf8'), canonicalHex: hex(bytes) };
    }),
  };
}

function primitivesVectors() {
  const wordOneZero = SHA(Buffer.from('sagat-octopus-orchestrator/044/seed-2'));
  wordOneZero.fill(0, 8, 16);
  const seeds = [
    ['all-zero (replaced by SplitMix64 started at 0)', Buffer.alloc(32).toString('hex')],
    ['sequential 00..1f', SEQUENTIAL_SEED],
    ['all-ff', Buffer.alloc(32, 0xff).toString('hex')],
    ['sha256 of "sagat-octopus-orchestrator/044/seed-1"', hex(SHA(Buffer.from('sagat-octopus-orchestrator/044/seed-1')))],
    ['second state word zero (the first next() is 0)', hex(wordOneZero)],
  ];
  const rejection = [3, 1000].map((n) => {
    const generator = new Generator(hex(wordOneZero));
    const before = new Generator(hex(wordOneZero));
    const draws = [before.next(), before.next()].map((x) => x.toString(16).padStart(16, '0'));
    const { value, nextCalls } = generator.randomInt(n);
    return { seedHex: hex(wordOneZero), n, value, nextCalls, draws };
  });
  return {
    $comment:
      'Deterministic primitives (README.md, Technical Plan 1.8). Per seed: `stateWords`, the four 64-bit words after' +
      ' the all-zero replacement, in hexadecimal; `next`, the first 8 outputs of a fresh generator (they are the draws' +
      ' under random[0..7]); then ONE stream from a fresh generator in this order: `random` (64 values), `randomInt`' +
      ' (for each n in the listed order, 8 values, with the next() calls each consumed), `uuid` (8 values).' +
      ' `rejection`: randomInt(n) as the first call of a fresh generator whose first next() is 0, which is below' +
      ' (2^64 - n) mod n, so it draws again.',
    order: ['next (separately, fresh)', 'random x 64', 'randomInt: n in order, 8 each', 'uuid x 8'],
    seeds: seeds.map(([label, seedHex]) => {
      const fresh = new Generator(seedHex);
      const next = Array.from({ length: 8 }, () => fresh.next().toString(16).padStart(16, '0'));
      const stream = primitiveStream(seedHex);
      return {
        label,
        seedHex,
        stateWords: new Generator(seedHex).words(),
        next,
        random: stream.random,
        randomInt: stream.randomInt,
        uuid: stream.uuid,
        streamNextCalls: stream.nextCalls,
      };
    }),
    rejection,
  };
}

function chainList(name, specs, comment) {
  let chain = Buffer.alloc(32);
  const entries = specs.map((spec, index) => {
    const seq = index + 1;
    const command = structuredClone(spec);
    if (command.kind === 'timer_start') command.target = `c-${seq}`;
    if (command.kind === 'saga_open') {
      command.target = `s-${seq}`;
      command.typed.sagaOpen = { scope: `s-${seq}`, ...command.typed.sagaOpen };
    }
    const entry = commandEntry(seq, command);
    const d = dOf(seq, command.kind, command.target, entry.body);
    chain = extend(chain, d);
    const out = { seq, kind: entry.kind, target: entry.target };
    for (const key of ['sagaScope', 'fragment', 'value', 'typed']) if (key in entry) out[key] = entry[key];
    out.body = entry.body;
    out.bodySha256 = entry.bodySha256;
    out.dHex = hex(d);
    out.chainHex = hex(chain);
    return out;
  });
  return { name, ...(comment ? { $comment: comment } : {}), commands: entries };
}

function chainVectors() {
  const work = [
    activity(
      'payments.charge',
      { orderId: 'ord-1', amount: 12.5, currency: 'EUR', note: 'café ☕ 😀' },
      {
        retry: { maxAttempts: 5, initialDelayMs: 500, maxDelayMs: 60000, strategy: 'exponential', jitter: true },
        timeoutMs: 30000,
        compensateWith: 'payments.refund',
      },
    ),
    runScript(scriptDigest('rows-normalize'), { rows: [1, 2.5, -0.000001, 1.5e-7] }, { retry: { maxAttempts: 2 } }),
    timer(86400000),
    waitForEvent('payment.settled', 'ord-1', 3600000, 60000),
    child('shipping', { orderId: 'ord-1', lines: [] }, { cancelPolicy: 'request_cancel', businessId: 'ship-ord-1' }),
    approval('refund-review', { refundId: 'r-1', amountCents: 125000 }),
    timerCancel('c-3'),
    stepCancel('c-5'),
    patch('use-fraud-check'),
    sideEffect('exchange-rate', { rate: 1.0842, at: '2026-01-01T00:00:00Z' }),
  ];
  const sagas = (input, state) => [
    sagaOpen({ onCompensationFailure: 'manual_review', cancelPolicy: 'compensate_completed' }),
    { ...activity('inventory.reserve', input), sagaScope: 's-1' },
    sagaOpen({ onCompensationFailure: 'fail', cancelPolicy: 'wait_in_flight_then_compensate' }, 's-1'),
    { ...activity('payments.charge', { orderId: 'ord-2', amountCents: 4000 }), sagaScope: 's-3' },
    sagaClose('s-3'),
    sagaAbort('s-1'),
    setState(state),
    consumeSignals({ zeta: [9], alpha: [7, 3], '😀': [12], 'דּ': [11] }),
    fail('PAYMENT_DECLINED', 'The card was declined — try another'),
  ];
  const updates = [
    acceptUpdate(4),
    activity('limits.apply', { accountId: 'acc-1', amount: 50 }),
    rejectUpdate(6, 'VALIDATION_FAILED'),
    completeUpdate(4, { limit: 150 }),
    acceptUpdate(9),
    failUpdate(9, 'RESULT_SCHEMA_VIOLATION'),
    complete({ limit: 150, history: [9007199254740991, 0.1, -0, 1e-7], label: 'ü ' }),
  ];
  const continued = [
    activity('billing.charge', { cycle: 1 }),
    continueAsNew({ cycle: 2 }, { carried: [1, 2] }, true),
  ];
  const acknowledged = [timer(1000), acknowledgeCancellation()];
  const baseState = { status: 'compensating', attempts: 2, flags: [] };
  return {
    $comment:
      'Command chain vectors (README.md, The chain). Each command: `seq`, `kind`, `target`, the Command fields it' +
      ' carries (`fragment`, `value`, `sagaScope`, `typed`, readable as in commands.json), `body` B(k),' +
      ' `bodySha256` hex(SHA(C(B(k)))), `dHex` the bytes of d(k) and `chainHex` H(k). The two `-changed-` lists' +
      ' differ from sagas-state-signals in one value only.',
    lists: [
      chainList('work-commands', work),
      chainList('sagas-state-signals', sagas({ sku: 'SKU-1', quantity: 2 }, baseState)),
      chainList('updates-and-complete', updates),
      chainList('continue-as-new', continued),
      chainList('acknowledge-cancellation', acknowledged),
      chainList(
        'sagas-state-signals-changed-input',
        sagas({ sku: 'SKU-1', quantity: 3 }, baseState),
        'differs from sagas-state-signals only in the input of command 2: the chain differs from command 2 on',
      ),
      chainList(
        'sagas-state-signals-changed-state',
        sagas({ sku: 'SKU-1', quantity: 2 }, { ...baseState, attempts: 3 }),
        'differs from sagas-state-signals only in the set-state body of command 7: the chain differs from command 7 on',
      ),
    ],
  };
}

// ---------------------------------------------------------------------------------------------------------------
// The corpus and its check

function corpus() {
  const files = {
    'vectors/rfc8785.json': pretty(rfc8785Vectors()),
    'vectors/primitives.json': pretty(primitivesVectors()),
    'vectors/chain.json': pretty(chainVectors()),
  };
  for (const { name, build } of SCENARIOS) {
    const built = build(name);
    files[`scenarios/${name}/function.md`] = built.doc;
    for (const [sub, run] of built.runs) {
      Object.assign(files, run.files(sub ? `scenarios/${name}/${sub}` : `scenarios/${name}`));
    }
    for (const [file, text] of Object.entries(built.extra ?? {})) files[`scenarios/${name}/${file}`] = text;
  }
  return files;
}

function onDisk(dir) {
  const out = [];
  if (!existsSync(dir)) return out;
  for (const entry of readdirSync(dir)) {
    if (entry.startsWith('.')) continue; // a file manager's .DS_Store is not part of the corpus
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) out.push(...onDisk(path));
    else out.push(relative(HERE, path).split(sep).join('/'));
  }
  return out;
}

function main() {
  const check = process.argv.includes('--check');
  const files = corpus();
  const existing = [...onDisk(join(HERE, 'vectors')), ...onDisk(join(HERE, 'scenarios'))];
  if (check) {
    const names = [...new Set([...Object.keys(files), ...existing])].sort();
    for (const name of names) {
      const path = join(HERE, name);
      if (!(name in files)) {
        process.stderr.write(`${name}: not produced by generate.mjs (remove it or regenerate)\n`);
        process.exit(1);
      }
      if (!existsSync(path)) {
        process.stderr.write(`${name}: missing (run node generate.mjs)\n`);
        process.exit(1);
      }
      if (readFileSync(path, 'utf8') !== files[name]) {
        process.stderr.write(`${name}: differs from what generate.mjs produces\n`);
        process.exit(1);
      }
    }
    process.stdout.write(`the golden corpus is reproduced: ${names.length} files\n`);
    return;
  }
  for (const name of existing) if (!(name in files)) rmSync(join(HERE, name));
  for (const [name, text] of Object.entries(files)) {
    mkdirSync(dirname(join(HERE, name)), { recursive: true });
    writeFileSync(join(HERE, name), text);
  }
  process.stdout.write(`wrote ${Object.keys(files).length} files\n`);
}

main();
