// The fake workflow server of design 044 (workflow as code), Task 044-30: a WorkerService on 127.0.0.1 that serves one
// scenario run of the golden corpus (`specs/044-workflow-as-code/contracts/goldens/`) to a TypeScript worker under test.
//
// It serves RegisterWorker, PollTasks, StreamTasks (WORKFLOW leases only), ReadHistory, CompleteWorkflowTask and
// FailWorkflowTask; every other procedure is recorded and answered UNIMPLEMENTED (WriteBlob and ReadBlob included, so a
// blob-carried value is checked by its descriptor and carrier, never by its bytes).
//
// The leases follow commands.json in order: for activation k, one lease per failed attempt (`failedAttempts`, each
// bounded by its own `startedSeq`), then the lease that completes it (bounded by S(k) = `window.throughSeq`); an
// activation that ends in review (`review`, no S(k)) has only its failed attempts. Only one lease is outstanding at a
// time: a poll while one is outstanding, or after the last one, is answered with no task.
//
// Each CompleteWorkflowTask is compared with the golden activation: the fence, `applied_through_seq`, the replayed
// chain (commands.json's `replayedChainHex`, itself checked against chain.json when the corpus is loaded), and every
// command (number, kind, fragment bytes as RFC 8785 JSON, value, saga scope, typed member and its fields). A
// difference is recorded in `mismatches` with a precise message and answered WORKFLOW_TASK_REVIEW (the fake then serves
// no further lease, as the real server stops a run under review); a completion that matches is answered with the
// golden outcome (ACCEPTED, or ACCEPTED_DEFERRED with its cause).
//
// Conventions this fake fixes where worker.proto leaves a choice (a test that disagrees sees a mismatch that says so):
// - A terminal or continue-as-new command that the golden defers (`deferred`) is sent the first time as the last of
//   `commands`, numbered as the next command (the SDK cannot know it will be deferred); the server answers
//   ACCEPTED_DEFERRED and the number is not taken.
// - A re-emitted deferral (`reEmitted` in the golden) travels in `deferred_command` with command number 0, as
//   `CompleteWorkflowTaskRequest.deferred_command` says ("the terminal or continue-as-new command the SDK re-emits,
//   without a number"); the golden's number is the one the server gives it when it is accepted.
// - A value the golden carries as a blob (`valueBlob`) must carry a non-empty `blob_id` (any: WriteBlob is not served)
//   or, when the golden's carrier is "descriptor only", no carrier and the descriptor; descriptor fields, when sent,
//   must equal the golden descriptor. An inline value's descriptor fields, when sent, must be D(value). In a re-emitted
//   deferral, any value may also come with no carrier and its descriptor alone (Decision 044-259).
// - A FailWorkflowTask is compared with the golden's failed attempt, `actual_saga_scope` included (empty when the
//   golden's report names none); a scope outside `^s-[1-9][0-9]{0,9}$`, or on a kind other than an activity or a
//   script, is a mismatch.
// - The lease: `task_id` `wt-<k>-<attempt>`, `attempt` counted within the activation from 1, `generation` counted over
//   the run from 1, `gated_entries` and `state_max_bytes` from commands.json's `lease` (empty and 0 otherwise),
//   `visible_count` the visible events at or below the lease's S, `started_at_us` and the two flags from the
//   workflow_task.started at S, the first page from sequence 1 up to S within 256 KiB of the encoding.
//
// The messages are decoded with the loader options of the SDK (`keepCase`, int64 as numbers, enums as names, bytes as
// Buffers, defaults, the oneof's virtual field), so `received` and `history` hold what a client of src/worker/grpc.ts
// would see. The server's receive limit is the worker service's 320 KiB (Non-functional requirement 044-006).

import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import * as grpc from '@grpc/grpc-js';
import * as protoLoader from '@grpc/proto-loader';

const here = dirname(fileURLToPath(import.meta.url));
/** The SDK's copy of worker.proto (this file runs from `dist/test`, like the other tests). */
export const PROTO_PATH = join(here, '..', '..', 'proto', 'worker.proto');
const PACKAGE = 'sagat_octopus_orchestrator.v1';
/** The loader options of src/worker/grpc.ts. */
export const LOADER_OPTIONS: protoLoader.Options = { keepCase: true, longs: Number, enums: String, defaults: true, oneofs: true };

const FIRST_PAGE_BYTES = 256 * 1024;
const PAGE_BYTES = 3 * 1024 * 1024;
const PAGE_EVENTS = 1024;
const RECEIVE_LIMIT = 320 * 1024;
const LEASE_MS = 60_000;
const LEASE_CAP_MS = 120_000;
const RETRY_AFTER_MS = 50;
const ZERO_CHAIN = '0'.repeat(64);
/** A saga scope: `s-<n>`, n the command number of its saga_open (Decision 044-250). */
const SAGA_SCOPE = /^s-[1-9][0-9]{0,9}$/;

// ---------------------------------------------------------------------------------------------------------------
// RFC 8785 (README.md of the corpus: keys sorted by UTF-16 code units, numbers and strings as JSON.stringify prints
// them, a lone surrogate refused)

/** The RFC 8785 canonical JSON of a value, as UTF-8 bytes: C(v) of the corpus README. */
export function canonicalJson(value: unknown): Buffer {
  return Buffer.from(canonicalText(value), 'utf8');
}

function canonicalText(value: unknown): string {
  if (value === null) return 'null';
  if (value === true) return 'true';
  if (value === false) return 'false';
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) throw new Error(`RFC 8785 refuses the number ${value}`);
    return String(value);
  }
  if (typeof value === 'string') {
    if (/[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/.test(value)) {
      throw new Error('RFC 8785 refuses a lone surrogate');
    }
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) return `[${value.map(canonicalText).join(',')}]`;
  if (typeof value === 'object') {
    const object = value as Record<string, unknown>;
    return `{${Object.keys(object)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonicalText(object[key])}`)
      .join(',')}}`;
  }
  throw new Error(`RFC 8785 has no form for a ${typeof value}`);
}

const sha256Hex = (bytes: Uint8Array): string => createHash('sha256').update(bytes).digest('hex');
/** D(v) of the corpus README: the digest and size of the value's canonical bytes. */
export function descriptorOf(value: unknown): { sha256: string; size: number } {
  const bytes = canonicalJson(value);
  return { sha256: sha256Hex(bytes), size: bytes.length };
}

// ---------------------------------------------------------------------------------------------------------------
// The corpus

export interface GoldenBlob { generator: unknown; carrier: string; descriptor: { sha256: string; size: number } }
/** One command of commands.json, in its readable form (generate.mjs, `commandEntry`). */
export interface GoldenCommand {
  commandNumber?: number;
  kind: string;
  target: string;
  node?: string;
  sagaScope?: string;
  fragment?: Record<string, unknown>;
  value?: unknown;
  valueBlob?: GoldenBlob;
  typed?: Record<string, any>;
  noop?: boolean;
  reEmitted?: boolean;
  body?: unknown;
  bodySha256: string;
  journalKind: string;
  cause?: string;
}
export interface GoldenFailedAttempt {
  startedSeq: number;
  nowUs: number;
  worker?: string;
  cause: string;
  exceptionType?: string;
  report?: { commandNumber?: number; actualKind?: string; actualTarget?: string; actualSagaScope?: string; codeDigest?: string };
  [key: string]: unknown;
}
export interface GoldenActivation {
  k: number;
  codeVersion: number;
  window: { afterSeq: number; throughSeq: number | null };
  nowUs?: number;
  replayedChainHex?: string;
  appliedThroughSeq?: number;
  commands?: GoldenCommand[];
  deferred?: GoldenCommand;
  outcome?: string;
  cause?: string;
  completedSeq?: number;
  chainAfterHex?: string;
  commandCountAfter?: number;
  failedAttempts?: GoldenFailedAttempt[];
  review?: string;
  worker?: string;
  cache?: string;
  replayFromSeq?: number;
  historyPages?: Array<{ fromSeq: number; throughSeq: number; events: number; lastPage: boolean }>;
  [key: string]: unknown;
}
export interface GoldenCommands {
  scenario: string;
  run: string;
  runId: string;
  workflowType: string;
  seedHex: string;
  input: unknown;
  codeVersions: Record<string, string>;
  lease?: { gatedEntries?: string[]; stateMaxBytes?: number };
  activations: GoldenActivation[];
  [key: string]: unknown;
}
export interface GoldenChain {
  runId: string;
  activations: Array<{ k: number; commandCountAfter: number; chainHex: string; deferredBodySha256?: string }>;
  commands: Array<{ commandNumber: number; chainHex: string }>;
}
/** A decoded HistoryEvent, in the shape the SDK's loader options give. */
export type HistoryEvent = Record<string, any> & { seq: number; kind: string; body?: string };
export interface GoldenRun {
  directory: string;
  commands: GoldenCommands;
  chain: GoldenChain;
  history: HistoryEvent[];
}

/** The corpus directory: found by walking up from this file to the repository's `specs/`. */
export function defaultCorpusDir(): string {
  let directory = here;
  for (;;) {
    const candidate = join(directory, 'specs', '044-workflow-as-code', 'contracts', 'goldens');
    if (existsSync(join(candidate, 'scenarios'))) return candidate;
    const parent = dirname(directory);
    if (parent === directory) throw new Error(`no golden corpus above ${here}`);
    directory = parent;
  }
}

/** The scenario runs of the corpus: `[scenario, run]` with run `main` for the scenario's own directory; pending ones left out. */
export function listScenarioRuns(corpusDir = defaultCorpusDir()): Array<{ scenario: string; run: string }> {
  const out: Array<{ scenario: string; run: string }> = [];
  const root = join(corpusDir, 'scenarios');
  for (const scenario of readdirSync(root).sort()) {
    const directory = join(root, scenario);
    if (!statSync(directory).isDirectory() || existsSync(join(directory, 'PENDING'))) continue;
    if (existsSync(join(directory, 'commands.json'))) out.push({ scenario, run: 'main' });
    for (const run of readdirSync(directory).sort()) {
      if (statSync(join(directory, run)).isDirectory() && existsSync(join(directory, run, 'commands.json'))) {
        out.push({ scenario, run });
      }
    }
  }
  return out;
}

interface Proto {
  definition: protoLoader.PackageDefinition;
  service: grpc.ServiceDefinition;
  serviceConstructor: grpc.ServiceClientConstructor;
}
let proto: Proto | undefined;
function loadProto(): Proto {
  if (proto) return proto;
  const definition = protoLoader.loadSync(PROTO_PATH, LOADER_OPTIONS);
  const loaded = grpc.loadPackageDefinition(definition) as unknown as {
    sagat_octopus_orchestrator: { v1: { WorkerService: grpc.ServiceClientConstructor } };
  };
  const serviceConstructor = loaded.sagat_octopus_orchestrator.v1.WorkerService;
  proto = { definition, service: serviceConstructor.service, serviceConstructor };
  return proto;
}

/** The WorkerService client constructor over the SDK's worker.proto, with the SDK's loader options. */
export function workerServiceClient(address: string): grpc.Client & Record<string, any> {
  const { serviceConstructor } = loadProto();
  return new serviceConstructor(address, grpc.credentials.createInsecure()) as grpc.Client & Record<string, any>;
}

type FieldDescriptor = { name: string; label: string; type: string; typeName: string };
function descriptor(typeName: string): { field?: FieldDescriptor[]; value?: Array<{ name: string; number: number }> } {
  const entry = loadProto().definition[`${PACKAGE}.${typeName}`] as unknown as { type?: unknown } | undefined;
  if (!entry || !entry.type) throw new Error(`worker.proto has no type ${typeName}`);
  return entry.type as { field?: FieldDescriptor[]; value?: Array<{ name: string; number: number }> };
}
const lowerCamel = (name: string): string => name.replace(/_([a-z0-9])/g, (_, c: string) => c.toUpperCase());

const INT64 = new Set(['TYPE_INT64', 'TYPE_UINT64', 'TYPE_SINT64', 'TYPE_FIXED64', 'TYPE_SFIXED64']);
const INT32 = new Set(['TYPE_INT32', 'TYPE_UINT32', 'TYPE_SINT32', 'TYPE_FIXED32', 'TYPE_SFIXED32']);

/**
 * The proto3 JSON mapping of one message into the loader's object form (snake_case keys, int64 as decimal strings
 * kept exact for the encoder, bytes as Buffers, enums by name). Strict: an unknown field, a malformed int64, a
 * non-standard base64 or an unknown enum name throws with the path, so a corpus defect is loud.
 */
export function fromProto3Json(typeName: string, json: unknown, path = typeName): Record<string, unknown> {
  if (json === null || typeof json !== 'object' || Array.isArray(json)) throw new Error(`${path}: not a JSON object`);
  const fields = descriptor(typeName).field ?? [];
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(json as Record<string, unknown>)) {
    const field = fields.find((f) => lowerCamel(f.name) === key || f.name === key);
    if (!field) throw new Error(`${path}: ${typeName} has no field ${key}`);
    const at = `${path}.${key}`;
    if (field.label === 'LABEL_REPEATED') {
      if (!Array.isArray(value)) throw new Error(`${at}: a repeated field must be an array`);
      out[field.name] = value.map((item, i) => scalarFromJson(field, item, `${at}[${i}]`));
    } else {
      out[field.name] = scalarFromJson(field, value, at);
    }
  }
  return out;
}

function scalarFromJson(field: FieldDescriptor, value: unknown, at: string): unknown {
  if (INT64.has(field.type)) {
    const text = typeof value === 'number' ? String(value) : value;
    if (typeof text !== 'string' || !/^-?\d+$/.test(text)) throw new Error(`${at}: an int64 must be a decimal string`);
    // The SDK decodes int64 as a JavaScript number: a value beyond 2^53 - 1 would arrive rounded.
    if (!Number.isSafeInteger(Number(text)) || BigInt(text) !== BigInt(Number(text))) {
      throw new Error(`${at}: ${text} is beyond what the SDK's loader decodes exactly`);
    }
    return text;
  }
  if (INT32.has(field.type) || field.type === 'TYPE_DOUBLE' || field.type === 'TYPE_FLOAT') {
    if (typeof value !== 'number') throw new Error(`${at}: a number is expected`);
    return value;
  }
  switch (field.type) {
    case 'TYPE_BOOL':
      if (typeof value !== 'boolean') throw new Error(`${at}: a boolean is expected`);
      return value;
    case 'TYPE_STRING':
      if (typeof value !== 'string') throw new Error(`${at}: a string is expected`);
      return value;
    case 'TYPE_BYTES': {
      if (typeof value !== 'string') throw new Error(`${at}: bytes are a base64 string`);
      const bytes = Buffer.from(value, 'base64');
      if (bytes.toString('base64') !== value) throw new Error(`${at}: not standard padded base64`);
      return bytes;
    }
    case 'TYPE_ENUM': {
      const values = descriptor(field.typeName).value ?? [];
      const known = typeof value === 'number' ? values.find((v) => v.number === value) : values.find((v) => v.name === value);
      if (!known) throw new Error(`${at}: ${String(value)} is not a value of ${field.typeName}`);
      return known.name;
    }
    case 'TYPE_MESSAGE':
      return fromProto3Json(field.typeName, value, at);
    default:
      throw new Error(`${at}: the field type ${field.type} is not mapped`);
  }
}

const historyCodec = () => {
  const method = loadProto().service['ReadHistory'];
  if (!method) throw new Error('worker.proto has no ReadHistory');
  return method;
};

/** Reads one scenario run of the corpus and checks that its files agree with each other. */
export function loadGoldenRun(scenario: string, run = 'main', corpusDir = defaultCorpusDir()): GoldenRun {
  const scenarioDir = join(corpusDir, 'scenarios', scenario);
  if (existsSync(join(scenarioDir, 'PENDING'))) throw new Error(`the scenario ${scenario} is pending (its PENDING file)`);
  const directory = run === 'main' ? scenarioDir : join(scenarioDir, run);
  const commands = JSON.parse(readFileSync(join(directory, 'commands.json'), 'utf8')) as GoldenCommands;
  const chain = JSON.parse(readFileSync(join(directory, 'chain.json'), 'utf8')) as GoldenChain;
  const codec = historyCodec();
  const history: HistoryEvent[] = [];
  const lines = readFileSync(join(directory, 'history.jsonl'), 'utf8').split('\n').filter((line) => line.length > 0);
  lines.forEach((line, index) => {
    const where = `${scenario}/${run}/history.jsonl line ${index + 1}`;
    const json = JSON.parse(line) as Record<string, unknown>;
    const wire = fromProto3Json('HistoryEvent', json, where);
    const decoded = codec.responseDeserialize(codec.responseSerialize({ events: [wire] })) as { events: HistoryEvent[] };
    const event = decoded.events[0];
    if (!event) throw new Error(`${where}: the event does not survive its own encoding`);
    const bodies = Object.keys(json).filter((key) => !['seq', 'kind', 'origin', 'commandNumber', 'atUs'].includes(key));
    if (bodies.length !== 1 || lowerCamel(event.body ?? '') !== bodies[0]) {
      throw new Error(`${where}: one member of the oneof body is expected, found ${bodies.join(', ') || 'none'}`);
    }
    const previous = history[history.length - 1];
    if (previous && !(event.seq > previous.seq)) throw new Error(`${where}: the sequences are not increasing`);
    history.push(event);
  });
  checkRun({ directory, commands, chain, history }, `${scenario}/${run}`);
  return { directory, commands, chain, history };
}

function checkRun(golden: GoldenRun, name: string): void {
  const { commands, chain, history } = golden;
  if (chain.runId !== commands.runId) throw new Error(`${name}: chain.json and commands.json name different runs`);
  const started = (seq: number, nowUs: number | undefined, where: string): void => {
    const event = history.find((e) => e.seq === seq);
    if (!event || event.kind !== 'HISTORY_WORKFLOW_TASK_STARTED') {
      throw new Error(`${name}: ${where}: sequence ${seq} is not a visible workflow_task.started`);
    }
    if (nowUs !== undefined && event['at_us'] !== nowUs) {
      throw new Error(`${name}: ${where}: nowUs ${nowUs} is not the atUs ${event['at_us']} of sequence ${seq}`);
    }
  };
  let chainBefore = ZERO_CHAIN;
  for (const activation of commands.activations) {
    const where = `activation ${activation.k}`;
    for (const failed of activation.failedAttempts ?? []) started(failed.startedSeq, failed.nowUs, `${where}, a failed attempt`);
    if (activation.window.throughSeq === null) continue;
    started(activation.window.throughSeq, activation.nowUs, where);
    if (activation.replayedChainHex !== chainBefore) {
      throw new Error(`${name}: ${where}: replayedChainHex ${activation.replayedChainHex} is not chain.json's ${chainBefore}`);
    }
    const entry = chain.activations.find((a) => a.k === activation.k);
    if (!entry || entry.chainHex !== activation.chainAfterHex || entry.commandCountAfter !== activation.commandCountAfter) {
      throw new Error(`${name}: ${where}: chain.json does not hold the chain after it`);
    }
    for (const command of activation.commands ?? []) {
      if (command.commandNumber === undefined) continue;
      const recorded = chain.commands.find((c) => c.commandNumber === command.commandNumber);
      if (!recorded) throw new Error(`${name}: ${where}: chain.json has no command ${command.commandNumber}`);
    }
    chainBefore = entry.chainHex;
  }
}

// ---------------------------------------------------------------------------------------------------------------
// Golden commands on the wire

const WORK_KINDS = new Set(['activity', 'script', 'child', 'approval']);
const wireKind = (kind: string): string => `COMMAND_${kind.toUpperCase()}`;

/** The typed member (`Command.typed`, snake_case) that carries a golden command of this kind, if any. */
function typedMember(command: GoldenCommand): string | undefined {
  switch (command.kind) {
    case 'timer_cancel':
    case 'step_cancel':
      return 'target';
    case 'saga_open':
      return 'saga_open';
    case 'saga_close':
      return 'saga_close';
    case 'saga_abort':
      return 'saga_abort';
    case 'marker_patch':
    case 'marker_side_effect':
      return 'marker';
    case 'set_state':
      return 'set_state';
    case 'update_accept':
    case 'update_reject':
      return 'update_decision';
    case 'update_complete':
      return 'update_result';
    case 'consume_signals':
      return 'consume_signals';
    case 'complete':
      return 'complete';
    case 'fail':
      return 'fail';
    case 'continue_as_new':
      return 'continue_as_new';
    case 'acknowledge_cancellation':
      return 'acknowledge_cancellation';
    default:
      return undefined;
  }
}

/** A blob id the scripted clients use for a golden blob value (the fake accepts any non-empty id). */
export const goldenBlobId = (blob: GoldenBlob): string => `golden-blob-${blob.descriptor.sha256.slice(0, 16)}`;

function wireValue(blob: GoldenBlob | undefined, value: unknown): Record<string, unknown> {
  if (blob) {
    const descriptor = { descriptor_sha256: blob.descriptor.sha256, descriptor_size: blob.descriptor.size };
    return /^descriptor only/.test(blob.carrier) ? descriptor : { blob_id: goldenBlobId(blob), ...descriptor };
  }
  return { json: canonicalJson(value) };
}

/** The `Command` a correct SDK sends for a golden command, numbered `commandNumber` (0 for a deferred command). */
export function goldenCommandToWire(command: GoldenCommand, commandNumber = command.commandNumber ?? 0): Record<string, unknown> {
  const wire: Record<string, unknown> = { command_number: commandNumber, kind: wireKind(command.kind) };
  if (command.fragment) wire['fragment_json'] = canonicalJson(command.fragment);
  if (WORK_KINDS.has(command.kind)) wire['value'] = wireValue(command.valueBlob, command.value);
  if (command.sagaScope) wire['saga_scope'] = command.sagaScope;
  const typed = command.typed ?? {};
  switch (typedMember(command)) {
    case 'target':
      wire['target'] = { target: typed['target'].target };
      break;
    case 'saga_open': {
      const open = typed['sagaOpen'];
      wire['saga_open'] = {
        scope: open.scope,
        parent_scope: open.parentScope ?? '',
        on_compensation_failure: open.onCompensationFailure,
        cancel_policy: open.cancelPolicy,
      };
      break;
    }
    case 'saga_close':
      wire['saga_close'] = { target: typed['sagaClose'].target };
      break;
    case 'saga_abort':
      wire['saga_abort'] = { target: typed['sagaAbort'].target };
      break;
    case 'marker': {
      const marker = typed['marker'];
      wire['marker'] = {
        marker_id: marker.markerId,
        ...('sideEffectValue' in marker ? { side_effect_value: wireValue(undefined, marker.sideEffectValue) } : {}),
      };
      break;
    }
    case 'set_state':
      wire['set_state'] = wireValue(undefined, typed['setState']);
      break;
    case 'update_decision': {
      const decision = typed['updateDecision'];
      wire['update_decision'] = {
        admitted_seq: Number(decision.admittedSeq),
        accepted: decision.accepted === true,
        rejection_reason: decision.rejectionReason ?? '',
      };
      break;
    }
    case 'update_result': {
      const result = typed['updateResult'];
      wire['update_result'] = {
        admitted_seq: Number(result.admittedSeq),
        ...('result' in result ? { result: wireValue(undefined, result.result) } : {}),
        failure_code: result.failureCode ?? '',
      };
      break;
    }
    case 'consume_signals':
      wire['consume_signals'] = {
        signals: (typed['consumeSignals'].signals as Array<{ signalName: string; seqs: string[] }>).map((s) => ({
          signal_name: s.signalName,
          seqs: s.seqs.map(Number),
        })),
      };
      break;
    case 'complete':
      wire['complete'] = wireValue(command.valueBlob, typed['complete']);
      break;
    case 'fail':
      wire['fail'] = { code: typed['fail'].code, safe_message: typed['fail'].safeMessage };
      break;
    case 'continue_as_new': {
      const next = typed['continueAsNew'];
      wire['continue_as_new'] = {
        input: wireValue(undefined, next.input),
        ...('state' in next ? { state: wireValue(undefined, next.state) } : {}),
        active_version: next.activeVersion === true,
      };
      break;
    }
    case 'acknowledge_cancellation':
      wire['acknowledge_cancellation'] = {};
      break;
    default:
      break;
  }
  return wire;
}

/**
 * The fields of the CompleteWorkflowTaskRequest a correct SDK sends for a golden activation, under the conventions of
 * this file's header (a first deferral last in `commands`, a re-emission in `deferred_command`).
 */
export function goldenCompletion(activation: GoldenActivation): Record<string, unknown> {
  const { commands, deferredCommand } = expectedWire(activation);
  const out: Record<string, unknown> = {
    applied_through_seq: activation.appliedThroughSeq ?? 0,
    replayed_chain: Buffer.from(activation.replayedChainHex ?? ZERO_CHAIN, 'hex'),
    commands: commands.map(({ golden, number }) => goldenCommandToWire(golden, number)),
  };
  if (deferredCommand) out['deferred_command'] = goldenCommandToWire(deferredCommand, 0);
  return out;
}

/** The golden commands of an activation in the order and with the numbers they take on the wire. */
function expectedWire(activation: GoldenActivation): {
  commands: Array<{ golden: GoldenCommand; number: number }>;
  deferredCommand: GoldenCommand | undefined;
} {
  const golden = activation.commands ?? [];
  const reEmitted = golden.find((c) => c.reEmitted);
  const commands = golden.filter((c) => !c.reEmitted).map((c) => ({ golden: c, number: c.commandNumber ?? 0 }));
  let deferredCommand = reEmitted;
  const deferral = activation.deferred;
  if (deferral) {
    if (deferral.reEmitted) deferredCommand = deferral;
    else commands.push({ golden: deferral, number: (activation.commandCountAfter ?? 0) + 1 });
  }
  return { commands, deferredCommand };
}

// ---------------------------------------------------------------------------------------------------------------
// The server

export interface FakeWorkflowServerOptions {
  /** The scenario directory under `scenarios/`, e.g. `code-run-lifecycle`. */
  scenario: string;
  /** A further run of the scenario (its subdirectory, e.g. `successor`); `main` (the default) is the scenario's own. */
  run?: string;
  /** At most this many events in a lease's first page (the 256 KiB bound applies too); 0 sends an empty first page. */
  firstPageMaxEvents?: number;
  /** At most this many events per ReadHistory page; 1,024 by default, the protocol's bound. */
  pageSize?: number;
  /** The corpus directory; found from this file by default. */
  corpusDir?: string;
}

export interface ReceivedCall { method: string; request: Record<string, any>; at: number }
export interface Mismatch { method: string; k: number | null; attempt: number | null; message: string }
export interface HistoryPage { k: number; attempt: number; fromSeq: number; throughSeq: number; events: number; lastPage: boolean }
export interface FailureRecord {
  k: number;
  attempt: number;
  cause: string;
  commandNumber: number;
  actualKind: string;
  actualTarget: string;
  actualSagaScope: string;
  exceptionType: string;
  codeDigest: string;
}

interface LeasePlan {
  k: number;
  attempt: number;
  seq: number;
  activation: GoldenActivation;
  failure: GoldenFailedAttempt | undefined;
}
interface IssuedLease {
  plan: LeasePlan;
  taskId: string;
  generation: number;
  leaseToken: string;
  claimedAt: number;
  settled: boolean;
  lease: Record<string, any>;
}
interface Session { workerId: string; sessionToken: string; accepted: string; offers: Array<{ workflow_type: string; code_digest: string }> }

export interface FakeWorkflowServer {
  /** `127.0.0.1:<port>`. */
  readonly address: string;
  /** The scenario run being served (commands.json, chain.json and the decoded history). */
  readonly golden: GoldenRun;
  /** Every call, in the order it arrived, with its decoded request (a client-streaming call has an empty request). */
  readonly received: ReceivedCall[];
  /** Every difference from the goldens, with a precise message; empty when the worker did exactly what they say. */
  readonly mismatches: Mismatch[];
  /** The leases issued, in order (TaskLease objects as sent). */
  readonly leases: Array<Record<string, any>>;
  /** The ReadHistory pages sent, in the form of commands.json's `historyPages`. */
  readonly historyPages: HistoryPage[];
  /** The completions answered: activation, attempt, outcome and cause. */
  readonly completions: Array<{ k: number; attempt: number; outcome: string; cause: string }>;
  /** The FailWorkflowTask reports received, with their cause and fields. */
  readonly failures: FailureRecord[];
  /** True once a completion differed (answered REVIEW): no further lease is served. */
  readonly underReview: boolean;
  /** True once every lease of the golden is issued and settled. */
  readonly finished: boolean;
  close(): Promise<void>;
}

class ServerState implements FakeWorkflowServer {
  address = '';
  readonly received: ReceivedCall[] = [];
  readonly mismatches: Mismatch[] = [];
  readonly leases: Array<Record<string, any>> = [];
  readonly historyPages: HistoryPage[] = [];
  readonly completions: Array<{ k: number; attempt: number; outcome: string; cause: string }> = [];
  readonly failures: FailureRecord[] = [];
  underReview = false;
  private readonly plans: LeasePlan[] = [];
  private next = 0;
  private current: IssuedLease | undefined;
  private generation = 0;
  private readonly sessions: Session[] = [];
  private readonly answered = new Map<string, { k: number; attempt: number; response: Record<string, unknown> }>();
  private readonly eventBytes: number[];
  private readonly server = new grpc.Server({ 'grpc.max_receive_message_length': RECEIVE_LIMIT });

  constructor(readonly golden: GoldenRun, private readonly options: FakeWorkflowServerOptions) {
    for (const activation of golden.commands.activations) {
      (activation.failedAttempts ?? []).forEach((failure, index) => {
        this.plans.push({ k: activation.k, attempt: index + 1, seq: failure.startedSeq, activation, failure });
      });
      if (activation.window.throughSeq !== null) {
        this.plans.push({
          k: activation.k,
          attempt: (activation.failedAttempts?.length ?? 0) + 1,
          seq: activation.window.throughSeq,
          activation,
          failure: undefined,
        });
      }
    }
    const codec = historyCodec();
    this.eventBytes = golden.history.map((event) => (codec.responseSerialize({ events: [event] }) as Buffer).length);
  }

  get finished(): boolean {
    return this.next >= this.plans.length && (!this.current || this.current.settled);
  }

  async start(): Promise<void> {
    const { service } = loadProto();
    const impl: grpc.UntypedServiceImplementation = {};
    const served: Record<string, grpc.UntypedHandleCall> = {
      RegisterWorker: this.unary('RegisterWorker', (request) => this.register(request)),
      PollTasks: this.unary('PollTasks', (request) => this.poll(request)),
      StreamTasks: (call: grpc.ServerWritableStream<any, any>) => {
        this.record('StreamTasks', call.request);
        try {
          call.write(this.poll(call.request));
          call.end();
        } catch (error) {
          call.emit('error', error);
        }
      },
      CompleteWorkflowTask: this.unary('CompleteWorkflowTask', (request) => this.complete(request)),
      FailWorkflowTask: this.unary('FailWorkflowTask', (request) => this.fail(request)),
      ReadHistory: (call: grpc.ServerWritableStream<any, any>) => {
        this.record('ReadHistory', call.request);
        try {
          for (const page of this.readHistory(call.request)) call.write(page);
          call.end();
        } catch (error) {
          call.emit('error', error);
        }
      },
    };
    for (const [name, method] of Object.entries(service)) {
      const handler = served[name];
      if (handler) {
        impl[name] = handler;
        continue;
      }
      const unimplemented = (): grpc.ServiceError => statusError(grpc.status.UNIMPLEMENTED, `the fake workflow server does not serve ${name}`);
      if (method.requestStream) {
        impl[name] = method.responseStream
          ? (call: grpc.ServerDuplexStream<any, any>) => {
              this.record(name, {});
              call.emit('error', unimplemented());
            }
          : (_call: grpc.ServerReadableStream<any, any>, callback: grpc.sendUnaryData<any>) => {
              this.record(name, {});
              callback(unimplemented());
            };
      } else {
        impl[name] = method.responseStream
          ? (call: grpc.ServerWritableStream<any, any>) => {
              this.record(name, call.request);
              call.emit('error', unimplemented());
            }
          : (call: grpc.ServerUnaryCall<any, any>, callback: grpc.sendUnaryData<any>) => {
              this.record(name, call.request);
              callback(unimplemented());
            };
      }
    }
    this.server.addService(service, impl);
    const port = await new Promise<number>((resolve, reject) =>
      this.server.bindAsync('127.0.0.1:0', grpc.ServerCredentials.createInsecure(), (error, bound) =>
        error ? reject(error) : resolve(bound),
      ),
    );
    this.address = `127.0.0.1:${port}`;
  }

  close(): Promise<void> {
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.server.forceShutdown();
        resolve();
      }, 1000);
      this.server.tryShutdown(() => {
        clearTimeout(timer);
        resolve();
      });
    });
  }

  private record(method: string, request: Record<string, any>): void {
    this.received.push({ method, request, at: performance.now() });
  }

  private unary(method: string, answer: (request: any) => unknown): grpc.handleUnaryCall<any, any> {
    return (call, callback) => {
      this.record(method, call.request);
      try {
        callback(null, answer(call.request));
      } catch (error) {
        callback(error as grpc.ServiceError);
      }
    };
  }

  private mismatch(method: string, plan: LeasePlan | undefined, message: string): void {
    this.mismatches.push({ method, k: plan?.k ?? null, attempt: plan?.attempt ?? null, message });
  }

  // RegisterWorker -------------------------------------------------------------------------------------------

  private register(request: Record<string, any>): Record<string, unknown> {
    const capabilities = request['capabilities'] ?? {};
    const offered = String(capabilities.protocol_version ?? '');
    const match = /^1\.(\d+)$/.exec(offered);
    if (!match) throw statusError(grpc.status.INVALID_ARGUMENT, `unsupported protocol version ${offered}`, 'PROTOCOL_UNSUPPORTED');
    const minor = Math.min(Number(match[1]), 10);
    const accepted = `1.${minor}`;
    const offers = (capabilities.workflow_types ?? []) as Session['offers'];
    if (minor < 10) {
      this.mismatch('RegisterWorker', undefined, `the session offers protocol ${offered}: workflow leases need 1.10`);
    } else if (!capabilities.offers_workflow_as_code) {
      this.mismatch('RegisterWorker', undefined, 'the session does not set offers_workflow_as_code');
    }
    const n = this.sessions.length + 1;
    const session: Session = { workerId: `fake-worker-${n}`, sessionToken: `fake-session-${n}`, accepted, offers };
    this.sessions.push(session);
    return {
      worker_id: session.workerId,
      session_token: session.sessionToken,
      accepted_protocol_version: accepted,
      ...(minor >= 7 ? { step_logs: { enabled: false } } : {}),
    };
  }

  // PollTasks and StreamTasks -----------------------------------------------------------------------------------

  private poll(request: Record<string, any>): Record<string, unknown> {
    const session = this.sessions.find((s) => s.workerId === request['worker_id'] && s.sessionToken === request['session_token']);
    if (!session) throw statusError(grpc.status.UNAUTHENTICATED, 'unknown session');
    const empty = { tasks: [], retry_after_ms: RETRY_AFTER_MS };
    if (session.accepted !== '1.10' || this.underReview || !(request['max_tasks'] > 0)) return empty;
    if (this.current && !this.current.settled) return empty;
    const plan = this.plans[this.next];
    if (!plan) return empty;
    this.next += 1;
    const lease = this.issue(plan);
    const digest = lease.lease['workflow_task'].code_digest as string;
    if (!session.offers.some((o) => o.workflow_type === this.golden.commands.workflowType && o.code_digest === digest)) {
      this.mismatch('PollTasks', plan, `the polling session did not offer (${this.golden.commands.workflowType}, ${digest})`);
    }
    return { tasks: [lease.lease], retry_after_ms: 0 };
  }

  private issue(plan: LeasePlan): IssuedLease {
    const { commands } = this.golden;
    this.generation += 1;
    const generation = this.generation;
    const taskId = `wt-${plan.k}-${plan.attempt}`;
    const leaseToken = `fake-lease-${generation}`;
    const claimedAt = Date.now();
    const expires = claimedAt + LEASE_MS;
    const startedEvent = this.golden.history.find((e) => e.seq === plan.seq);
    const started = (startedEvent?.['workflow_task_started'] ?? {}) as Record<string, boolean>;
    const visible = this.golden.history.filter((e) => e.seq <= plan.seq);
    const firstPage: HistoryEvent[] = [];
    let bytes = 0;
    const maxEvents = this.options.firstPageMaxEvents ?? Number.POSITIVE_INFINITY;
    for (let i = 0; i < visible.length && firstPage.length < maxEvents; i += 1) {
      const size = this.eventBytes[i] ?? 0;
      if (bytes + size > FIRST_PAGE_BYTES) break;
      bytes += size;
      firstPage.push(visible[i] as HistoryEvent);
    }
    const workflowTask = {
      run_id: commands.runId,
      attempt: plan.attempt,
      generation,
      lease_token: leaseToken,
      expires_at_epoch_ms: expires,
      seed: Buffer.from(commands.seedHex, 'hex'),
      started_at_us: startedEvent?.['at_us'] ?? 0,
      window_start_seq: plan.activation.window.afterSeq,
      gated_entries: commands.lease?.gatedEntries ?? [],
      state_max_bytes: commands.lease?.stateMaxBytes ?? 0,
      first_page: firstPage,
      visible_count: visible.length,
      continue_as_new_suggested: started['continue_as_new_suggested'] === true,
      history_limit_reached: started['history_limit_reached'] === true,
      traceparent: '',
      workflow_type: commands.workflowType,
      code_digest: commands.codeVersions[`v${plan.activation.codeVersion}`] ?? '',
    };
    const lease = {
      task_id: taskId,
      run_id: commands.runId,
      step_id: '~workflow',
      attempt: plan.attempt,
      generation,
      lease_token: leaseToken,
      expires_at_epoch_ms: expires,
      traceparent: '',
      task_kind: 'WORKFLOW',
      workflow_task: workflowTask,
    };
    this.current = { plan, taskId, generation, leaseToken, claimedAt, settled: false, lease };
    this.leases.push(lease);
    return this.current;
  }

  /** The current lease when the request's fence names it, else undefined. */
  private fenced(request: Record<string, any>): IssuedLease | undefined {
    const lease = this.current;
    if (!lease || lease.settled) return undefined;
    const same =
      request['task_id'] === lease.taskId &&
      request['run_id'] === this.golden.commands.runId &&
      request['attempt'] === lease.plan.attempt &&
      request['generation'] === lease.generation &&
      request['lease_token'] === lease.leaseToken;
    return same ? lease : undefined;
  }

  // ReadHistory ----------------------------------------------------------------------------------------------------

  private readHistory(request: Record<string, any>): Array<Record<string, unknown>> {
    if (request['read_token']) throw statusError(grpc.status.NOT_FOUND, 'no such run, lease or token');
    const fromSeq = Number(request['from_seq']);
    if (!(fromSeq >= 1)) throw statusError(grpc.status.INVALID_ARGUMENT, 'from_seq is below 1');
    const lease = this.fenced(request);
    if (!lease) {
      this.mismatch('ReadHistory', this.current?.plan, 'ReadHistory names a lease that is not current');
      throw statusError(grpc.status.NOT_FOUND, 'no such run, lease or token');
    }
    const pageSize = this.options.pageSize ?? PAGE_EVENTS;
    const expires = Math.min(Date.now() + LEASE_MS, lease.claimedAt + LEASE_CAP_MS);
    lease.lease['expires_at_epoch_ms'] = expires;
    const indices = this.golden.history.flatMap((e, i) => (e.seq >= fromSeq && e.seq <= lease.plan.seq ? [i] : []));
    const pages: Array<Record<string, unknown>> = [];
    let page: number[] = [];
    let bytes = 0;
    const flush = (lastPage: boolean): void => {
      const events = page.map((i) => this.golden.history[i] as HistoryEvent);
      pages.push({ events, last_page: lastPage, lease_expires_at_epoch_ms: expires });
      this.historyPages.push({
        k: lease.plan.k,
        attempt: lease.plan.attempt,
        fromSeq: events[0]?.seq ?? fromSeq,
        throughSeq: events[events.length - 1]?.seq ?? fromSeq - 1,
        events: events.length,
        lastPage,
      });
      page = [];
      bytes = 0;
    };
    for (const i of indices) {
      const size = this.eventBytes[i] ?? 0;
      if (page.length > 0 && (page.length >= pageSize || bytes + size > PAGE_BYTES)) flush(false);
      page.push(i);
      bytes += size;
    }
    flush(true);
    return pages;
  }

  // CompleteWorkflowTask -------------------------------------------------------------------------------------------

  private replay(request: Record<string, any>, method: string): Record<string, unknown> | undefined {
    const key = String(request['idempotency_key'] ?? '');
    const recorded = key ? this.answered.get(`${method}:${key}`) : undefined;
    if (!recorded) return undefined;
    if (request['task_id'] !== `wt-${recorded.k}-${recorded.attempt}`) {
      throw statusError(grpc.status.ABORTED, 'the idempotency key was used for another report', 'IDEMPOTENCY_KEY_REUSED');
    }
    return { ...recorded.response, already_applied: true };
  }

  private complete(request: Record<string, any>): Record<string, unknown> {
    const replayed = this.replay(request, 'CompleteWorkflowTask');
    if (replayed) return replayed;
    const lease = this.fenced(request);
    if (!lease) {
      this.mismatch('CompleteWorkflowTask', this.current?.plan, `CompleteWorkflowTask for a lease that is not current (task ${String(request['task_id'])})`);
      throw statusError(grpc.status.ABORTED, 'the lease is not current', 'STALE_ATTEMPT');
    }
    const { plan } = lease;
    lease.settled = true;
    const activation = plan.activation;
    const at = `activation ${plan.k} (attempt ${plan.attempt})`;
    const before = this.mismatches.length;
    let cause = 'CAUSE_COMMAND_INVALID';
    const differ = (message: string, reviewCause?: string): void => {
      if (this.mismatches.length === before && reviewCause) cause = reviewCause;
      this.mismatch('CompleteWorkflowTask', plan, `${at}: ${message}`);
    };
    if (plan.failure) {
      differ(`the golden fails this attempt with ${plan.failure.cause}; the worker completed it`, 'CAUSE_NONDETERMINISM');
    } else {
      const applied = Number(request['applied_through_seq']);
      if (applied !== activation.appliedThroughSeq) {
        differ(
          `applied_through_seq: expected ${activation.appliedThroughSeq}, received ${applied}`,
          applied < (activation.appliedThroughSeq ?? 0) ? 'CAUSE_NONDETERMINISM_RISK' : undefined,
        );
      }
      const chain = Buffer.from(request['replayed_chain'] ?? []).toString('hex');
      if (chain !== activation.replayedChainHex) {
        differ(`replayed_chain: expected ${activation.replayedChainHex} (chain.json), received ${chain || '(empty)'}`, 'CAUSE_CHAIN_MISMATCH');
      }
      const flags = (request['behaviour_flags'] ?? []) as string[];
      if (flags.length > 0) differ(`behaviour_flags must be empty in this minor, received ${JSON.stringify(flags)}`);
      this.compareCommands(request, activation, differ);
    }
    let response: Record<string, unknown>;
    if (this.mismatches.length > before) {
      this.underReview = true;
      response = { outcome: 'WORKFLOW_TASK_REVIEW', cause, already_applied: false };
    } else {
      response = {
        outcome: activation.outcome ?? 'WORKFLOW_TASK_ACCEPTED',
        cause: activation.cause ?? 'WORKFLOW_TASK_FAILURE_CAUSE_UNSPECIFIED',
        already_applied: false,
      };
    }
    this.completions.push({ k: plan.k, attempt: plan.attempt, outcome: String(response['outcome']), cause: String(response['cause']) });
    const key = String(request['idempotency_key'] ?? '');
    if (key) this.answered.set(`CompleteWorkflowTask:${key}`, { k: plan.k, attempt: plan.attempt, response });
    return response;
  }

  private compareCommands(request: Record<string, any>, activation: GoldenActivation, differ: (m: string) => void): void {
    const { commands: expected, deferredCommand } = expectedWire(activation);
    const received = (request['commands'] ?? []) as Array<Record<string, any>>;
    const describe = (list: Array<Record<string, any>>): string => list.map((c) => `${c['command_number']}:${c['kind']}`).join(', ') || 'none';
    if (received.length !== expected.length) {
      differ(
        `${received.length} commands received, the golden has ${expected.length} ` +
          `(expected ${expected.map((e) => `${e.number}:${wireKind(e.golden.kind)}`).join(', ') || 'none'}; received ${describe(received)})`,
      );
    }
    for (let i = 0; i < Math.min(received.length, expected.length); i += 1) {
      const want = expected[i] as { golden: GoldenCommand; number: number };
      compareCommand(`command ${want.number} (${want.golden.kind} ${want.golden.target})`, received[i] ?? {}, want.golden, want.number, differ);
    }
    const deferred = request['deferred_command'] as Record<string, any> | null | undefined;
    if (deferredCommand && !deferred) {
      differ(
        `deferred_command: expected the re-emitted ${deferredCommand.kind} without a number ` +
          '(worker.proto CompleteWorkflowTaskRequest.deferred_command), received none',
      );
    } else if (!deferredCommand && deferred) {
      differ(`deferred_command: the golden re-emits nothing in this activation, received ${String(deferred['kind'])}`);
    } else if (deferredCommand && deferred) {
      compareCommand(`deferred_command (${deferredCommand.kind})`, deferred, deferredCommand, 0, differ);
    }
  }

  // FailWorkflowTask -----------------------------------------------------------------------------------------------

  private fail(request: Record<string, any>): Record<string, unknown> {
    const replayed = this.replay(request, 'FailWorkflowTask');
    if (replayed) return replayed;
    const lease = this.fenced(request);
    if (!lease) {
      this.mismatch('FailWorkflowTask', this.current?.plan, `FailWorkflowTask for a lease that is not current (task ${String(request['task_id'])})`);
      throw statusError(grpc.status.ABORTED, 'the lease is not current', 'STALE_ATTEMPT');
    }
    const { plan } = lease;
    lease.settled = true;
    const record: FailureRecord = {
      k: plan.k,
      attempt: plan.attempt,
      cause: String(request['cause']),
      commandNumber: Number(request['command_number'] ?? 0),
      actualKind: String(request['actual_kind'] ?? ''),
      actualTarget: String(request['actual_target'] ?? ''),
      actualSagaScope: String(request['actual_saga_scope'] ?? ''),
      exceptionType: String(request['exception_type'] ?? ''),
      codeDigest: String(request['code_digest'] ?? ''),
    };
    this.failures.push(record);
    const at = `activation ${plan.k} (attempt ${plan.attempt})`;
    const differ = (message: string): void => this.mismatch('FailWorkflowTask', plan, `${at}: ${message}`);
    const golden = plan.failure;
    if (!golden) {
      differ(`the golden completes this activation; the worker failed it with ${record.cause}`);
      this.underReview = true;
    } else {
      const check = (field: string, expected: unknown, actual: unknown): void => {
        if (expected !== undefined && expected !== actual) differ(`${field}: expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`);
      };
      check('cause', golden.cause, record.cause);
      check('exception_type', golden.exceptionType, record.exceptionType);
      check('command_number', golden.report?.commandNumber, record.commandNumber);
      check('actual_kind', golden.report?.actualKind, record.actualKind);
      check('actual_target', golden.report?.actualTarget, record.actualTarget);
      // The scope the function used (Decision 044-244): compared whenever the golden reports a command, empty when
      // the golden names none; the server drops a scope outside its pattern (Decision 044-250), so a conforming SDK
      // never sends one, and only an activity or a script has a scope.
      if (golden.report) check('actual_saga_scope', golden.report.actualSagaScope ?? '', record.actualSagaScope);
      check('code_digest', golden.report?.codeDigest, record.codeDigest);
    }
    if (record.actualSagaScope !== '') {
      if (!SAGA_SCOPE.test(record.actualSagaScope)) {
        differ(`actual_saga_scope ${JSON.stringify(record.actualSagaScope)} is outside ${SAGA_SCOPE.source}; the server drops it`);
      }
      if (record.actualKind !== 'COMMAND_ACTIVITY' && record.actualKind !== 'COMMAND_SCRIPT') {
        differ(`actual_saga_scope is set for ${record.actualKind}; only an activity or a script has a saga scope`);
      }
    }
    const response = { already_applied: false };
    const key = String(request['idempotency_key'] ?? '');
    if (key) this.answered.set(`FailWorkflowTask:${key}`, { k: plan.k, attempt: plan.attempt, response });
    return response;
  }
}

// ---------------------------------------------------------------------------------------------------------------
// Comparing one command

function render(bytes: Uint8Array | null | undefined): string {
  if (!bytes || bytes.length === 0) return '(empty)';
  const text = Buffer.from(bytes).toString('utf8');
  return /^[\x20-\x7e -￿]*$/.test(text) ? text : `base64:${Buffer.from(bytes).toString('base64')}`;
}

type ExpectedValue = { json: unknown } | { blob: GoldenBlob } | undefined;

function compareValue(
  where: string,
  received: Record<string, any> | null | undefined,
  expected: ExpectedValue,
  differ: (m: string) => void,
  reEmitted = false,
): void {
  if (!expected) {
    if (received) differ(`${where}: the golden has no value, received one (${String(received['carrier'])})`);
    return;
  }
  if (!received) {
    differ(`${where}: the golden has a value, received none`);
    return;
  }
  const sha = String(received['descriptor_sha256'] ?? '');
  const size = Number(received['descriptor_size'] ?? 0);
  // A re-emitted deferral may name any value its deferral holds by its descriptor alone, whatever the deferral's
  // carrier (Decision 044-259): the server resolves the descriptor to the value it holds.
  if (reEmitted && !received['carrier']) {
    const d = 'json' in expected ? descriptorOf(expected.json) : expected.blob.descriptor;
    if (sha !== d.sha256) differ(`${where}.descriptor_sha256: expected ${d.sha256}, received ${sha || '(empty)'}`);
    if (size !== d.size) differ(`${where}.descriptor_size: expected ${d.size}, received ${size}`);
    return;
  }
  if ('json' in expected) {
    const want = canonicalJson(expected.json);
    if (received['carrier'] !== 'json') {
      differ(`${where}: expected the inline value ${want.toString('utf8')}, received the carrier ${String(received['carrier'] ?? 'none')}`);
      return;
    }
    const got = Buffer.from(received['json'] ?? []);
    if (!got.equals(want)) differ(`${where}.json: expected ${want.toString('utf8')}, received ${render(got)}`);
    const d = descriptorOf(expected.json);
    if (sha && sha !== d.sha256) differ(`${where}.descriptor_sha256: expected ${d.sha256} or empty, received ${sha}`);
    if (size && size !== d.size) differ(`${where}.descriptor_size: expected ${d.size} or 0, received ${size}`);
    return;
  }
  const { blob } = expected;
  const descriptorOnly = /^descriptor only/.test(blob.carrier);
  if (descriptorOnly) {
    if (received['carrier']) differ(`${where}: the golden names its blob by descriptor only, received the carrier ${String(received['carrier'])}`);
    if (sha !== blob.descriptor.sha256) differ(`${where}.descriptor_sha256: expected ${blob.descriptor.sha256}, received ${sha || '(empty)'}`);
    if (size !== blob.descriptor.size) differ(`${where}.descriptor_size: expected ${blob.descriptor.size}, received ${size}`);
    return;
  }
  if (received['carrier'] !== 'blob_id' || !received['blob_id']) {
    differ(`${where}: the golden carries a blob (${blob.carrier}), received the carrier ${String(received['carrier'] ?? 'none')}`);
  }
  if (sha && sha !== blob.descriptor.sha256) differ(`${where}.descriptor_sha256: expected ${blob.descriptor.sha256} or empty, received ${sha}`);
  if (size && size !== blob.descriptor.size) differ(`${where}.descriptor_size: expected ${blob.descriptor.size} or 0, received ${size}`);
}

function compareCommand(where: string, received: Record<string, any>, golden: GoldenCommand, number: number, differ: (m: string) => void): void {
  const same = (field: string, expected: unknown, actual: unknown): void => {
    if (expected !== actual) differ(`${where}: ${field}: expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`);
  };
  const reEmitted = golden.reEmitted === true;
  same('command_number', number, received['command_number']);
  same('kind', wireKind(golden.kind), received['kind']);
  const fragment = golden.fragment ? canonicalJson(golden.fragment) : Buffer.alloc(0);
  const gotFragment = Buffer.from(received['fragment_json'] ?? []);
  if (!gotFragment.equals(fragment)) differ(`${where}: fragment_json: expected ${render(fragment)}, received ${render(gotFragment)}`);
  const work = WORK_KINDS.has(golden.kind);
  compareValue(
    `${where}: value`,
    received['value'],
    work ? (golden.valueBlob ? { blob: golden.valueBlob } : { json: golden.value }) : undefined,
    differ,
    reEmitted,
  );
  same('saga_scope', golden.sagaScope ?? '', received['saga_scope'] ?? '');
  const member = typedMember(golden);
  same('typed', member ?? null, received['typed'] ?? null);
  if (!member || received['typed'] !== member) return;
  const typed = golden.typed ?? {};
  const got = (received[member] ?? {}) as Record<string, any>;
  const field = (name: string): string => `${member}.${name}`;
  const json = (value: unknown): ExpectedValue => (value === undefined ? undefined : { json: value });
  switch (member) {
    case 'target':
      same(field('target'), typed['target'].target, got['target']);
      break;
    case 'saga_close':
      same(field('target'), typed['sagaClose'].target, got['target']);
      break;
    case 'saga_abort':
      same(field('target'), typed['sagaAbort'].target, got['target']);
      break;
    case 'saga_open': {
      const open = typed['sagaOpen'];
      same(field('scope'), open.scope, got['scope']);
      same(field('parent_scope'), open.parentScope ?? '', got['parent_scope']);
      same(field('on_compensation_failure'), open.onCompensationFailure, got['on_compensation_failure']);
      same(field('cancel_policy'), open.cancelPolicy, got['cancel_policy']);
      break;
    }
    case 'marker':
      same(field('marker_id'), typed['marker'].markerId, got['marker_id']);
      compareValue(`${where}: ${field('side_effect_value')}`, got['side_effect_value'], json(typed['marker'].sideEffectValue), differ, reEmitted);
      break;
    case 'set_state':
      compareValue(`${where}: set_state`, got, json(typed['setState']), differ, reEmitted);
      break;
    case 'update_decision': {
      const decision = typed['updateDecision'];
      same(field('admitted_seq'), Number(decision.admittedSeq), got['admitted_seq']);
      same(field('accepted'), decision.accepted === true, got['accepted']);
      same(field('rejection_reason'), decision.rejectionReason ?? '', got['rejection_reason']);
      break;
    }
    case 'update_result': {
      const result = typed['updateResult'];
      same(field('admitted_seq'), Number(result.admittedSeq), got['admitted_seq']);
      compareValue(`${where}: ${field('result')}`, got['result'], 'result' in result ? { json: result.result } : undefined, differ, reEmitted);
      same(field('failure_code'), result.failureCode ?? '', got['failure_code']);
      break;
    }
    case 'consume_signals': {
      const want = (typed['consumeSignals'].signals as Array<{ signalName: string; seqs: string[] }>).map((s) => ({
        signal_name: s.signalName,
        seqs: s.seqs.map(Number),
      }));
      const have = ((got['signals'] ?? []) as Array<Record<string, any>>).map((s) => ({
        signal_name: s['signal_name'],
        seqs: (s['seqs'] ?? []).map(Number),
      }));
      if (JSON.stringify(want) !== JSON.stringify(have)) {
        differ(`${where}: consume_signals.signals: expected ${JSON.stringify(want)}, received ${JSON.stringify(have)}`);
      }
      break;
    }
    case 'complete':
      compareValue(`${where}: complete`, got, golden.valueBlob ? { blob: golden.valueBlob } : json(typed['complete']), differ, reEmitted);
      break;
    case 'fail':
      same(field('code'), typed['fail'].code, got['code']);
      same(field('safe_message'), typed['fail'].safeMessage, got['safe_message']);
      break;
    case 'continue_as_new': {
      const next = typed['continueAsNew'];
      compareValue(`${where}: ${field('input')}`, got['input'], json(next.input), differ, reEmitted);
      compareValue(`${where}: ${field('state')}`, got['state'], json(next.state), differ, reEmitted);
      same(field('active_version'), next.activeVersion === true, got['active_version']);
      break;
    }
    default:
      break;
  }
}

function statusError(code: grpc.status, details: string, reason?: string): grpc.ServiceError {
  const metadata = new grpc.Metadata();
  if (reason) metadata.set('x-sagat-error', reason);
  return Object.assign(new Error(details), { code, details, metadata }) as grpc.ServiceError;
}

/**
 * Starts a fake WorkerService on 127.0.0.1 (an ephemeral port) that serves one scenario run of the golden corpus.
 * Close it with `close()`.
 */
export async function startFakeWorkflowServer(options: FakeWorkflowServerOptions): Promise<FakeWorkflowServer> {
  const golden = loadGoldenRun(options.scenario, options.run ?? 'main', options.corpusDir ?? defaultCorpusDir());
  const state = new ServerState(golden, options);
  await state.start();
  return state;
}
