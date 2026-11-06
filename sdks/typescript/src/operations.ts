// The client surface of feature 018: schedules, business identifiers, batch jobs, the with-start calls and
// the reset of a run (`specs/018-schedules-bulk-operations/contracts/sdk-conformance.md`). One module, three
// groups reached from `SagatClient` (`schedules`, `batch`, `businessIds`).
//
// Rules shared with SPEC §3: every call that changes something carries one `Idempotency-Key`, given or
// generated once and kept by every retry; a value of an enumeration this SDK does not know is kept as the raw
// string it came as, never an error; every typed answer keeps the JSON it was read from in `raw`.
import { randomUUID } from 'node:crypto';
import { SagatError, invalid } from './errors.js';
import type { HttpRequest, HttpResponse } from './http.js';
import { instant } from './json.js';
import { checkTraceparent } from './traceparent.js';
import { idOf, namespaceIdOf, num, object, optDate, optStr, revision, str } from './wire.js';

/** What the client modules need of the transport; `SagatClient` supplies it. */
export interface Transport {
  request(req: HttpRequest): Promise<HttpResponse>;
}

/** Options of a call that changes something. */
export interface MutationOptions {
  /** 8 to 160 printable ASCII characters. A fresh one per call when absent, reused by every retry of that call. */
  idempotencyKey?: string;
  traceparent?: string;
  signal?: AbortSignal;
}

export interface ReadOptions {
  traceparent?: string;
  signal?: AbortSignal;
}

export interface PageOptions extends ReadOptions {
  cursor?: string;
  limit?: number;
}

export interface Page<T> {
  items: T[];
  nextCursor: string | null;
}

const KEY = /^[\x21-\x7e]{8,160}$/;
const BUSINESS_ID_PATH = /^[\x21-\x7e]{1,200}$/;

function key(given?: string): string {
  const value = given ?? randomUUID();
  if (!KEY.test(value)) throw invalid('an idempotency key is 8 to 160 printable ASCII characters');
  return value;
}

function trace(value?: string): string | undefined {
  return value === undefined ? undefined : checkTraceparent(value);
}

/** An instant given as a `Date` or as the RFC 3339 text the caller already has. Sent in UTC, as a `Z` instant. */
function when(value: Date | string, what: string): string {
  const date = value instanceof Date ? value : new Date(value);
  if (Number.isNaN(date.getTime())) throw invalid(`${what} is not an instant`);
  return instant(date);
}

function page<T>(body: unknown, what: string, read: (item: unknown) => T): Page<T> {
  const raw = object(body, what);
  const items = raw['items'];
  if (!Array.isArray(items)) throw new SagatError({ code: 'PROTOCOL', message: 'items is missing from the page' });
  return { items: items.map(read), nextCursor: optStr(raw, 'nextCursor') };
}

function strings(value: unknown): string[] {
  return Array.isArray(value) ? value.map(String) : [];
}

function counters(value: unknown): Record<string, number> {
  const out: Record<string, number> = {};
  if (value !== null && typeof value === 'object' && !Array.isArray(value)) {
    for (const [name, count] of Object.entries(value as Record<string, unknown>)) {
      if (typeof count === 'number') out[name] = count;
    }
  }
  return out;
}

// -------------------------------------------------------------------------------------------- schedules

export type OverlapPolicy = 'SKIP' | 'BUFFER_ONE' | 'BUFFER_ALL' | 'CANCEL_OTHER' | 'ALLOW_ALL';

export interface ScheduleRange { start: number; end?: number; step?: number }
export type ScheduleField = ScheduleRange[];

export interface ScheduleCalendarSpec {
  second?: ScheduleField; minute?: ScheduleField; hour?: ScheduleField; dayOfMonth?: ScheduleField;
  month?: ScheduleField; year?: ScheduleField; dayOfWeek?: ScheduleField; comment?: string;
}

export interface ScheduleSpec {
  calendars?: ScheduleCalendarSpec[];
  cron?: string[];
  intervals?: { everyMs: number; offsetMs?: number }[];
  exclusions?: ScheduleCalendarSpec[];
  startAt?: string | Date;
  endAt?: string | Date;
  timezone?: string;
  jitterMs?: number;
  dst?: { gap?: 'SKIP' | 'SHIFT_FORWARD'; ambiguous?: 'ONCE_EARLIER' | 'TWICE' };
}

export interface SchedulePolicies {
  overlap?: OverlapPolicy;
  catchupWindowMs?: number;
  catchupMode?: 'ONE' | 'ALL';
  pauseOnFailure?: boolean;
  bufferLimit?: number;
}

export interface ScheduleAction {
  workflowId: string;
  versionPolicy?: 'ACTIVE' | 'PINNED';
  versionId?: string;
  /** The input of every run, at most 64 KiB. It is never returned: only its digest is. */
  input: Record<string, unknown>;
  nominalTimeField?: string;
  runTimeoutMs?: number;
}

/** What a schedule is written as (`createSchedule`, `updateSchedule`). */
export interface ScheduleWrite {
  name: string;
  spec: ScheduleSpec;
  policies?: SchedulePolicies;
  action: ScheduleAction;
  paused?: boolean;
  note?: string;
  remainingActions?: number;
}

export interface Schedule {
  scheduleId: string;
  name: string;
  origin: string;
  state: string;
  revision: number;
  exhausted: boolean;
  /** The action as the server keeps it: without the input, with its digest. */
  action: { workflowId: string | null; versionPolicy: string | null; versionId: string | null; inputDigest: string | null; nominalTimeField: string | null; runTimeoutMs: number | null };
  note: string | null;
  noteBy: string | null;
  remainingActions: number | null;
  nextActionAt: Date | null;
  createdAt: Date;
  updatedAt: Date | null;
  raw: Record<string, unknown>;
}

export interface ScheduleActionRecord {
  nominalTime: Date;
  actualTime: Date | null;
  kind: string;
  origin: string | null;
  revision: number;
  /** One of the outcomes this SDK knows, or another string: the set only grows. */
  outcome: string;
  reason: string | null;
  runId: string | null;
  backfillId: string | null;
  summaryCount: number | null;
  summaryLastNominalTime: Date | null;
  decidedAt: Date;
  raw: Record<string, unknown>;
}

export interface ScheduleDescription extends Schedule {
  nextActions: { nominalTime: Date; actualTime: Date }[];
  recentActions: ScheduleActionRecord[];
  openRuns: string[];
  bufferedCount: number;
  counters: Record<string, number>;
}

export interface ScheduleBackfill {
  backfillId: string;
  start: Date;
  end: Date;
  cursor: Date | null;
  state: string;
  overlap: string;
  counts: Record<string, number>;
  raw: Record<string, unknown>;
}

export interface ScheduleListFilter extends PageOptions {
  state?: 'ACTIVE' | 'PAUSED';
  origin?: 'API' | 'TRIGGER';
  workflowId?: string;
  /** Names that start with this text. */
  name?: string;
}

export interface ScheduleActionFilter extends PageOptions {
  outcome?: string;
}

function toDate(raw: Record<string, unknown>, field: string): Date {
  const value = optDate(raw, field);
  if (!value) throw new SagatError({ code: 'PROTOCOL', message: `${field} is missing from the answer` });
  return value;
}

function toSchedule(value: unknown): Schedule {
  const raw = object(value, 'a schedule');
  const action = raw['action'] && typeof raw['action'] === 'object' ? (raw['action'] as Record<string, unknown>) : {};
  return {
    scheduleId: str(raw, 'scheduleId'),
    name: str(raw, 'name'),
    origin: str(raw, 'origin'),
    state: str(raw, 'state'),
    revision: revision(raw, 'revision', 1),
    exhausted: raw['exhausted'] === true,
    action: {
      workflowId: optStr(action, 'workflowId'), versionPolicy: optStr(action, 'versionPolicy'), versionId: optStr(action, 'versionId'),
      inputDigest: optStr(action, 'inputDigest'), nominalTimeField: optStr(action, 'nominalTimeField'),
      runTimeoutMs: typeof action['runTimeoutMs'] === 'number' ? action['runTimeoutMs'] : null,
    },
    note: optStr(raw, 'note'),
    noteBy: optStr(raw, 'noteBy'),
    remainingActions: typeof raw['remainingActions'] === 'number' ? raw['remainingActions'] : null,
    nextActionAt: optDate(raw, 'nextActionAt'),
    createdAt: toDate(raw, 'createdAt'),
    updatedAt: optDate(raw, 'updatedAt'),
    raw,
  };
}

function toActionRecord(value: unknown): ScheduleActionRecord {
  const raw = object(value, 'a schedule action');
  return {
    nominalTime: toDate(raw, 'nominalTime'),
    actualTime: optDate(raw, 'actualTime'),
    kind: str(raw, 'kind'),
    origin: optStr(raw, 'origin'),
    revision: revision(raw, 'revision', 1),
    outcome: str(raw, 'outcome'),
    reason: optStr(raw, 'reason'),
    runId: optStr(raw, 'runId'),
    backfillId: optStr(raw, 'backfillId'),
    summaryCount: typeof raw['summaryCount'] === 'number' ? raw['summaryCount'] : null,
    summaryLastNominalTime: optDate(raw, 'summaryLastNominalTime'),
    decidedAt: toDate(raw, 'decidedAt'),
    raw,
  };
}

function toDescription(value: unknown): ScheduleDescription {
  const raw = object(value, 'a schedule description');
  const next = raw['nextActions'];
  const recent = raw['recentActions'];
  return {
    ...toSchedule(raw),
    nextActions: Array.isArray(next)
      ? next.map((item) => {
        const entry = object(item, 'a next action');
        return { nominalTime: toDate(entry, 'nominalTime'), actualTime: toDate(entry, 'actualTime') };
      })
      : [],
    recentActions: Array.isArray(recent) ? recent.map(toActionRecord) : [],
    openRuns: strings(raw['openRuns']),
    bufferedCount: num(raw, 'bufferedCount', 0),
    counters: counters(raw['counters']),
  };
}

function toBackfill(value: unknown): ScheduleBackfill {
  const raw = object(value, 'a backfill');
  return {
    backfillId: str(raw, 'backfillId'),
    start: toDate(raw, 'start'),
    end: toDate(raw, 'end'),
    cursor: optDate(raw, 'cursor'),
    state: str(raw, 'state'),
    overlap: str(raw, 'overlap'),
    counts: counters(raw['counts']),
    raw,
  };
}

/** The write of a schedule on the wire: the instants of the spec are sent as `Z` instants. */
function scheduleBody(write: ScheduleWrite): Record<string, unknown> {
  if (write === null || typeof write !== 'object') throw invalid('a schedule is an object');
  const spec: Record<string, unknown> = { ...write.spec };
  if (write.spec?.startAt !== undefined) spec['startAt'] = when(write.spec.startAt, 'startAt');
  if (write.spec?.endAt !== undefined) spec['endAt'] = when(write.spec.endAt, 'endAt');
  return { ...write, spec };
}

export class Schedules {
  constructor(private readonly http: Transport) {}

  private path(namespaceId: string, scheduleId?: string, tail = ''): string {
    const base = `/namespaces/${encodeURIComponent(namespaceIdOf(namespaceId))}/schedules`;
    return scheduleId === undefined ? base : `${base}/${encodeURIComponent(idOf(scheduleId, 'a schedule id'))}${tail}`;
  }

  /** Creates a schedule. A name in use is `CONFLICT`; the same key and body answers what the first call did. */
  async create(namespaceId: string, write: ScheduleWrite, options: MutationOptions = {}): Promise<Schedule> {
    const response = await this.http.request({
      method: 'POST', path: this.path(namespaceId), body: scheduleBody(write),
      idempotencyKey: key(options.idempotencyKey), traceparent: trace(options.traceparent), signal: options.signal, safeToRepeat: true,
    });
    return toSchedule(response.body);
  }

  /** A schedule with its next times, its latest decisions, the runs it has open and its counters. */
  async get(namespaceId: string, scheduleId: string, options: ReadOptions & { nextCount?: number; recentCount?: number } = {}): Promise<ScheduleDescription> {
    const response = await this.http.request({
      method: 'GET', path: this.path(namespaceId, scheduleId),
      query: { nextCount: options.nextCount, recentCount: options.recentCount },
      traceparent: trace(options.traceparent), signal: options.signal, safeToRepeat: true,
    });
    return toDescription(response.body);
  }

  async list(namespaceId: string, filter: ScheduleListFilter = {}): Promise<Page<Schedule>> {
    const response = await this.http.request({
      method: 'GET', path: this.path(namespaceId),
      query: {
        cursor: filter.cursor, limit: filter.limit, state: filter.state, origin: filter.origin, name: filter.name,
        workflowId: filter.workflowId === undefined ? undefined : idOf(filter.workflowId, 'a workflow id'),
      },
      traceparent: trace(filter.traceparent), signal: filter.signal, safeToRepeat: true,
    });
    return page(response.body, 'a page of schedules', toSchedule);
  }

  /** Every schedule the filter selects, following the opaque cursors. */
  async *listAll(namespaceId: string, filter: Omit<ScheduleListFilter, 'cursor'> = {}): AsyncGenerator<Schedule> {
    let cursor: string | undefined;
    do {
      const current = await this.list(namespaceId, { ...filter, cursor });
      for (const item of current.items) yield item;
      cursor = current.nextCursor ?? undefined;
    } while (cursor);
  }

  /** Replaces the spec, the policies and the action as a new revision; a stale `expectedRevision` is `REVISION_CONFLICT`. */
  async update(namespaceId: string, scheduleId: string, write: ScheduleWrite, expectedRevision: number, options: MutationOptions = {}): Promise<Schedule> {
    if (!Number.isSafeInteger(expectedRevision) || expectedRevision < 1) throw invalid('expectedRevision is a positive integer');
    const response = await this.http.request({
      method: 'PUT', path: this.path(namespaceId, scheduleId), body: { ...scheduleBody(write), expectedRevision },
      idempotencyKey: key(options.idempotencyKey), traceparent: trace(options.traceparent), signal: options.signal, safeToRepeat: true,
    });
    return toSchedule(response.body);
  }

  async delete(namespaceId: string, scheduleId: string, expectedRevision: number, options: MutationOptions = {}): Promise<void> {
    if (!Number.isSafeInteger(expectedRevision) || expectedRevision < 1) throw invalid('expectedRevision is a positive integer');
    await this.http.request({
      method: 'DELETE', path: this.path(namespaceId, scheduleId), query: { expectedRevision },
      idempotencyKey: key(options.idempotencyKey), traceparent: trace(options.traceparent), signal: options.signal, safeToRepeat: true,
    });
  }

  private async note(namespaceId: string, scheduleId: string, tail: string, note: string, options: MutationOptions): Promise<Schedule> {
    if (typeof note !== 'string' || note.length === 0) throw invalid('a note is a non-empty text');
    const response = await this.http.request({
      method: 'POST', path: this.path(namespaceId, scheduleId, tail), body: { note },
      idempotencyKey: key(options.idempotencyKey), traceparent: trace(options.traceparent), signal: options.signal, safeToRepeat: true,
    });
    return toSchedule(response.body);
  }

  pause(namespaceId: string, scheduleId: string, note: string, options: MutationOptions = {}): Promise<Schedule> {
    return this.note(namespaceId, scheduleId, '/pause', note, options);
  }

  resume(namespaceId: string, scheduleId: string, note: string, options: MutationOptions = {}): Promise<Schedule> {
    return this.note(namespaceId, scheduleId, '/resume', note, options);
  }

  /** Acts on the schedule now, also while it is paused. The answer says what became of the action. */
  async trigger(namespaceId: string, scheduleId: string, options: MutationOptions & { overlap?: OverlapPolicy } = {}): Promise<ScheduleActionRecord> {
    const body: Record<string, unknown> = {};
    if (options.overlap !== undefined) body['overlap'] = options.overlap;
    const response = await this.http.request({
      method: 'POST', path: this.path(namespaceId, scheduleId, '/trigger'), body,
      idempotencyKey: key(options.idempotencyKey), traceparent: trace(options.traceparent), signal: options.signal, safeToRepeat: true,
    });
    return toActionRecord(response.body);
  }

  /** Acts on every nominal time of the past range `[start, end)` that has no action yet. */
  async backfill(
    namespaceId: string, scheduleId: string, start: Date | string, end: Date | string,
    options: MutationOptions & { overlap?: OverlapPolicy } = {},
  ): Promise<ScheduleBackfill> {
    const body: Record<string, unknown> = { start: when(start, 'start'), end: when(end, 'end') };
    if (options.overlap !== undefined) body['overlap'] = options.overlap;
    const response = await this.http.request({
      method: 'POST', path: this.path(namespaceId, scheduleId, '/backfills'), body,
      idempotencyKey: key(options.idempotencyKey), traceparent: trace(options.traceparent), signal: options.signal, safeToRepeat: true,
    });
    return toBackfill(response.body);
  }

  async cancelBackfill(namespaceId: string, scheduleId: string, backfillId: string, options: MutationOptions = {}): Promise<ScheduleBackfill> {
    const response = await this.http.request({
      method: 'POST', path: this.path(namespaceId, scheduleId, `/backfills/${encodeURIComponent(idOf(backfillId, 'a backfill id'))}/cancel`),
      idempotencyKey: key(options.idempotencyKey), traceparent: trace(options.traceparent), signal: options.signal, safeToRepeat: true,
    });
    return toBackfill(response.body);
  }

  /** At most the latest 100 backfills of the schedule. */
  async listBackfills(namespaceId: string, scheduleId: string, options: ReadOptions = {}): Promise<ScheduleBackfill[]> {
    const response = await this.http.request({
      method: 'GET', path: this.path(namespaceId, scheduleId, '/backfills'),
      traceparent: trace(options.traceparent), signal: options.signal, safeToRepeat: true,
    });
    const items = object(response.body, 'a list of backfills')['items'];
    return Array.isArray(items) ? items.map(toBackfill) : [];
  }

  /** The decisions the schedule has made, newest first. */
  async listActions(namespaceId: string, scheduleId: string, filter: ScheduleActionFilter = {}): Promise<Page<ScheduleActionRecord>> {
    const response = await this.http.request({
      method: 'GET', path: this.path(namespaceId, scheduleId, '/actions'),
      query: { cursor: filter.cursor, limit: filter.limit, outcome: filter.outcome },
      traceparent: trace(filter.traceparent), signal: filter.signal, safeToRepeat: true,
    });
    return page(response.body, 'a page of schedule actions', toActionRecord);
  }
}

// ------------------------------------------------------------------------------------------ business ids

export interface BusinessIdBinding {
  businessId: string;
  /** The run that holds the identifier, or the initial run of the continuation chain that does. */
  executionId: string;
  /** The run that carries the work now. */
  currentRunId: string;
  open: boolean;
  lastStatus: string | null;
  workflowId: string;
  boundAt: Date;
  reusableAfter: Date | null;
  raw: Record<string, unknown>;
}

export class BusinessIds {
  constructor(private readonly http: Transport) {}

  /** How an identifier stands in a namespace. An identifier nobody holds (or that cannot be one) is `NOT_FOUND`. */
  async get(namespaceId: string, businessId: string, options: ReadOptions = {}): Promise<BusinessIdBinding> {
    // Sent as given: no trimming and no case folding. Only what cannot be a path segment is refused here: `.` and
    // `..` (also written `%2E`) are resolved by every URL parser as a step in the path, so the request would reach
    // another route and not the identifier.
    if (typeof businessId !== 'string' || !BUSINESS_ID_PATH.test(businessId) || businessId === '.' || businessId === '..') {
      throw invalid('a business identifier is 1 to 200 visible ASCII characters and not `.` or `..`');
    }
    const response = await this.http.request({
      method: 'GET', path: `/namespaces/${encodeURIComponent(namespaceIdOf(namespaceId))}/business-ids/${encodeURIComponent(businessId)}`,
      traceparent: trace(options.traceparent), signal: options.signal, safeToRepeat: true,
    });
    const raw = object(response.body, 'a business id binding');
    return {
      businessId: str(raw, 'businessId'), executionId: str(raw, 'executionId'), currentRunId: str(raw, 'currentRunId'),
      open: raw['open'] === true, lastStatus: optStr(raw, 'lastStatus'), workflowId: str(raw, 'workflowId'),
      boundAt: toDate(raw, 'boundAt'), reusableAfter: optDate(raw, 'reusableAfter'), raw,
    };
  }
}

// --------------------------------------------------------------------------------------------------- batch

export type BatchSelector =
  | { filter: { workflowId?: string; versionId?: string; status?: string[]; origin?: string; scheduleId?: string; createdAfter?: string | Date; createdBefore?: string | Date; businessIdPrefix?: string } }
  | { runIds: string[] }
  | { query: string };

export interface BatchRequest {
  selector: BatchSelector;
  /** `CANCEL`, `SIGNAL`, `RERUN`, `RETRY_STEP`, `PAUSE`, `RESUME`, `MARK_MANUAL_REVIEW`, ... The set only grows. */
  action: string;
  signal?: { name: string; payload: Record<string, unknown> };
  update?: { name: string; args?: unknown };
  stepId?: string;
  businessId?: 'KEEP' | 'DROP';
  reason: string;
  maxItems?: number;
  itemsPerSecond?: number;
}

export interface BatchPreview {
  matched: number;
  capped: boolean;
  sample: string[];
  raw: Record<string, unknown>;
}

export interface BatchJob {
  batchId: string;
  action: string;
  state: string;
  failureReason: string | null;
  createdBy: string;
  hasPayload: boolean;
  maxItems: number | null;
  itemsPerSecond: number | null;
  counts: { selected: number; succeeded: number; skipped: number; failed: number; cancelled: number; deferred: number };
  createdAt: Date;
  startedAt: Date | null;
  finishedAt: Date | null;
  expiresAt: Date | null;
  raw: Record<string, unknown>;
}

export interface BatchItem {
  ordinal: number;
  runId: string;
  state: string;
  reason: string | null;
  errorCode: string | null;
  attempts: number;
  newRunId: string | null;
  raw: Record<string, unknown>;
}

function toJob(value: unknown): BatchJob {
  const raw = object(value, 'a batch job');
  const counts = raw['counts'] && typeof raw['counts'] === 'object' ? (raw['counts'] as Record<string, unknown>) : {};
  return {
    batchId: str(raw, 'batchId'), action: str(raw, 'action'), state: str(raw, 'state'), failureReason: optStr(raw, 'failureReason'),
    createdBy: str(raw, 'createdBy'), hasPayload: raw['hasPayload'] === true,
    maxItems: typeof raw['maxItems'] === 'number' ? raw['maxItems'] : null,
    itemsPerSecond: typeof raw['itemsPerSecond'] === 'number' ? raw['itemsPerSecond'] : null,
    counts: {
      selected: num(counts, 'selected', 0), succeeded: num(counts, 'succeeded', 0), skipped: num(counts, 'skipped', 0),
      failed: num(counts, 'failed', 0), cancelled: num(counts, 'cancelled', 0), deferred: num(counts, 'deferred', 0),
    },
    createdAt: toDate(raw, 'createdAt'), startedAt: optDate(raw, 'startedAt'), finishedAt: optDate(raw, 'finishedAt'), expiresAt: optDate(raw, 'expiresAt'), raw,
  };
}

function toItem(value: unknown): BatchItem {
  const raw = object(value, 'a batch item');
  return {
    ordinal: revision(raw, 'ordinal'), runId: str(raw, 'runId'), state: str(raw, 'state'), reason: optStr(raw, 'reason'),
    errorCode: optStr(raw, 'errorCode'), attempts: num(raw, 'attempts', 0), newRunId: optStr(raw, 'newRunId'), raw,
  };
}

function selectorBody(selector: BatchSelector): unknown {
  if (selector === null || typeof selector !== 'object') throw invalid('a selector is an object');
  if ('filter' in selector) {
    const filter: Record<string, unknown> = { ...selector.filter };
    if (selector.filter.createdAfter !== undefined) filter['createdAfter'] = when(selector.filter.createdAfter, 'createdAfter');
    if (selector.filter.createdBefore !== undefined) filter['createdBefore'] = when(selector.filter.createdBefore, 'createdBefore');
    return { filter };
  }
  return selector;
}

export class BatchOperations {
  constructor(private readonly http: Transport) {}

  private path(namespaceId: string, tail = ''): string {
    return `/namespaces/${encodeURIComponent(namespaceIdOf(namespaceId))}/batch-operations${tail}`;
  }

  /** Counts what a selector matches, up to `maxItems`; changes nothing. */
  async preview(namespaceId: string, selector: BatchSelector, maxItems?: number, options: ReadOptions = {}): Promise<BatchPreview> {
    const body: Record<string, unknown> = { selector: selectorBody(selector) };
    if (maxItems !== undefined) body['maxItems'] = maxItems;
    const response = await this.http.request({
      method: 'POST', path: this.path(namespaceId, '/preview'), body,
      traceparent: trace(options.traceparent), signal: options.signal, safeToRepeat: true,
    });
    const raw = object(response.body, 'a batch preview');
    return { matched: revision(raw, 'matched'), capped: raw['capped'] === true, sample: strings(raw['sample']), raw };
  }

  /** Creates a durable job. A request sent again with the same key and body is answered with the first job. */
  async create(namespaceId: string, request: BatchRequest, options: MutationOptions = {}): Promise<BatchJob> {
    const response = await this.http.request({
      method: 'POST', path: this.path(namespaceId), body: { ...request, selector: selectorBody(request.selector) },
      idempotencyKey: key(options.idempotencyKey), traceparent: trace(options.traceparent), signal: options.signal, safeToRepeat: true,
    });
    return toJob(response.body);
  }

  async get(namespaceId: string, batchId: string, options: ReadOptions = {}): Promise<BatchJob> {
    const response = await this.http.request({
      method: 'GET', path: this.path(namespaceId, `/${encodeURIComponent(idOf(batchId, 'a batch id'))}`),
      traceparent: trace(options.traceparent), signal: options.signal, safeToRepeat: true,
    });
    return toJob(response.body);
  }

  async list(namespaceId: string, filter: PageOptions & { state?: string } = {}): Promise<Page<BatchJob>> {
    const response = await this.http.request({
      method: 'GET', path: this.path(namespaceId), query: { cursor: filter.cursor, limit: filter.limit, state: filter.state },
      traceparent: trace(filter.traceparent), signal: filter.signal, safeToRepeat: true,
    });
    return page(response.body, 'a page of batch jobs', toJob);
  }

  /** Stops admitting items; what was applied stays applied. A job that already ended is `CONFLICT`. */
  async cancel(namespaceId: string, batchId: string, reason: string, options: MutationOptions = {}): Promise<BatchJob> {
    const response = await this.http.request({
      method: 'POST', path: this.path(namespaceId, `/${encodeURIComponent(idOf(batchId, 'a batch id'))}/cancel`), body: { reason },
      idempotencyKey: key(options.idempotencyKey), traceparent: trace(options.traceparent), signal: options.signal, safeToRepeat: true,
    });
    return toJob(response.body);
  }

  async listItems(namespaceId: string, batchId: string, filter: PageOptions & { state?: string } = {}): Promise<Page<BatchItem>> {
    const response = await this.http.request({
      method: 'GET', path: this.path(namespaceId, `/${encodeURIComponent(idOf(batchId, 'a batch id'))}/items`),
      query: { cursor: filter.cursor, limit: filter.limit, state: filter.state },
      traceparent: trace(filter.traceparent), signal: filter.signal, safeToRepeat: true,
    });
    return page(response.body, 'a page of batch items', toItem);
  }
}

// ------------------------------------------------------------------------------------------- with start

export type BusinessIdConflictPolicy = 'FAIL' | 'USE_EXISTING';
export type BusinessIdReusePolicy = 'ALLOW_DUPLICATE' | 'ALLOW_DUPLICATE_FAILED_ONLY' | 'REJECT_DUPLICATE';

/** The options the start calls share. */
export interface WithStartOptions extends MutationOptions {
  versionId?: string;
  correlationId?: string;
  deadline?: Date;
  deadlineInMs?: number;
  businessIdReusePolicy?: BusinessIdReusePolicy;
}

export interface SignalWithStartResult {
  runId: string;
  /** Whether this request created the run. */
  started: boolean;
  /** The nodes the signal woke. */
  deliveredTo: string[];
  raw: Record<string, unknown>;
}

export interface UpdateWithStartResult<TResult = unknown> {
  runId: string;
  started: boolean;
  update: { outcome: string; receipt: UpdateWithStartReceipt<TResult> | null; reason: string | null };
  raw: Record<string, unknown>;
}

export interface UpdateWithStartReceipt<TResult = unknown> {
  runId: string;
  operation: string;
  updateKey: string;
  status: string;
  result: TResult;
  beforeStateRevision: number;
  afterStateRevision: number;
  runVersion: number;
  replayed: boolean;
  raw: Record<string, unknown>;
}

export function toSignalWithStart(value: unknown): SignalWithStartResult {
  const raw = object(value, 'a signal-with-start result');
  return { runId: str(raw, 'runId'), started: raw['started'] === true, deliveredTo: strings(raw['deliveredTo']), raw };
}

export function toUpdateWithStart<TResult = unknown>(value: unknown): UpdateWithStartResult<TResult> {
  const raw = object(value, 'an update-with-start result');
  const update = object(raw['update'], 'the update of the result');
  const outcome = str(update, 'outcome');
  let receipt: UpdateWithStartReceipt<TResult> | null = null;
  if (update['receipt'] !== undefined && update['receipt'] !== null) {
    const r = object(update['receipt'], 'an update receipt');
    receipt = {
      runId: str(r, 'runId'), operation: str(r, 'operation'), updateKey: str(r, 'updateKey'), status: str(r, 'status'),
      result: r['result'] as TResult, beforeStateRevision: revision(r, 'beforeStateRevision'),
      afterStateRevision: revision(r, 'afterStateRevision'), runVersion: revision(r, 'runVersion'),
      replayed: r['replayed'] === true, raw: r,
    };
  }
  return { runId: str(raw, 'runId'), started: raw['started'] === true, update: { outcome, receipt, reason: optStr(update, 'reason') }, raw };
}

// -------------------------------------------------------------------------------------------------- reset

export interface ResetPoint {
  sequence: number;
  /** `RUN_STARTED`, `STEP_COMPLETED`, ...: the set only grows. */
  kind: string;
  node: string | null;
  occurredAt: Date;
  raw: Record<string, unknown>;
}

export interface ResetPlan {
  sequence: number;
  restored: { node: string; kind: string }[];
  reexecuted: { node: string; completedInSource: boolean; markers: string[] }[];
  reapply: unknown[];
  requiresAcknowledgement: boolean;
  prefixEvents: number;
  raw: Record<string, unknown>;
}

export interface ResetRequest {
  sequence: number;
  reason: string;
  reapply?: 'NONE' | 'SIGNALS' | 'UPDATES' | 'ALL';
  sourcePolicy?: 'REQUIRE_TERMINAL' | 'CANCEL_SOURCE';
  businessId?: 'KEEP' | 'DROP';
  acknowledgeReexecution?: boolean;
}

export interface Reset {
  resetId: string;
  sourceRunId: string;
  sequence: number;
  state: string;
  failureReason: string | null;
  newRunId: string | null;
  reapplyReport: unknown[];
  createdAt: Date | null;
  raw: Record<string, unknown>;
}

export function toResetPoint(value: unknown): ResetPoint {
  const raw = object(value, 'a reset point');
  return { sequence: revision(raw, 'sequence', 1), kind: str(raw, 'kind'), node: optStr(raw, 'node'), occurredAt: toDate(raw, 'occurredAt'), raw };
}

export function toResetPlan(value: unknown): ResetPlan {
  const raw = object(value, 'a reset plan');
  const restored = Array.isArray(raw['restored']) ? raw['restored'] : [];
  const reexecuted = Array.isArray(raw['reexecuted']) ? raw['reexecuted'] : [];
  return {
    sequence: revision(raw, 'sequence', 1),
    restored: restored.map((item) => {
      const r = object(item, 'a restored step');
      return { node: str(r, 'node'), kind: str(r, 'kind') };
    }),
    reexecuted: reexecuted.map((item) => {
      const r = object(item, 'a re-executed step');
      return { node: str(r, 'node'), completedInSource: r['completedInSource'] === true, markers: strings(r['markers']) };
    }),
    reapply: Array.isArray(raw['reapply']) ? raw['reapply'] : [],
    requiresAcknowledgement: raw['requiresAcknowledgement'] === true,
    prefixEvents: revision(raw, 'prefixEvents', 1),
    raw,
  };
}

export function toReset(value: unknown): Reset {
  const raw = object(value, 'a reset');
  return {
    resetId: str(raw, 'resetId'), sourceRunId: str(raw, 'sourceRunId'), sequence: revision(raw, 'sequence', 1), state: str(raw, 'state'),
    failureReason: optStr(raw, 'failureReason'), newRunId: optStr(raw, 'newRunId'),
    reapplyReport: Array.isArray(raw['reapplyReport']) ? raw['reapplyReport'] : [], createdAt: optDate(raw, 'createdAt'), raw,
  };
}
