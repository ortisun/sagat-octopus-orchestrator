import { randomUUID } from 'node:crypto';
import { createHash } from 'node:crypto';
import { Readable } from 'node:stream';
import { safeLogger, silentLogger, warnIfNotSecure, type ClientConfig } from './config.js';
import { SagatError, invalid } from './errors.js';
import { HttpTransport, type HttpResponse } from './http.js';
import { instant } from './json.js';
import { sleep } from './retry.js';
import { checkTraceparent } from './traceparent.js';
import type { BlobDescriptor } from './worker/activity.js';

export type RunStatus =
  | 'RUNNING' | 'WAITING' | 'PAUSED' | 'CANCELLING' | 'COMPENSATING' | 'MANUAL_REVIEW'
  | 'SUCCEEDED' | 'FAILED' | 'CANCELLED' | 'TIMED_OUT' | 'COMPENSATED' | 'PARTIAL' | 'COMPENSATION_FAILED';

export const TERMINAL_STATUSES: readonly RunStatus[] = [
  'SUCCEEDED', 'FAILED', 'CANCELLED', 'TIMED_OUT', 'COMPENSATED', 'PARTIAL', 'COMPENSATION_FAILED',
];

export interface Run {
  runId: string;
  workflowId: string;
  versionId: string;
  status: RunStatus;
  /** Number of journal events applied; usable as `expectedRunVersion`. */
  version: number;
  paused: boolean;
  startedBy: string | null;
  createdAt: Date;
  updatedAt: Date | null;
  finishedAt: Date | null;
  deadline: Date | null;
  traceId: string | null;
  correlationId: string | null;
  depth: number;
  parent: { runId: string; node: string; cancelPolicy: string } | null;
  /** The answer as the server sent it, so a field of a newer server is reachable. */
  raw: Record<string, unknown>;
}

export interface RunPage {
  items: Run[];
  nextCursor: string | null;
}

export interface StartOptions {
  versionId?: string;
  /** 8 to 160 printable ASCII characters. A fresh one per call when absent, reused by every retry of that call. */
  idempotencyKey?: string;
  correlationId?: string;
  /** The instant at which the run times out; must be in the future and within a year. */
  deadline?: Date;
  /** The same, as a duration from now, computed once before the first attempt. */
  deadlineInMs?: number;
  traceparent?: string;
}

export interface UploadRunInputOptions {
  contentType?: string;
  expectedSize?: number;
  expectedSha256?: string;
}

export interface ListOptions {
  workflowId?: string;
  status?: RunStatus;
  limit?: number;
  cursor?: string;
}

export interface CallOptions {
  idempotencyKey?: string;
  traceparent?: string;
}

export interface OperationResult {
  operationId: string;
  accepted: boolean;
  runStatus: string | null;
  runVersion: number | null;
  raw: Record<string, unknown>;
}

export interface SignalResult {
  deliveredTo: string[];
}

export interface PublishEventInput {
  /** Unique within the source for the deduplication window; generated per call when absent. */
  eventId?: string;
  type: string;
  data: Record<string, unknown>;
  subject?: string;
  occurredAt?: Date;
  correlationKey?: string;
  dedupeTtlSeconds?: number;
}

export interface EventAccepted {
  eventId: string;
  source: string;
  status: 'ACCEPTED' | 'DUPLICATE';
}

export interface WaitOptions {
  timeoutMs?: number;
  pollIntervalMs?: number;
  signal?: AbortSignal;
}

export interface JournalEvent {
  raw: Record<string, unknown>;
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** A run id is a UUID: anything else would be a path of its own (`..`, `x/operations`) and is refused before a request (SPEC §10). */
function runIdOf(value: string): string {
  if (typeof value !== 'string' || !UUID.test(value)) throw invalid('a run id is a UUID');
  return value.toLowerCase();
}

/** A name in a URL path segment: never empty and never `.` or `..`; the rest is percent-encoded where it is used. */
function segmentOf(value: string, what: string): string {
  if (typeof value !== 'string' || value === '' || value === '.' || value === '..') throw invalid(`${what} is not a name: it is empty, \`.\` or \`..\``);
  return value;
}
const KEY = /^[\x21-\x7e]{8,160}$/;

function object(value: unknown, what: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new SagatError({ code: 'PROTOCOL', message: `${what} is not an object` });
  }
  return value as Record<string, unknown>;
}

function str(raw: Record<string, unknown>, key: string): string {
  const value = raw[key];
  if (typeof value !== 'string') throw new SagatError({ code: 'PROTOCOL', message: `${key} is missing from the answer` });
  return value;
}

function optStr(raw: Record<string, unknown>, key: string): string | null {
  const value = raw[key];
  return typeof value === 'string' ? value : null;
}

function optDate(raw: Record<string, unknown>, key: string): Date | null {
  const value = raw[key];
  if (typeof value !== 'string') return null;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? null : date;
}

function num(raw: Record<string, unknown>, key: string, fallback: number): number {
  const value = raw[key];
  return typeof value === 'number' ? value : fallback;
}

export function toRun(value: unknown): Run {
  const raw = object(value, 'a run');
  const created = optDate(raw, 'createdAt');
  if (!created) throw new SagatError({ code: 'PROTOCOL', message: 'createdAt is missing from the answer' });
  const parent = raw['parent'] && typeof raw['parent'] === 'object' ? (raw['parent'] as Record<string, unknown>) : null;
  return {
    runId: str(raw, 'runId'),
    workflowId: str(raw, 'workflowId'),
    versionId: str(raw, 'versionId'),
    status: str(raw, 'status') as RunStatus,
    version: num(raw, 'version', 0),
    paused: raw['paused'] === true,
    startedBy: optStr(raw, 'startedBy'),
    createdAt: created,
    updatedAt: optDate(raw, 'updatedAt'),
    finishedAt: optDate(raw, 'finishedAt'),
    deadline: optDate(raw, 'deadline'),
    traceId: optStr(raw, 'traceId'),
    correlationId: optStr(raw, 'correlationId'),
    depth: num(raw, 'depth', 0),
    parent: parent
      ? { runId: String(parent['runId']), node: String(parent['node']), cancelPolicy: String(parent['cancelPolicy']) }
      : null,
    raw,
  };
}

function toPage(body: unknown): RunPage {
  const raw = object(body, 'a page');
  const items = raw['items'];
  if (!Array.isArray(items)) throw new SagatError({ code: 'PROTOCOL', message: 'items is missing from the page' });
  return { items: items.map(toRun), nextCursor: optStr(raw, 'nextCursor') };
}

/** The client of the REST API (SPEC §3). */
export class SagatClient {
  // `#` fields: neither JSON.stringify, util.inspect nor Object.entries of a client can reach the token.
  readonly #http: HttpTransport;
  readonly #config: ClientConfig;
  private readonly names = new Map<string, { id: string; at: number }>();

  constructor(config: ClientConfig) {
    this.#config = config;
    if (!config.apiUrl || !config.namespace) throw invalid('apiUrl and namespace are required');
    warnIfNotSecure(config.apiUrl, safeLogger(config.logger ?? silentLogger));
    this.#http = new HttpTransport(config);
  }

  private trace(explicit?: string): string | undefined {
    const value = explicit ?? this.#config.traceContext?.();
    return value === undefined ? undefined : checkTraceparent(value);
  }

  private key(given?: string): string {
    const key = given ?? randomUUID();
    if (!KEY.test(key)) throw invalid('an idempotency key is 8 to 160 printable ASCII characters');
    return key;
  }

  /** The id of a workflow given by id or by name (the active version of the workflow of that name). */
  async resolveWorkflow(workflow: string): Promise<string> {
    if (UUID.test(workflow)) return workflow.toLowerCase();
    const cached = this.names.get(workflow);
    if (cached && Date.now() - cached.at < 60_000) return cached.id;
    let cursor: string | undefined;
    do {
      const response: HttpResponse = await this.#http.request({
        method: 'GET',
        path: '/workflows',
        query: { limit: 200, cursor },
        safeToRepeat: true,
        traceparent: this.trace(),
      });
      const page = object(response.body, 'a page');
      const items = Array.isArray(page['items']) ? page['items'] : [];
      for (const item of items) {
        const draft = object(item, 'a workflow');
        if (draft['name'] === workflow) {
          const id = str(draft, 'id');
          this.names.set(workflow, { id, at: Date.now() });
          return id;
        }
      }
      cursor = optStr(page, 'nextCursor') ?? undefined;
    } while (cursor);
    throw new SagatError({ code: 'NOT_FOUND', message: `no workflow is named ${JSON.stringify(workflow)} in this namespace`, status: 404, reason: 'WORKFLOW_NOT_FOUND' });
  }

  /** Starts a run of a published workflow, given by id or by name. */
  async start(workflow: string, input: Record<string, unknown>, options: StartOptions = {}): Promise<Run> {
    // What the SDK can refuse by itself is refused before anything is asked of the server.
    const idempotencyKey = this.key(options.idempotencyKey);
    const traceparent = this.trace(options.traceparent);
    const workflowId = await this.resolveWorkflow(workflow);
    let deadline = options.deadline;
    if (options.deadlineInMs !== undefined) deadline = new Date(Date.now() + options.deadlineInMs);
    const body: Record<string, unknown> = { workflowId, input };
    if (options.versionId) body['versionId'] = options.versionId;
    if (options.correlationId !== undefined) body['correlationId'] = options.correlationId;
    if (deadline) body['deadline'] = instant(deadline);
    const response = await this.#http.request({
      method: 'POST',
      path: '/runs',
      body,
      idempotencyKey,
      traceparent,
      safeToRepeat: true,
    });
    return toRun(response.body);
  }

  /** Uploads a one-pass run input. The returned descriptor is accepted only when it matches the bytes sent. */
  async uploadRunInput(
    source: AsyncIterable<Uint8Array> | Iterable<Uint8Array>,
    options: UploadRunInputOptions = {},
  ): Promise<BlobDescriptor> {
    if (options.expectedSize !== undefined && (!Number.isSafeInteger(options.expectedSize) || options.expectedSize < 0)) {
      throw invalid('blob expectedSize is a non-negative safe integer');
    }
    if (options.expectedSha256 !== undefined && !/^[0-9a-f]{64}$/.test(options.expectedSha256)) {
      throw invalid('blob expectedSha256 is lowercase SHA-256 hex');
    }
    const digest = createHash('sha256');
    let size = 0;
    let sourceEof = false;
    const measured = async function* (): AsyncIterable<Buffer> {
      for await (const value of source) {
        if (!(value instanceof Uint8Array)) throw invalid('a blob source yields Uint8Array chunks');
        const chunk = Buffer.from(value);
        size += chunk.length;
        digest.update(chunk);
        yield chunk;
      }
      sourceEof = true;
    };
    const headers: Record<string, string> = {};
    if (options.expectedSize !== undefined) headers['x-octopus-blob-size'] = String(options.expectedSize);
    if (options.expectedSha256 !== undefined) headers['x-octopus-blob-sha256'] = options.expectedSha256;
    const response = await this.#http.request({
      method: 'POST', path: '/run-input-blobs', rawBody: Readable.from(measured()),
      contentType: options.contentType, headers, safeToRepeat: false, traceparent: this.trace(),
    });
    const sha256 = digest.digest('hex');
    const raw = object(response.body, 'a blob descriptor');
    const descriptor = { id: str(raw, 'id'), size: num(raw, 'size', -1), sha256: str(raw, 'sha256'), contentType: str(raw, 'contentType') };
    if (!sourceEof || !UUID.test(descriptor.id) || descriptor.size !== size || descriptor.sha256 !== sha256 || descriptor.contentType === '') {
      throw new SagatError({ code: 'PROTOCOL', message: 'the run-input descriptor does not match the uploaded bytes' });
    }
    return descriptor;
  }

  /** Starts a run by atomically claiming a descriptor returned by uploadRunInput. */
  async startWithBlob(workflow: string, descriptor: BlobDescriptor, options: StartOptions = {}): Promise<Run> {
    if (!descriptor || !UUID.test(descriptor.id) || !Number.isSafeInteger(descriptor.size) || descriptor.size < 0 ||
        !/^[0-9a-f]{64}$/.test(descriptor.sha256) || !descriptor.contentType) {
      throw invalid('a run-input blob descriptor is malformed');
    }
    const idempotencyKey = this.key(options.idempotencyKey);
    const traceparent = this.trace(options.traceparent);
    const workflowId = await this.resolveWorkflow(workflow);
    let deadline = options.deadline;
    if (options.deadlineInMs !== undefined) deadline = new Date(Date.now() + options.deadlineInMs);
    const body: Record<string, unknown> = { workflowId, inputBlobRef: descriptor.id };
    if (options.versionId) body['versionId'] = options.versionId;
    if (options.correlationId !== undefined) body['correlationId'] = options.correlationId;
    if (deadline) body['deadline'] = instant(deadline);
    const response = await this.#http.request({ method: 'POST', path: '/runs', body, idempotencyKey, traceparent, safeToRepeat: true });
    return toRun(response.body);
  }

  async get(runId: string): Promise<Run> {
    const response = await this.#http.request({ method: 'GET', path: `/runs/${encodeURIComponent(runIdOf(runId))}`, safeToRepeat: true, traceparent: this.trace() });
    return toRun(response.body);
  }

  async list(options: ListOptions = {}): Promise<RunPage> {
    const response = await this.#http.request({
      method: 'GET',
      path: '/runs',
      query: { workflowId: options.workflowId, status: options.status, limit: options.limit, cursor: options.cursor },
      safeToRepeat: true,
      traceparent: this.trace(),
    });
    return toPage(response.body);
  }

  /** Every run that matches, page after page. */
  async *listAll(options: Omit<ListOptions, 'cursor'> = {}): AsyncGenerator<Run> {
    let cursor: string | undefined;
    do {
      const page = await this.list({ ...options, cursor });
      yield* page.items;
      cursor = page.nextCursor ?? undefined;
    } while (cursor);
  }

  async operate(
    runId: string,
    action: 'PAUSE' | 'RESUME' | 'CANCEL' | 'RETRY_STEP' | 'RETRY_COMPENSATION' | 'SKIP_COMPENSATION' | 'RESOLVE_STEP',
    reason: string,
    options: CallOptions & { stepId?: string; outcome?: 'SUCCEEDED' | 'FAILED'; errorCode?: string; expectedRunVersion?: number } = {},
  ): Promise<OperationResult> {
    const body: Record<string, unknown> = { action, reason };
    if (options.stepId !== undefined) body['stepId'] = options.stepId;
    if (options.outcome !== undefined) body['outcome'] = options.outcome;
    if (options.errorCode !== undefined) body['errorCode'] = options.errorCode;
    if (options.expectedRunVersion !== undefined) body['expectedRunVersion'] = options.expectedRunVersion;
    const response = await this.#http.request({
      method: 'POST',
      path: `/runs/${encodeURIComponent(runIdOf(runId))}/operations`,
      body,
      idempotencyKey: this.key(options.idempotencyKey),
      traceparent: this.trace(options.traceparent),
      safeToRepeat: true,
    });
    const raw = object(response.body, 'an operation result');
    return {
      operationId: str(raw, 'operationId'),
      accepted: raw['accepted'] === true,
      runStatus: optStr(raw, 'runStatus'),
      runVersion: typeof raw['runVersion'] === 'number' ? raw['runVersion'] : null,
      raw,
    };
  }

  /** Cancels a run (the server's reason has 8 to 2000 characters). */
  cancel(runId: string, reason: string, options: CallOptions & { expectedRunVersion?: number } = {}): Promise<OperationResult> {
    return this.operate(runId, 'CANCEL', reason, options);
  }

  /** Sends a signal to the steps of a run that wait for it; a signal nobody waits for is a `CONFLICT`. */
  async signal(runId: string, name: string, payload: Record<string, unknown>, options: CallOptions = {}): Promise<SignalResult> {
    const response = await this.#http.request({
      method: 'POST',
      path: `/runs/${encodeURIComponent(runIdOf(runId))}/signals/${encodeURIComponent(segmentOf(name, 'a signal name'))}`,
      body: payload,
      idempotencyKey: this.key(options.idempotencyKey),
      traceparent: this.trace(options.traceparent),
      safeToRepeat: true,
    });
    const raw = object(response.body, 'a signal result');
    const delivered = raw['deliveredTo'];
    return { deliveredTo: Array.isArray(delivered) ? delivered.map(String) : [] };
  }

  /** Publishes an event to the inbox of the namespace, once per event id for the deduplication window. */
  async publishEvent(event: PublishEventInput, options: CallOptions = {}): Promise<EventAccepted> {
    const body: Record<string, unknown> = { eventId: event.eventId ?? randomUUID(), type: event.type, data: event.data };
    if (event.subject !== undefined) body['subject'] = event.subject;
    if (event.occurredAt) body['occurredAt'] = instant(event.occurredAt);
    if (event.correlationKey !== undefined) body['correlationKey'] = event.correlationKey;
    if (event.dedupeTtlSeconds !== undefined) body['dedupeTtlSeconds'] = event.dedupeTtlSeconds;
    const response = await this.#http.request({
      method: 'POST',
      path: '/events',
      body,
      traceparent: this.trace(options.traceparent),
      // The event id is the key: the server answers a repetition DUPLICATE.
      safeToRepeat: true,
    });
    const raw = object(response.body, 'an event answer');
    return { eventId: str(raw, 'eventId'), source: str(raw, 'source'), status: str(raw, 'status') as EventAccepted['status'] };
  }

  /** The journal of a run, a page at a time. */
  async events(runId: string, options: { cursor?: string; limit?: number } = {}): Promise<{ items: JournalEvent[]; nextCursor: string | null }> {
    const response = await this.#http.request({
      method: 'GET',
      path: `/runs/${encodeURIComponent(runIdOf(runId))}/events`,
      query: { cursor: options.cursor, limit: options.limit },
      safeToRepeat: true,
      traceparent: this.trace(),
    });
    const raw = object(response.body, 'a page');
    const items = Array.isArray(raw['items']) ? raw['items'].map((item) => ({ raw: object(item, 'an event') })) : [];
    return { items, nextCursor: optStr(raw, 'nextCursor') };
  }

  /** Polls `get` until the run is in a terminal status or `timeoutMs` (default 60 s) passes. */
  async waitForCompletion(runId: string, options: WaitOptions = {}): Promise<Run> {
    const deadline = Date.now() + (options.timeoutMs ?? 60_000);
    const interval = options.pollIntervalMs ?? 500;
    for (;;) {
      const run = await this.get(runId);
      if (TERMINAL_STATUSES.includes(run.status)) return run;
      if (Date.now() >= deadline) {
        throw new SagatError({ code: 'TRANSPORT', message: `the run did not finish within ${options.timeoutMs ?? 60_000} ms`, reason: 'WAIT_TIMEOUT' });
      }
      await sleep(Math.round(interval * (0.8 + Math.random() * 0.4)), options.signal);
    }
  }
}
