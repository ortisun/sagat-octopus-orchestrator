/*
 * The one adapter between the worker protocol of design 044 (`worker.proto`, minor 1.10) and the runtime's own model
 * (`context.ts`): history events from the wire (as the gRPC loader decodes them) and from export lines (the proto3 JSON
 * mapping, as the corpus's `history.jsonl` holds them), and the commands and failure reports a workflow task answers.
 * Decisions 044-319 and 044-320 of `specs/044-workflow-as-code/research.md`.
 */
import { rfc8785, type CommandKind, type EventOrigin, type HistoryEvent, type WorkflowTaskFailure } from './context.js';
import { descriptorOf } from './chain.js';
import type { PreparedCommand } from './run.js';

export class MalformedHistory extends Error {
  constructor(message: string) { super(message); this.name = 'MalformedHistory'; }
}

/** What a history payload holds once resolved: a value, a withheld value, or nothing. */
export type PayloadValue = { value: unknown } | { redacted: true } | undefined;

/** A blob a history payload references, for the worker to read under its lease before the events are mapped. */
export interface HistoryBlobRef { blobId: string; size: number; sha256: string }

const ORIGINS: Record<string, EventOrigin> = {
  HISTORY_ORIGIN_COMPLETION: 'completion', HISTORY_ORIGIN_EFFECT: 'effect', HISTORY_ORIGIN_ENGINE: 'engine', HISTORY_ORIGIN_EXTERNAL: 'external',
};
const ORIGIN_NAMES: Record<EventOrigin, string> = Object.fromEntries(Object.entries(ORIGINS).map(([name, origin]) => [origin, name])) as Record<EventOrigin, string>;

const COMMAND_KINDS: readonly CommandKind[] = [
  'activity', 'script', 'timer_start', 'timer_cancel', 'wait_event', 'child', 'approval', 'step_cancel', 'saga_open', 'saga_close',
  'saga_abort', 'marker_patch', 'marker_side_effect', 'set_state', 'update_accept', 'update_reject', 'update_complete',
  'consume_signals', 'complete', 'fail', 'continue_as_new', 'acknowledge_cancellation',
];

/** `COMMAND_TIMER_START` is `timer_start` (Technical Plan 6.2). */
export function commandKindOf(name: unknown): CommandKind | undefined {
  if (typeof name !== 'string' || !name.startsWith('COMMAND_')) return undefined;
  const kind = name.slice('COMMAND_'.length).toLowerCase() as CommandKind;
  return COMMAND_KINDS.includes(kind) ? kind : undefined;
}

export function commandKindName(kind: string): string {
  return kind === '' ? 'COMMAND_KIND_UNSPECIFIED' : `COMMAND_${kind.toUpperCase()}`;
}

const STEP_OUTCOMES: Record<string, 'completed' | 'failed' | 'timed_out' | 'cancelled'> = {
  STEP_OUTCOME_COMPLETED: 'completed', STEP_OUTCOME_FAILED: 'failed', STEP_OUTCOME_TIMED_OUT: 'timed_out', STEP_OUTCOME_CANCELLED: 'cancelled',
};
const CHILD_STATUSES: Record<string, 'completed' | 'failed' | 'cancelled' | 'timed_out' | 'partial'> = {
  COMPLETED: 'completed', FAILED: 'failed', CANCELLED: 'cancelled', TIMED_OUT: 'timed_out', PARTIAL: 'partial',
};
const DECISIONS = new Set(['approved', 'rejected', 'expired']);
const SAGA_OUTCOMES = new Set(['compensated', 'partial', 'compensation_failed']);
const NODE = /^c-([1-9][0-9]{0,15})$/;
const SCOPE = /^s-[1-9][0-9]{0,15}$/;

function int(value: unknown, what: string): number {
  const number = typeof value === 'string' && /^-?\d+$/.test(value) ? Number(value) : value;
  if (typeof number !== 'number' || !Number.isSafeInteger(number)) throw new MalformedHistory(`${what} is not an integer the SDK reads exactly`);
  return number;
}

function text(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

function record(value: unknown): Record<string, any> | undefined {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, any>) : undefined;
}

function nodeNumber(node: unknown, what: string): number {
  const match = NODE.exec(text(node));
  if (!match) throw new MalformedHistory(`${what} names no step c-<n>`);
  return Number(match[1]);
}

/** The blobs the payloads of these events reference (wire form), each once. */
export function blobRefsOf(events: Iterable<Record<string, any>>): HistoryBlobRef[] {
  const refs = new Map<string, HistoryBlobRef>();
  for (const event of events) {
    for (const payload of payloadsOf(event)) {
      const blob = record(payload?.['blob']);
      if (!blob || text(payload?.['placeholder']) !== '' || text(blob['blob_id']) === '') continue;
      refs.set(blob['blob_id'], { blobId: blob['blob_id'], size: int(blob['size'] ?? 0, 'a blob size'), sha256: text(blob['sha256']) });
    }
  }
  return [...refs.values()];
}

function payloadsOf(event: Record<string, any>): Array<Record<string, any> | undefined> {
  const body = record(event[text(event['body'])]);
  if (!body) return [];
  return [body['input'], body['carried_state'], body['output'], body['payload'], body['args'], body['value']].map(record);
}

/**
 * Maps one event of the wire (the loader's object form) to the runtime's model. `blobs` holds the values of the blobs
 * the worker read; a blob it did not read (an export, a read it was refused) is a withheld value.
 */
export function eventFromWire(event: Record<string, any>, blobs: ReadonlyMap<string, unknown> = new Map()): HistoryEvent {
  const seq = int(event['seq'], 'an event sequence');
  if (seq < 1) throw new MalformedHistory('an event sequence is at least 1');
  const kind = text(event['kind']);
  const member = text(event['body']);
  const body = record(event[member]) ?? {};
  let redacted = false;
  const payload = (value: unknown): PayloadValue => {
    const resolved = payloadOf(value, blobs);
    if (resolved && 'redacted' in resolved) redacted = true;
    return resolved;
  };
  const withValue = <T extends object>(target: T, key: string, value: PayloadValue): T => {
    if (value && 'value' in value) (target as Record<string, unknown>)[key] = value.value;
    return target;
  };
  let mapped: DistributiveOmit<HistoryEvent, 'seq' | 'origin' | 'atUs' | 'redacted'>;
  switch (kind) {
    case 'HISTORY_RUN_STARTED': {
      const started = { kind: 'run.started' as const };
      withValue(started, 'input', payload(body['input']));
      withValue(started, 'carriedState', payload(body['carried_state']));
      mapped = started;
      break;
    }
    case 'HISTORY_WORKFLOW_TASK_STARTED':
      mapped = { kind: 'workflow_task.started', continueAsNewSuggested: body['continue_as_new_suggested'] === true, historyLimitReached: body['history_limit_reached'] === true };
      break;
    case 'HISTORY_WORKFLOW_TASK_COMPLETED': {
      mapped = { kind: 'workflow_task.completed' };
      if (body['deferred'] === true) {
        const deferred = commandKindOf(body['deferred_kind']);
        if (!deferred) throw new MalformedHistory(`event ${seq} defers no command kind`);
        mapped = { kind: 'workflow_task.completed', deferral: { kind: deferred, target: text(body['deferred_target']) } };
      }
      break;
    }
    case 'HISTORY_COMMAND_RECORDED': {
      const commandKind = commandKindOf(body['kind']);
      if (!commandKind) throw new MalformedHistory(`event ${seq} records no command kind`);
      const recorded: Extract<HistoryEvent, { kind: 'command.record' }> = {
        seq, kind: 'command.record', commandNumber: int(event['command_number'], 'a command number'), commandKind, target: text(body['target']),
      };
      const scope = text(body['saga_scope']);
      if (scope !== '') {
        if (!SCOPE.test(scope)) throw new MalformedHistory(`event ${seq} names the saga scope ${JSON.stringify(scope)}`);
        recorded.sagaScope = scope;
      }
      const { seq: _seq, ...rest } = recorded;
      mapped = rest;
      break;
    }
    case 'HISTORY_MARKER_RECORDED': {
      const marker = { kind: 'marker.recorded' as const, commandNumber: int(event['command_number'], 'a command number'),
        markerKind: body['side_effect'] === true ? 'side_effect' as const : 'patch' as const, markerId: text(body['marker_id']) };
      mapped = marker.markerKind === 'side_effect' ? withValue(marker, 'value', payload(body['value'])) : marker;
      break;
    }
    case 'HISTORY_STEP_OUTCOME': {
      const status = STEP_OUTCOMES[text(body['outcome'])];
      if (!status) throw new MalformedHistory(`event ${seq} has no step outcome`);
      const outcome: Extract<HistoryEvent, { kind: 'step.outcome' }> = { seq, kind: 'step.outcome', commandNumber: nodeNumber(body['node'], `event ${seq}`), status };
      if (text(body['error_code']) !== '') outcome.errorCode = body['error_code'];
      if (status === 'completed') withValue(outcome, 'output', payload(body['output']) ?? { value: null });
      const { seq: _seq, ...rest } = outcome;
      mapped = rest;
      break;
    }
    case 'HISTORY_TIMER_FIRED':
      mapped = { kind: 'timer.fired', commandNumber: nodeNumber(body['node'], `event ${seq}`) };
      break;
    case 'HISTORY_WAIT_OUTCOME': {
      const timedOut = body['timed_out'] === true;
      const wait = { kind: 'wait.outcome' as const, commandNumber: nodeNumber(body['node'], `event ${seq}`), status: timedOut ? 'timed_out' as const : 'satisfied' as const };
      mapped = timedOut ? wait : withValue(wait, 'payload', payload(body['payload']) ?? { value: null });
      break;
    }
    case 'HISTORY_CHILD_FINISHED': {
      const status = CHILD_STATUSES[text(body['status'])];
      if (!status) throw new MalformedHistory(`event ${seq} has no child status`);
      const child: Extract<HistoryEvent, { kind: 'child.finished' }> = { seq, kind: 'child.finished', commandNumber: nodeNumber(body['node'], `event ${seq}`), status };
      if (text(body['error_code']) !== '') child.errorCode = body['error_code'];
      if (status === 'completed') withValue(child, 'output', payload(body['output']) ?? { value: null });
      const { seq: _seq, ...rest } = child;
      mapped = rest;
      break;
    }
    case 'HISTORY_APPROVAL_DECIDED': {
      const decision = text(body['decision']);
      if (!DECISIONS.has(decision)) throw new MalformedHistory(`event ${seq} has no approval decision`);
      mapped = { kind: 'approval.decided', commandNumber: nodeNumber(body['node'], `event ${seq}`), decision: decision as 'approved' | 'rejected' | 'expired' };
      break;
    }
    case 'HISTORY_SAGA_OUTCOME': {
      const outcome = text(body['outcome']);
      const scope = text(body['scope']);
      if (!SAGA_OUTCOMES.has(outcome) || !SCOPE.test(scope)) throw new MalformedHistory(`event ${seq} has no saga outcome`);
      mapped = { kind: 'saga.outcome', scope, outcome: outcome as 'compensated' | 'partial' | 'compensation_failed' };
      break;
    }
    case 'HISTORY_SIGNAL_BUFFERED':
      mapped = withValue({ kind: 'signal.buffered' as const, name: text(body['signal_name']) }, 'payload', payload(body['payload']) ?? { value: null });
      break;
    case 'HISTORY_UPDATE_ADMITTED':
      mapped = withValue({ kind: 'update.admitted' as const, name: text(body['update_name']) }, 'args', payload(body['args']) ?? { value: null });
      break;
    case 'HISTORY_CANCELLATION_REQUESTED':
      mapped = { kind: 'run.cancel_requested' };
      break;
    default:
      throw new MalformedHistory(`event ${seq} is of no kind this SDK reads (${JSON.stringify(kind)})`);
  }
  const out = { ...mapped, seq } as HistoryEvent;
  const origin = ORIGINS[text(event['origin'])];
  if (origin) out.origin = origin;
  if (event['at_us'] !== undefined && event['at_us'] !== null) out.atUs = int(event['at_us'], 'an event time');
  if (redacted) out.redacted = true;
  return out;
}

function payloadOf(value: unknown, blobs: ReadonlyMap<string, unknown>): PayloadValue {
  const payload = record(value);
  if (!payload) return undefined;
  if (text(payload['placeholder']) !== '') return { redacted: true };
  const json = payload['json'];
  if (json instanceof Uint8Array && (payload['value'] === 'json' || payload['value'] === undefined)) {
    if (json.length === 0 && payload['value'] !== 'json') return undefined;
    try {
      return { value: JSON.parse(Buffer.from(json).toString('utf8')) as unknown };
    } catch {
      throw new MalformedHistory('a history payload is not JSON');
    }
  }
  const blob = record(payload['blob']);
  if (blob) {
    const id = text(blob['blob_id']);
    return id !== '' && blobs.has(id) ? { value: blobs.get(id) } : { redacted: true };
  }
  return undefined;
}

// ---------------------------------------------------------------------------------------------------------------------
// The proto3 JSON mapping of an export line (and of the corpus), to the loader's object form.

const INT64_FIELDS = new Set(['seq', 'command_number', 'at_us', 'grace_ms', 'size']);
const EVENT_BODIES = new Set([
  'run_started', 'workflow_task_started', 'workflow_task_completed', 'command_record', 'step_outcome', 'timer_fired', 'wait_outcome',
  'child_finished', 'approval_decided', 'saga_outcome', 'signal_buffered', 'update_admitted', 'cancellation_requested', 'marker_recorded',
]);

const snake = (key: string): string => key.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`);

function fromJson(value: unknown, key: string): unknown {
  if (Array.isArray(value)) return value.map((item) => fromJson(item, key));
  if (value !== null && typeof value === 'object') {
    const out: Record<string, unknown> = {};
    for (const [name, item] of Object.entries(value as Record<string, unknown>)) out[snake(name)] = fromJson(item, snake(name));
    if (key === 'json' || key === '') return out;
    if ('json' in out || 'blob' in out) out['value'] = 'json' in out ? 'json' : 'blob';
    return out;
  }
  if (key === 'json') {
    if (typeof value !== 'string' || !/^[A-Za-z0-9+/]*={0,2}$/.test(value)) throw new MalformedHistory('a payload is not base64');
    return Buffer.from(value, 'base64');
  }
  if (INT64_FIELDS.has(key)) return int(value, key);
  return value;
}

/** Maps one event of an export line (the proto3 JSON mapping) to the runtime's model; unknown fields are ignored. */
export function eventFromProto3Json(json: unknown): HistoryEvent {
  const line = record(json);
  if (!line || typeof line['kind'] !== 'string') throw new MalformedHistory('a history event has no kind');
  if (line['seq'] === undefined) throw new MalformedHistory('a history event has no sequence');
  const wire = fromJson(line, '') as Record<string, unknown>;
  const members = Object.keys(wire).filter((name) => EVENT_BODIES.has(name));
  if (members.length > 1) throw new MalformedHistory(`event ${String(wire['seq'])} has more than one body`);
  if (members[0]) wire['body'] = members[0];
  return eventFromWire(wire);
}

/** The proto3 JSON mapping of a model event, as an export line holds it (the test environment writes its export so). */
export function eventToProto3Json(event: HistoryEvent): Record<string, unknown> {
  const payload = (value: unknown): Record<string, unknown> => ({ json: Buffer.from(rfc8785(value), 'utf8').toString('base64') });
  const node = (n: number): string => `c-${n}`;
  const out: Record<string, unknown> = { seq: String(event.seq) };
  const put = (kind: string, member: string, body: Record<string, unknown>, commandNumber?: number): void => {
    out['kind'] = kind;
    if (event.origin) out['origin'] = ORIGIN_NAMES[event.origin];
    if (commandNumber !== undefined) out['commandNumber'] = String(commandNumber);
    if (event.atUs !== undefined) out['atUs'] = String(event.atUs);
    out[member] = body;
  };
  switch (event.kind) {
    case 'run.started': {
      const body: Record<string, unknown> = { input: payload(event.input ?? null) };
      if (event.carriedState !== undefined) body['carriedState'] = payload(event.carriedState);
      put('HISTORY_RUN_STARTED', 'runStarted', body);
      break;
    }
    case 'workflow_task.started': {
      const body: Record<string, unknown> = {};
      if (event.continueAsNewSuggested) body['continueAsNewSuggested'] = true;
      if (event.historyLimitReached) body['historyLimitReached'] = true;
      put('HISTORY_WORKFLOW_TASK_STARTED', 'workflowTaskStarted', body);
      break;
    }
    case 'workflow_task.completed':
      put('HISTORY_WORKFLOW_TASK_COMPLETED', 'workflowTaskCompleted', event.deferral
        ? { deferred: true, deferredKind: commandKindName(event.deferral.kind), deferredTarget: event.deferral.target } : {});
      break;
    case 'command.record': {
      const body: Record<string, unknown> = { kind: commandKindName(event.commandKind) };
      if (event.target !== '') body['target'] = event.target;
      if (event.sagaScope) body['sagaScope'] = event.sagaScope;
      put('HISTORY_COMMAND_RECORDED', 'commandRecord', body, event.commandNumber);
      break;
    }
    case 'marker.recorded': {
      const body: Record<string, unknown> = { markerId: event.markerId };
      if (event.markerKind === 'side_effect') { body['sideEffect'] = true; body['value'] = payload(event.value ?? null); }
      put('HISTORY_MARKER_RECORDED', 'markerRecorded', body, event.commandNumber);
      break;
    }
    case 'step.outcome': {
      const name = Object.entries(STEP_OUTCOMES).find(([, status]) => status === event.status)![0];
      const body: Record<string, unknown> = { node: node(event.commandNumber), outcome: name };
      if (event.status === 'completed') body['output'] = payload(event.output ?? null);
      if (event.errorCode) body['errorCode'] = event.errorCode;
      put('HISTORY_STEP_OUTCOME', 'stepOutcome', body);
      break;
    }
    case 'timer.fired': put('HISTORY_TIMER_FIRED', 'timerFired', { node: node(event.commandNumber) }); break;
    case 'wait.outcome':
      put('HISTORY_WAIT_OUTCOME', 'waitOutcome', event.status === 'timed_out'
        ? { node: node(event.commandNumber), timedOut: true } : { node: node(event.commandNumber), payload: payload(event.payload ?? null) });
      break;
    case 'child.finished': {
      const body: Record<string, unknown> = { node: node(event.commandNumber), status: event.status.toUpperCase() };
      if (event.status === 'completed') body['output'] = payload(event.output ?? null);
      if (event.errorCode) body['errorCode'] = event.errorCode;
      put('HISTORY_CHILD_FINISHED', 'childFinished', body);
      break;
    }
    case 'approval.decided': put('HISTORY_APPROVAL_DECIDED', 'approvalDecided', { node: node(event.commandNumber), decision: event.decision }); break;
    case 'saga.outcome': put('HISTORY_SAGA_OUTCOME', 'sagaOutcome', { scope: event.scope, outcome: event.outcome }); break;
    case 'signal.buffered': put('HISTORY_SIGNAL_BUFFERED', 'signalBuffered', { signalName: event.name, payload: payload(event.payload ?? null) }); break;
    case 'update.admitted': put('HISTORY_UPDATE_ADMITTED', 'updateAdmitted', { updateName: event.name, args: payload(event.args ?? null) }); break;
    case 'run.cancel_requested': put('HISTORY_CANCELLATION_REQUESTED', 'cancellationRequested', {}); break;
  }
  return out;
}

// ---------------------------------------------------------------------------------------------------------------------
// Commands and failure reports.

const VALUE_FIELD: Partial<Record<CommandKind, 'input' | 'subject'>> = { activity: 'input', script: 'input', child: 'input', approval: 'subject' };
const FRAGMENT_KINDS = new Set<CommandKind>(['activity', 'script', 'timer_start', 'wait_event', 'child', 'approval']);

/** How a value travels: inline canonical JSON, a blob the worker wrote, or (a re-emitted deferral) its descriptor alone. */
export type ValueCarrier = { blobId: string } | 'descriptor' | undefined;

function commandValue(value: unknown, carrier: ValueCarrier): Record<string, unknown> {
  if (carrier === undefined) return { json: Buffer.from(rfc8785(value), 'utf8') };
  const descriptor = descriptorOf(value);
  const named = { descriptor_sha256: descriptor.sha256, descriptor_size: descriptor.size };
  return carrier === 'descriptor' ? named : { blob_id: carrier.blobId, ...named };
}

/**
 * The `Command` of a prepared command, in the loader's object form. `carrier` says how its blob-capable value travels
 * (undefined: inline); `commandNumber` 0 is a re-emitted deferral.
 */
export function commandToWire(command: PreparedCommand, commandNumber: number, carrier: ValueCarrier): Record<string, unknown> & { command_number: number; kind: string } {
  const wire: Record<string, unknown> & { command_number: number; kind: string } = { command_number: commandNumber, kind: commandKindName(command.kind) };
  const body = command.body;
  if (FRAGMENT_KINDS.has(command.kind)) {
    const field = VALUE_FIELD[command.kind];
    const fragment: Record<string, unknown> = { ...body };
    if (field) delete fragment[field];
    wire['fragment_json'] = Buffer.from(rfc8785(fragment), 'utf8');
    // An approval's subject is always inline; an activity's, a script's or a child's input may be a blob.
    if (field) wire['value'] = commandValue(body[field] ?? null, command.kind === 'approval' ? undefined : carrier);
    if (command.sagaScope !== undefined && (command.kind === 'activity' || command.kind === 'script')) wire['saga_scope'] = command.sagaScope;
    return wire;
  }
  switch (command.kind) {
    case 'timer_cancel': case 'step_cancel': wire['target'] = { target: body['target'] }; break;
    case 'saga_open':
      wire['saga_open'] = {
        scope: body['scope'], parent_scope: body['parentScope'] ?? '', on_compensation_failure: body['onCompensationFailure'], cancel_policy: body['cancelPolicy'],
      };
      break;
    case 'saga_close': wire['saga_close'] = { target: body['scope'] }; break;
    case 'saga_abort': wire['saga_abort'] = { target: body['scope'] }; break;
    case 'marker_patch': wire['marker'] = { marker_id: body['markerId'] }; break;
    case 'marker_side_effect': wire['marker'] = { marker_id: body['markerId'], side_effect_value: commandValue(body['value'], carrier) }; break;
    case 'set_state': wire['set_state'] = commandValue(body['state'], undefined); break;
    case 'update_accept': case 'update_reject':
      wire['update_decision'] = { admitted_seq: body['admittedSeq'], accepted: body['accepted'] === true, rejection_reason: body['accepted'] === true ? '' : body['reason'] };
      break;
    case 'update_complete':
      wire['update_result'] = 'result' in body
        ? { admitted_seq: body['admittedSeq'], result: commandValue(body['result'], carrier), failure_code: '' }
        : { admitted_seq: body['admittedSeq'], failure_code: body['failureCode'] };
      break;
    case 'consume_signals':
      wire['consume_signals'] = { signals: (body['signals'] as { name: string; seqs: number[] }[]).map((signal) => ({ signal_name: signal.name, seqs: signal.seqs })) };
      break;
    case 'complete': wire['complete'] = commandValue(body['output'], carrier); break;
    case 'fail': wire['fail'] = { code: body['code'], safe_message: body['message'] }; break;
    case 'continue_as_new': {
      const next: Record<string, unknown> = { input: commandValue(body['input'], undefined), active_version: body['version'] === 'active' };
      if (body['state'] !== null && body['state'] !== undefined) next['state'] = commandValue(body['state'], undefined);
      wire['continue_as_new'] = next;
      break;
    }
    case 'acknowledge_cancellation': wire['acknowledge_cancellation'] = {}; break;
    default: break;
  }
  return wire;
}

/** The value of a command that may travel as a blob (the field the runtime moved), for `WriteBlob`. */
export function blobValueOf(command: PreparedCommand): unknown {
  switch (command.kind) {
    case 'activity': case 'script': case 'child': return command.body['input'] ?? null;
    case 'marker_side_effect': return command.body['value'];
    case 'update_complete': return command.body['result'];
    case 'complete': return command.body['output'];
    default: return undefined;
  }
}

/** The fields of a `FailWorkflowTaskRequest` that a failure gives. */
export function failureToWire(failure: WorkflowTaskFailure): {
  cause: string; command_number: number; actual_kind: string; actual_target: string; actual_saga_scope: string; exception_type: string;
} {
  return {
    cause: `CAUSE_${failure.cause}`,
    command_number: failure.commandNumber ?? 0,
    actual_kind: commandKindName(failure.actualKind ?? ''),
    actual_target: failure.actualTarget ?? '',
    actual_saga_scope: failure.actualSagaScope ?? '',
    exception_type: failure.exceptionType ?? '',
  };
}

type DistributiveOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never;
