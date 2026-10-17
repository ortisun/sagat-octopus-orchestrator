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
  /**
   * The run's segment in its continuation chain, or null for a run outside any chain. A segment
   * that is `SUCCEEDED` with `completionKind` `CONTINUED` ended into its successor: it is not the
   * process outcome, which is the chain's `finalOutcome` (`waitForChainCompletion`).
   */
  chain: RunChainSegment | null;
  /** The ended run this one was started again from (`RERUN_AS_NEW`), or null for a run that is not a rerun. */
  rerunOf: string | null;
  /** The answer as the server sent it, so a field of a newer server is reachable. */
  raw: Record<string, unknown>;
}

export type SegmentCompletionKind = 'CONTINUED' | 'FINAL';

export interface RunChainSegment {
  chainId: string;
  segmentIndex: number;
  /** Null while the segment is open. */
  completionKind: SegmentCompletionKind | null;
  successorRunId: string | null;
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
  /** For `RERUN_AS_NEW`: the run that was started, which names its origin in `rerunOf`. Null for any other action. */
  newRunId: string | null;
  raw: Record<string, unknown>;
}

export type ApprovalDecision = 'APPROVE' | 'REJECT';

export interface ApprovalDecisionOptions {
  /** Unique key for this decision. Reuse it only when retrying the same decision. */
  decisionKey: string;
  reasonCode?: string;
  traceparent?: string;
}

export interface ApprovalDecisionReceipt {
  receiptId: string;
  approvalId: string;
  decisionKey: string;
  decision: ApprovalDecision;
  reasonCode: string | null;
  actor: string;
  decidedAt: Date;
  outcomeSequence: number;
  replayed: boolean;
  raw: Record<string, unknown>;
}

export type ApprovalStatus = 'PENDING' | 'APPROVED' | 'REJECTED' | 'EXPIRED' | 'CANCELLED';

export interface ApprovalScope {
  scopeNodeId: string;
  iterationIndex: number;
}

export interface ApprovalListItem {
  approvalId: string;
  runId: string;
  nodeId: string;
  instancePath: ApprovalScope[];
  status: ApprovalStatus;
  deadline: Date;
  settledAt: Date | null;
  raw: Record<string, unknown>;
}

export interface ApprovalPage {
  items: ApprovalListItem[];
  nextCursor: string | null;
  raw: Record<string, unknown>;
}

export interface ApprovalListOptions extends QueryOptions {
  cursor?: string;
  limit?: number;
}

export interface QueryOptions {
  traceparent?: string;
  signal?: AbortSignal;
}

export interface ChainQueryOptions extends QueryOptions {
  expectedRunId?: string;
}

export interface QueryResult<TResult = unknown> {
  runId: string;
  runVersion: number;
  stateRevision: number;
  result: TResult;
  raw: Record<string, unknown>;
}

export interface UpdateOptions {
  /** Caller-chosen durable key. Every retry of this update keeps the exact value. */
  updateKey: string;
  expectedStateRevision?: number;
  traceparent?: string;
  signal?: AbortSignal;
}

export interface ChainUpdateOptions extends UpdateOptions {
  expectedRunId?: string;
}

export interface UpdateReceipt<TResult = unknown> {
  runId: string;
  operation: string;
  updateKey: string;
  status: 'APPLIED';
  result: TResult;
  beforeStateRevision: number;
  afterStateRevision: number;
  runVersion: number;
  replayed: boolean;
  raw: Record<string, unknown>;
}

export interface SignalResult {
  deliveredTo: string[];
}

export type ChainFinalOutcome =
  | 'SUCCEEDED' | 'FAILED' | 'CANCELLED' | 'TIMED_OUT' | 'COMPENSATED' | 'PARTIAL' | 'COMPENSATION_FAILED';

export interface ChainSegment {
  index: number;
  runId: string;
  predecessorRunId: string | null;
  successorRunId: string | null;
}

/** One segment as the paged list of a chain's runs gives it: lineage and lifecycle, never a payload. */
export interface ChainRun {
  index: number;
  runId: string;
  status: RunStatus;
  /** Null while the segment is open. */
  completionKind: SegmentCompletionKind | null;
  predecessorRunId: string | null;
  successorRunId: string | null;
  createdAt: Date;
  finishedAt: Date | null;
}

export interface ChainRunPage {
  items: ChainRun[];
  nextCursor: string | null;
  raw: Record<string, unknown>;
}

export interface ChainRunListOptions extends QueryOptions {
  /** The `nextCursor` of the previous page; opaque. */
  cursor?: string;
  /** 1 to 200; the server clamps values outside that range. */
  limit?: number;
}

export interface ChainInspection {
  chainId: string;
  currentRunId: string;
  revision: number;
  cancelRequested: boolean;
  finalOutcome: ChainFinalOutcome | null;
  segments: ChainSegment[];
  raw: Record<string, unknown>;
}

export interface ChainCommandOptions extends CallOptions {
  expectedRunId: string;
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

function namespaceIdOf(value: string): string {
  if (typeof value !== 'string' || !UUID.test(value)) throw invalid('a namespace id is a UUID');
  return value.toLowerCase();
}

/** A name in a URL path segment: never empty and never `.` or `..`; the rest is percent-encoded where it is used. */
function segmentOf(value: string, what: string): string {
  if (typeof value !== 'string' || value === '' || value === '.' || value === '..') throw invalid(`${what} is not a name: it is empty, \`.\` or \`..\``);
  return value;
}
const KEY = /^[\x21-\x7e]{8,160}$/;
const APPROVAL_DECISION_KEY = /^[\x21-\x7e]{1,128}$/;
const REASON_CODE = /^[A-Za-z0-9_.:-]{1,64}$/;
const DECLARED_OPERATION_NAME = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/;
const UPDATE_KEY = /^[\x21-\x7e]{1,128}$/;

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

function revision(raw: Record<string, unknown>, key: string, minimum = 0): number {
  const value = raw[key];
  if (!Number.isSafeInteger(value) || (value as number) < minimum) {
    throw new SagatError({ code: 'PROTOCOL', message: `${key} is invalid in the answer` });
  }
  return value as number;
}

function operationName(value: string): string {
  if (typeof value !== 'string' || !DECLARED_OPERATION_NAME.test(value)) {
    throw invalid('an operation name is 1 to 128 letters, digits, dots, underscores, colons or hyphens and starts with a letter or digit');
  }
  return value;
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
    chain: toRunChain(raw['chain']),
    rerunOf: optStr(raw, 'rerunOf'),
    raw,
  };
}

function completionKind(value: unknown): SegmentCompletionKind | null {
  if (value === null || value === undefined) return null;
  if (value === 'CONTINUED' || value === 'FINAL') return value;
  throw new SagatError({ code: 'PROTOCOL', message: 'completionKind is invalid in the answer' });
}

function toRunChain(value: unknown): RunChainSegment | null {
  if (value === null || value === undefined) return null;
  const chain = object(value, 'a run chain segment');
  const segmentIndex = revision(chain, 'segmentIndex');
  if (segmentIndex > 9_999) throw new SagatError({ code: 'PROTOCOL', message: 'segmentIndex is invalid in the answer' });
  return {
    chainId: str(chain, 'chainId'), segmentIndex, completionKind: completionKind(chain['completionKind']),
    successorRunId: optStr(chain, 'successorRunId'),
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
    action: 'PAUSE' | 'RESUME' | 'CANCEL' | 'RETRY_STEP' | 'RETRY_COMPENSATION' | 'SKIP_COMPENSATION' | 'RESOLVE_STEP' | 'RERUN_AS_NEW',
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
      newRunId: optStr(raw, 'newRunId'),
      raw,
    };
  }

  /**
   * Starts a new run of the same version, with the same input and correlation id, from a run that ended
   * (needs `run:operate` and `run:start`; a run that has not ended is refused with `CONFLICT`). The answer's
   * `newRunId` is the new run, which names its origin in `rerunOf`; a segment of a continuation chain starts
   * a run with a chain of its own.
   */
  rerunAsNew(runId: string, reason: string, options: CallOptions = {}): Promise<OperationResult> {
    return this.operate(runId, 'RERUN_AS_NEW', reason, options);
  }

  /** Cancels a run (the server's reason has 8 to 2000 characters). */
  cancel(runId: string, reason: string, options: CallOptions & { expectedRunVersion?: number } = {}): Promise<OperationResult> {
    return this.operate(runId, 'CANCEL', reason, options);
  }

  /** Lists the approval requests of one run in immutable opening order. */
  async listApprovals(runId: string, options: ApprovalListOptions = {}): Promise<ApprovalPage> {
    const response = await this.#http.request({
      method: 'GET', path: `/runs/${encodeURIComponent(runIdOf(runId))}/approvals`,
      query: { cursor: options.cursor, limit: options.limit }, traceparent: this.trace(options.traceparent),
      signal: options.signal, safeToRepeat: true,
    });
    const raw = object(response.body, 'an approval page');
    const values = raw['items'];
    if (!Array.isArray(values) || values.length > 200) throw new SagatError({ code: 'PROTOCOL', message: 'items is invalid in the answer' });
    const statuses = new Set<string>(['PENDING', 'APPROVED', 'REJECTED', 'EXPIRED', 'CANCELLED']);
    const items = values.map((value): ApprovalListItem => {
      const item = object(value, 'an approval list item');
      const status = str(item, 'status');
      const deadline = optDate(item, 'deadline');
      if (!statuses.has(status)) throw new SagatError({ code: 'PROTOCOL', message: 'status is invalid in the answer' });
      if (!deadline) throw new SagatError({ code: 'PROTOCOL', message: 'deadline is missing from the answer' });
      const path = item['instancePath'];
      if (!Array.isArray(path) || path.length > 8) throw new SagatError({ code: 'PROTOCOL', message: 'instancePath is invalid in the answer' });
      return {
        approvalId: str(item, 'approvalId'), runId: str(item, 'runId'), nodeId: str(item, 'nodeId'),
        instancePath: path.map((segment) => {
          const scope = object(segment, 'an approval scope');
          const iterationIndex = revision(scope, 'iterationIndex');
          if (iterationIndex > 9_999) throw new SagatError({ code: 'PROTOCOL', message: 'iterationIndex is invalid in the answer' });
          return { scopeNodeId: str(scope, 'scopeNodeId'), iterationIndex };
        }),
        status: status as ApprovalStatus, deadline, settledAt: optDate(item, 'settledAt'), raw: item,
      };
    });
    return { items, nextCursor: optStr(raw, 'nextCursor'), raw };
  }

  /** Streams every approval request by following the server's opaque cursor. */
  async *listAllApprovals(runId: string, options: Omit<ApprovalListOptions, 'cursor'> = {}): AsyncGenerator<ApprovalListItem> {
    let cursor: string | undefined;
    do {
      const page = await this.listApprovals(runId, { ...options, cursor });
      yield* page.items;
      cursor = page.nextCursor ?? undefined;
    } while (cursor);
  }

  /** Records an eligible actor's durable decision for an approval request. */
  async decideApproval(
    approvalId: string,
    decision: ApprovalDecision,
    options: ApprovalDecisionOptions,
  ): Promise<ApprovalDecisionReceipt> {
    if (!UUID.test(approvalId)) throw invalid('an approval id is a UUID');
    if (decision !== 'APPROVE' && decision !== 'REJECT') throw invalid('an approval decision is APPROVE or REJECT');
    if (!options || typeof options.decisionKey !== 'string' || !APPROVAL_DECISION_KEY.test(options.decisionKey)) {
      throw invalid('an approval decision key is 1 to 128 printable ASCII characters');
    }
    if (options.reasonCode !== undefined && (typeof options.reasonCode !== 'string' || !REASON_CODE.test(options.reasonCode))) {
      throw invalid('an approval reason code is 1 to 64 letters, digits, dots, underscores, colons or hyphens');
    }
    const body: Record<string, unknown> = { decisionKey: options.decisionKey, decision };
    if (options.reasonCode !== undefined) body['reasonCode'] = options.reasonCode;
    const response = await this.#http.request({
      method: 'POST',
      path: `/approvals/${encodeURIComponent(approvalId.toLowerCase())}/decisions`,
      body,
      traceparent: this.trace(options.traceparent),
      safeToRepeat: true,
    });
    const raw = object(response.body, 'an approval decision receipt');
    const decidedAt = optDate(raw, 'decidedAt');
    if (!decidedAt) throw new SagatError({ code: 'PROTOCOL', message: 'decidedAt is missing from the answer' });
    const receiptDecision = str(raw, 'decision');
    if (receiptDecision !== 'APPROVE' && receiptDecision !== 'REJECT') {
      throw new SagatError({ code: 'PROTOCOL', message: 'decision is invalid in the answer' });
    }
    const outcomeSequence = raw['outcomeSequence'];
    if (!Number.isSafeInteger(outcomeSequence) || (outcomeSequence as number) < 1) {
      throw new SagatError({ code: 'PROTOCOL', message: 'outcomeSequence is invalid in the answer' });
    }
    return {
      receiptId: str(raw, 'receiptId'), approvalId: str(raw, 'approvalId'), decisionKey: str(raw, 'decisionKey'),
      decision: receiptDecision, reasonCode: optStr(raw, 'reasonCode'), actor: str(raw, 'actor'), decidedAt,
      outcomeSequence: outcomeSequence as number, replayed: raw['replayed'] === true, raw,
    };
  }

  /** Evaluates a declared side-effect-free query over one committed state snapshot. */
  async query<TResult = unknown>(runId: string, name: string, args: unknown, options: QueryOptions = {}): Promise<QueryResult<TResult>> {
    const response = await this.#http.request({
      method: 'POST',
      path: `/runs/${encodeURIComponent(runIdOf(runId))}/queries/${encodeURIComponent(operationName(name))}`,
      body: { args },
      traceparent: this.trace(options.traceparent),
      signal: options.signal,
      safeToRepeat: true,
    });
    const raw = object(response.body, 'a query result');
    if (!Object.prototype.hasOwnProperty.call(raw, 'result')) {
      throw new SagatError({ code: 'PROTOCOL', message: 'result is missing from the answer' });
    }
    return {
      runId: str(raw, 'runId'),
      runVersion: revision(raw, 'runVersion'),
      stateRevision: revision(raw, 'stateRevision'),
      result: raw['result'] as TResult,
      raw,
    };
  }

  /** Evaluates a declared query against the chain head selected under the server's chain lock. */
  async queryChain<TResult = unknown>(
    namespaceId: string,
    chainId: string,
    name: string,
    args: unknown,
    options: ChainQueryOptions = {},
  ): Promise<QueryResult<TResult>> {
    const body: Record<string, unknown> = { args };
    if (options.expectedRunId !== undefined) body['expectedRunId'] = runIdOf(options.expectedRunId);
    const response = await this.#http.request({
      method: 'POST',
      path: `/namespaces/${encodeURIComponent(namespaceIdOf(namespaceId))}/chains/${encodeURIComponent(runIdOf(chainId))}/queries/${encodeURIComponent(operationName(name))}`,
      body,
      traceparent: this.trace(options.traceparent),
      signal: options.signal,
      safeToRepeat: true,
    });
    const raw = object(response.body, 'a query result');
    if (!Object.prototype.hasOwnProperty.call(raw, 'result')) {
      throw new SagatError({ code: 'PROTOCOL', message: 'result is missing from the answer' });
    }
    return {
      runId: str(raw, 'runId'),
      runVersion: revision(raw, 'runVersion'),
      stateRevision: revision(raw, 'stateRevision'),
      result: raw['result'] as TResult,
      raw,
    };
  }

  /** Applies or replays a declared durable state update. */
  async update<TResult = unknown>(runId: string, name: string, args: unknown, options: UpdateOptions): Promise<UpdateReceipt<TResult>> {
    if (!options || typeof options.updateKey !== 'string' || !UPDATE_KEY.test(options.updateKey)) {
      throw invalid('an update key is 1 to 128 printable ASCII characters');
    }
    if (options.expectedStateRevision !== undefined
      && (!Number.isSafeInteger(options.expectedStateRevision) || options.expectedStateRevision < 0)) {
      throw invalid('expectedStateRevision is a non-negative safe integer');
    }
    const body: Record<string, unknown> = { updateKey: options.updateKey, args };
    if (options.expectedStateRevision !== undefined) body['expectedStateRevision'] = options.expectedStateRevision;
    const response = await this.#http.request({
      method: 'POST',
      path: `/runs/${encodeURIComponent(runIdOf(runId))}/updates/${encodeURIComponent(operationName(name))}`,
      body,
      traceparent: this.trace(options.traceparent),
      signal: options.signal,
      safeToRepeat: true,
    });
    const raw = object(response.body, 'an update receipt');
    if (raw['status'] !== 'APPLIED') {
      throw new SagatError({ code: 'PROTOCOL', message: 'status is invalid in the answer' });
    }
    if (!Object.prototype.hasOwnProperty.call(raw, 'result')) {
      throw new SagatError({ code: 'PROTOCOL', message: 'result is missing from the answer' });
    }
    return {
      runId: str(raw, 'runId'), operation: str(raw, 'operation'), updateKey: str(raw, 'updateKey'),
      status: 'APPLIED', result: raw['result'] as TResult,
      beforeStateRevision: revision(raw, 'beforeStateRevision'),
      afterStateRevision: revision(raw, 'afterStateRevision', 1),
      runVersion: revision(raw, 'runVersion', 1), replayed: raw['replayed'] === true, raw,
    };
  }

  /** Applies or replays a declared update against a chain's current head. */
  async updateChain<TResult = unknown>(
    namespaceId: string, chainId: string, name: string, args: unknown, options: ChainUpdateOptions,
  ): Promise<UpdateReceipt<TResult>> {
    if (!options || typeof options.updateKey !== 'string' || !UPDATE_KEY.test(options.updateKey)) {
      throw invalid('an update key is 1 to 128 printable ASCII characters');
    }
    if (options.expectedStateRevision !== undefined
      && (!Number.isSafeInteger(options.expectedStateRevision) || options.expectedStateRevision < 0)) {
      throw invalid('expectedStateRevision is a non-negative safe integer');
    }
    const body: Record<string, unknown> = { updateKey: options.updateKey, args };
    if (options.expectedRunId !== undefined) body['expectedRunId'] = runIdOf(options.expectedRunId);
    if (options.expectedStateRevision !== undefined) body['expectedStateRevision'] = options.expectedStateRevision;
    const response = await this.#http.request({
      method: 'POST',
      path: `/namespaces/${encodeURIComponent(namespaceIdOf(namespaceId))}/chains/${encodeURIComponent(runIdOf(chainId))}/updates/${encodeURIComponent(operationName(name))}`,
      body,
      traceparent: this.trace(options.traceparent),
      signal: options.signal,
      safeToRepeat: true,
    });
    const raw = object(response.body, 'an update receipt');
    if (raw['status'] !== 'APPLIED') throw new SagatError({ code: 'PROTOCOL', message: 'status is invalid in the answer' });
    if (!Object.prototype.hasOwnProperty.call(raw, 'result')) {
      throw new SagatError({ code: 'PROTOCOL', message: 'result is missing from the answer' });
    }
    return {
      runId: str(raw, 'runId'), operation: str(raw, 'operation'), updateKey: str(raw, 'updateKey'),
      status: 'APPLIED', result: raw['result'] as TResult,
      beforeStateRevision: revision(raw, 'beforeStateRevision'),
      afterStateRevision: revision(raw, 'afterStateRevision', 1),
      runVersion: revision(raw, 'runVersion', 1), replayed: raw['replayed'] === true, raw,
    };
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

  /** Reads one execution chain snapshot, including its current head and immutable lineage. */
  async inspectChain(namespaceId: string, chainId: string, options: QueryOptions = {}): Promise<ChainInspection> {
    const response = await this.#http.request({
      method: 'GET',
      path: `/namespaces/${encodeURIComponent(namespaceIdOf(namespaceId))}/chains/${encodeURIComponent(runIdOf(chainId))}`,
      traceparent: this.trace(options.traceparent), signal: options.signal, safeToRepeat: true,
    });
    const raw = object(response.body, 'a chain inspection');
    const segments = raw['segments'];
    if (!Array.isArray(segments) || segments.length === 0) {
      throw new SagatError({ code: 'PROTOCOL', message: 'segments is invalid in the answer' });
    }
    const outcomes = new Set<string>(TERMINAL_STATUSES);
    const finalOutcome = raw['finalOutcome'];
    if (finalOutcome !== null && (typeof finalOutcome !== 'string' || !outcomes.has(finalOutcome))) {
      throw new SagatError({ code: 'PROTOCOL', message: 'finalOutcome is invalid in the answer' });
    }
    return {
      chainId: str(raw, 'chainId'), currentRunId: str(raw, 'currentRunId'), revision: revision(raw, 'revision'),
      cancelRequested: raw['cancelRequested'] === true, finalOutcome: finalOutcome as ChainFinalOutcome | null,
      segments: segments.map((value) => {
        const segment = object(value, 'a chain segment');
        const index = revision(segment, 'index');
        if (index > 65_535) throw new SagatError({ code: 'PROTOCOL', message: 'index is invalid in the answer' });
        return {
          index, runId: str(segment, 'runId'), predecessorRunId: optStr(segment, 'predecessorRunId'),
          successorRunId: optStr(segment, 'successorRunId'),
        };
      }), raw,
    };
  }

  /**
   * Lists the runs of a chain in segment order, one bounded page at a time. The cursor is opaque
   * and stable: a segment is only appended, so a page never repeats or skips one while the chain
   * grows. Pages carry lineage and lifecycle only, never an input, output or state.
   */
  async listChainRuns(namespaceId: string, chainId: string, options: ChainRunListOptions = {}): Promise<ChainRunPage> {
    const response = await this.#http.request({
      method: 'GET',
      path: `/namespaces/${encodeURIComponent(namespaceIdOf(namespaceId))}/chains/${encodeURIComponent(runIdOf(chainId))}/runs`,
      query: { cursor: options.cursor, limit: options.limit }, traceparent: this.trace(options.traceparent),
      signal: options.signal, safeToRepeat: true,
    });
    const raw = object(response.body, 'a chain run page');
    const values = raw['items'];
    if (!Array.isArray(values) || values.length > 200) throw new SagatError({ code: 'PROTOCOL', message: 'items is invalid in the answer' });
    const items = values.map((value): ChainRun => {
      const item = object(value, 'a chain run');
      const index = revision(item, 'index');
      if (index > 9_999) throw new SagatError({ code: 'PROTOCOL', message: 'index is invalid in the answer' });
      const created = optDate(item, 'createdAt');
      if (!created) throw new SagatError({ code: 'PROTOCOL', message: 'createdAt is missing from the answer' });
      return {
        index, runId: str(item, 'runId'), status: str(item, 'status') as RunStatus,
        completionKind: completionKind(item['completionKind']), predecessorRunId: optStr(item, 'predecessorRunId'),
        successorRunId: optStr(item, 'successorRunId'), createdAt: created, finishedAt: optDate(item, 'finishedAt'),
      };
    });
    return { items, nextCursor: optStr(raw, 'nextCursor'), raw };
  }

  /** Streams every run of a chain, in segment order, by following the server's opaque cursor. */
  async *listAllChainRuns(namespaceId: string, chainId: string, options: Omit<ChainRunListOptions, 'cursor'> = {}): AsyncGenerator<ChainRun> {
    let cursor: string | undefined;
    do {
      const page = await this.listChainRuns(namespaceId, chainId, { ...options, cursor });
      yield* page.items;
      cursor = page.nextCursor ?? undefined;
    } while (cursor);
  }

  /** Signals a root wait on the expected current chain head. */
  async signalChain(namespaceId: string, chainId: string, name: string, payload: unknown, options: ChainCommandOptions): Promise<SignalResult> {
    if (!options) throw invalid('chain signal options are required');
    const response = await this.#http.request({
      method: 'POST',
      path: `/namespaces/${encodeURIComponent(namespaceIdOf(namespaceId))}/chains/${encodeURIComponent(runIdOf(chainId))}/signals/${encodeURIComponent(segmentOf(name, 'a signal name'))}`,
      body: { expectedRunId: runIdOf(options.expectedRunId), payload },
      idempotencyKey: this.key(options.idempotencyKey), traceparent: this.trace(options.traceparent), safeToRepeat: true,
    });
    const raw = object(response.body, 'a chain signal result');
    const delivered = raw['deliveredTo'];
    return { deliveredTo: Array.isArray(delivered) ? delivered.map(String) : [] };
  }

  /** Latches cancellation and cancels the expected current chain head. */
  async cancelChain(namespaceId: string, chainId: string, reason: string, options: ChainCommandOptions): Promise<void> {
    if (!options) throw invalid('chain cancellation options are required');
    if (typeof reason !== 'string' || [...reason].length < 1 || [...reason].length > 512) {
      throw invalid('a chain cancellation reason is 1 to 512 characters');
    }
    await this.#http.request({
      method: 'POST',
      path: `/namespaces/${encodeURIComponent(namespaceIdOf(namespaceId))}/chains/${encodeURIComponent(runIdOf(chainId))}/cancel`,
      body: { expectedRunId: runIdOf(options.expectedRunId), reason },
      idempotencyKey: this.key(options.idempotencyKey), traceparent: this.trace(options.traceparent), safeToRepeat: true,
    });
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

  /** Waits for the chain's final outcome, following committed head changes. */
  async waitForChainCompletion(namespaceId: string, chainId: string, options: WaitOptions = {}): Promise<ChainInspection> {
    const timeout = options.timeoutMs ?? 60_000;
    const deadline = Date.now() + timeout;
    const interval = options.pollIntervalMs ?? 500;
    for (;;) {
      const chain = await this.inspectChain(namespaceId, chainId, { signal: options.signal });
      if (chain.finalOutcome !== null) return chain;
      if (Date.now() >= deadline) {
        throw new SagatError({ code: 'TRANSPORT', message: `the chain did not finish within ${timeout} ms`, reason: 'WAIT_TIMEOUT' });
      }
      await sleep(Math.round(interval * (0.8 + Math.random() * 0.4)), options.signal);
    }
  }
}
