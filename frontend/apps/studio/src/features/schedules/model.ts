import type { Overlap, Schedule, ScheduleWrite } from '../../api/endpoints';
import { isApiError } from '../../api/errors';
import { UnsafeJsonNumber, parseAuthoringJson } from '../../model/safeJson';
import type { StatusView } from '../../model/status';
import { businessIdProblem } from '../runs/businessId';

export const OVERLAPS: readonly Overlap[] = ['SKIP', 'BUFFER_ONE', 'BUFFER_ALL', 'CANCEL_OTHER', 'ALLOW_ALL'];

export const OVERLAP_TEXT: Record<Overlap, { label: string; help: string }> = {
  SKIP: { label: 'Skip', help: 'A time that falls while a run of the schedule is still open is skipped.' },
  BUFFER_ONE: { label: 'Keep one', help: 'The most recent such time waits for the open run to end, then starts.' },
  BUFFER_ALL: { label: 'Keep all', help: 'Every such time waits, up to a limit, and starts in turn.' },
  CANCEL_OTHER: { label: 'Cancel the open run', help: 'The open run is asked to stop, and the new time starts when it has.' },
  ALLOW_ALL: { label: 'Allow overlap', help: 'The new time starts whether or not a run is open.' },
};

export const UNITS = { SECONDS: 1_000, MINUTES: 60_000, HOURS: 3_600_000, DAYS: 86_400_000 } as const;
export type Unit = keyof typeof UNITS;
export const UNIT_LABEL: Record<Unit, string> = { SECONDS: 'seconds', MINUTES: 'minutes', HOURS: 'hours', DAYS: 'days' };

export const MAX_INPUT_BYTES = 64 * 1024;
export const MAX_NOTE = 512;
/** The longest prefix of a business id the platform takes (visible ASCII, 1 to 160 characters). */
export const MAX_BUSINESS_PREFIX = 160;
const UUID = /^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i;

export type When = 'INTERVAL' | 'CRON';

// --- the business id of what a schedule starts ----------------------------------------------------

/** The closed values of the contract: a value it adds is a type error here until the table below says it in words. */
export type BusinessConflict = NonNullable<NonNullable<Schedule['action']>['businessIdConflictPolicy']>;
export type BusinessReuse = NonNullable<NonNullable<Schedule['action']>['businessIdReusePolicy']>;

export const BUSINESS_CONFLICTS: readonly BusinessConflict[] = ['FAIL', 'USE_EXISTING'];
export const BUSINESS_REUSES: readonly BusinessReuse[] = ['ALLOW_DUPLICATE', 'ALLOW_DUPLICATE_FAILED_ONLY', 'REJECT_DUPLICATE'];
export const DEFAULT_CONFLICT: BusinessConflict = 'FAIL';
export const DEFAULT_REUSE: BusinessReuse = 'ALLOW_DUPLICATE';

/** What an action does when an open run holds its business id: `label` for a choice, `effect` to finish "the action ...". */
export const BUSINESS_CONFLICT_TEXT: Record<BusinessConflict, { label: string; effect: string }> = {
  FAIL: { label: 'Record a conflict and start nothing', effect: 'records a business id conflict and starts nothing' },
  USE_EXISTING: { label: 'Record that run and start nothing', effect: 'records that run as the answer and starts nothing' },
};

/** What an action does when a closed run still remembers its business id. */
export const BUSINESS_REUSE_TEXT: Record<BusinessReuse, { label: string; effect: string }> = {
  ALLOW_DUPLICATE: { label: 'Start a new run', effect: 'starts a new run' },
  ALLOW_DUPLICATE_FAILED_ONLY: { label: 'Start a new run unless it succeeded', effect: 'starts a new run unless that run succeeded, which is a business id conflict' },
  REJECT_DUPLICATE: { label: 'Record a conflict and start nothing', effect: 'records a business id conflict and starts nothing' },
};

/** The two policies in one sentence, for whoever is about to choose them. */
export function businessPolicySentence(conflict: BusinessConflict, reuse: BusinessReuse): string {
  return `If an open run holds the id, the action ${BUSINESS_CONFLICT_TEXT[conflict].effect}; if a closed run still remembers it (30 days by default), the action ${BUSINESS_REUSE_TEXT[reuse].effect}.`;
}

/**
 * Why the text of the prefix field cannot be sent, or nothing. An empty field is how a person says "no prefix"; spaces alone are
 * refused on the field, never read as that, so that a schedule does not lose the prefix someone thought they had written. The
 * text around a prefix is trimmed, as the request does.
 */
export function businessPrefixProblem(text: string): string | undefined {
  const trimmed = text.trim();
  if (trimmed === '' && text !== '') return 'Spaces alone are not a prefix: write one, or empty the field.';
  return businessIdProblem(trimmed, MAX_BUSINESS_PREFIX);
}

/** Whether the platform refused the prefix of the business id, as opposed to anything else about the schedule. */
export function refusedPrefix(error: unknown): boolean {
  return isApiError(error) && error.code === 'VALIDATION_FAILED' && error.reason === 'SCHEDULE_SPEC_INVALID' && error.details?.['field'] === '/action/businessIdPrefix';
}

/** An instant as the platform writes the time of a business id: RFC 3339 in UTC, to the second, ending in `Z`. */
function toTheSecond(at: Date): string {
  return at.toISOString().replace(/\.\d{3}Z$/, 'Z');
}

/**
 * The business id an action would make: the prefix, a dash and the nominal time. The time is the platform's (the next nominal time of
 * the schedule); with none to go by it is the clock's, so that the shape can be shown. Nothing here is sent.
 */
export function businessIdPreview(prefix: string, nominalTime: string | undefined, now: Date): string {
  const parsed = nominalTime === undefined ? Number.NaN : Date.parse(nominalTime);
  return `${prefix}-${toTheSecond(Number.isNaN(parsed) ? now : new Date(parsed))}`;
}

/** What the form does not show, carried from the schedule being edited so that saving does not reset it. */
export interface Carry {
  catchupWindowMs?: number;
  bufferLimit?: number;
  dst?: Record<string, unknown>;
  offsetMs?: number;
  versionPolicy?: 'ACTIVE' | 'PINNED';
  versionId?: string;
  nominalTimeField?: string;
  runTimeoutMs?: number;
}

export interface Draft {
  name: string;
  workflowId: string;
  /** A JSON object. Never returned by the platform, so never pre-filled. */
  input: string;
  when: When;
  every: string;
  unit: Unit;
  /** One cron expression per line. */
  cron: string;
  timezone: string;
  jitterMs: string;
  startAt: string;
  endAt: string;
  overlap: Overlap;
  catchupMode: 'ONE' | 'ALL';
  pauseOnFailure: boolean;
  remainingActions: string;
  paused: boolean;
  note: string;
  /** Empty for none: then neither policy is written. */
  businessIdPrefix: string;
  businessIdConflictPolicy: BusinessConflict;
  businessIdReusePolicy: BusinessReuse;
  carry: Carry;
}

export const EMPTY_DRAFT: Draft = {
  name: '',
  workflowId: '',
  input: '{}',
  when: 'INTERVAL',
  every: '1',
  unit: 'HOURS',
  cron: '',
  timezone: 'UTC',
  jitterMs: '0',
  startAt: '',
  endAt: '',
  overlap: 'SKIP',
  catchupMode: 'ONE',
  pauseOnFailure: false,
  remainingActions: '',
  paused: false,
  note: '',
  businessIdPrefix: '',
  businessIdConflictPolicy: DEFAULT_CONFLICT,
  businessIdReusePolicy: DEFAULT_REUSE,
  carry: {},
};

function whole(text: string): number | undefined {
  const trimmed = text.trim();
  return /^\d+$/.test(trimmed) && Number.isSafeInteger(Number(trimmed)) ? Number(trimmed) : undefined;
}

function instant(text: string): string | undefined {
  const trimmed = text.trim();
  if (trimmed === '') return undefined;
  const at = new Date(trimmed);
  return Number.isNaN(at.getTime()) ? undefined : at.toISOString();
}

/** The request a draft makes, or the first thing wrong with it (the platform checks every rule again). */
export function requestOf(draft: Draft): { body: ScheduleWrite } | { problem: string } {
  const name = draft.name.trim();
  if (name === '' || [...name].length > 128) return { problem: 'Give the schedule a name of 1 to 128 characters.' };
  if (!UUID.test(draft.workflowId)) return { problem: 'Choose the workflow the schedule starts.' };
  let input: unknown;
  try {
    // Not JSON.parse: a number too large to be exact would be rounded before it was sent.
    input = parseAuthoringJson(draft.input);
  } catch (cause) {
    if (cause instanceof UnsafeJsonNumber) return { problem: cause.message };
    return { problem: 'The input must be valid JSON.' };
  }
  if (input === null || typeof input !== 'object' || Array.isArray(input)) return { problem: 'The input must be a JSON object.' };
  if (new TextEncoder().encode(JSON.stringify(input)).length > MAX_INPUT_BYTES) return { problem: 'The input must be at most 64 KiB.' };

  const spec: Record<string, unknown> = {};
  if (draft.when === 'INTERVAL') {
    const every = whole(draft.every);
    if (every === undefined || every < 1) return { problem: 'Give a whole number of seconds, minutes, hours or days, at least 1.' };
    const everyMs = every * UNITS[draft.unit];
    if (!Number.isSafeInteger(everyMs)) return { problem: 'That interval is too long.' };
    const offsetMs = draft.carry.offsetMs;
    spec['intervals'] = [offsetMs === undefined ? { everyMs } : { everyMs, offsetMs }];
  } else {
    const expressions = draft.cron
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => line !== '');
    if (expressions.length === 0 || expressions.length > 10) return { problem: 'Give one to ten cron expressions, one to a line.' };
    spec['cron'] = expressions;
  }
  const timezone = draft.timezone.trim();
  if (timezone === '') return { problem: 'Give a time zone, such as UTC or Europe/Lisbon.' };
  spec['timezone'] = timezone;
  const jitter = whole(draft.jitterMs);
  if (jitter === undefined) return { problem: 'Give the jitter in whole milliseconds (0 for none).' };
  spec['jitterMs'] = jitter;
  const startAt = instant(draft.startAt);
  const endAt = instant(draft.endAt);
  if ((draft.startAt.trim() !== '' && startAt === undefined) || (draft.endAt.trim() !== '' && endAt === undefined)) {
    return { problem: 'Write the start and the end as dates, such as 2026-12-01T09:00:00Z.' };
  }
  if (startAt !== undefined && endAt !== undefined && Date.parse(endAt) <= Date.parse(startAt)) return { problem: 'The end must be after the start.' };
  if (startAt !== undefined) spec['startAt'] = startAt;
  if (endAt !== undefined) spec['endAt'] = endAt;
  if (draft.carry.dst !== undefined) spec['dst'] = draft.carry.dst;

  const remaining = draft.remainingActions.trim() === '' ? undefined : whole(draft.remainingActions);
  if (draft.remainingActions.trim() !== '' && (remaining === undefined || remaining < 1)) return { problem: 'Give the number of actions that may start a run as a whole number, at least 1, or leave it empty.' };
  if ([...draft.note].length > MAX_NOTE) return { problem: `Keep the note to ${MAX_NOTE} characters.` };
  const wrongPrefix = businessPrefixProblem(draft.businessIdPrefix);
  if (wrongPrefix) return { problem: `The business id prefix: ${wrongPrefix}` };
  const prefix = draft.businessIdPrefix.trim();

  const body: ScheduleWrite = {
    name,
    spec,
    policies: {
      overlap: draft.overlap,
      catchupMode: draft.catchupMode,
      pauseOnFailure: draft.pauseOnFailure,
      ...(draft.carry.catchupWindowMs === undefined ? {} : { catchupWindowMs: draft.carry.catchupWindowMs }),
      ...(draft.carry.bufferLimit === undefined ? {} : { bufferLimit: draft.carry.bufferLimit }),
    },
    action: {
      workflowId: draft.workflowId,
      versionPolicy: draft.carry.versionPolicy ?? 'ACTIVE',
      ...(draft.carry.versionId === undefined ? {} : { versionId: draft.carry.versionId }),
      input: input as Record<string, unknown>,
      ...(draft.carry.nominalTimeField === undefined ? {} : { nominalTimeField: draft.carry.nominalTimeField }),
      ...(draft.carry.runTimeoutMs === undefined ? {} : { runTimeoutMs: draft.carry.runTimeoutMs }),
      // all three or none: the policies mean nothing without a prefix
      ...(prefix === '' ? {} : { businessIdPrefix: prefix, businessIdConflictPolicy: draft.businessIdConflictPolicy, businessIdReusePolicy: draft.businessIdReusePolicy }),
    },
    paused: draft.paused,
  };
  if (draft.note.trim() !== '') body.note = draft.note.trim();
  if (remaining !== undefined) body.remainingActions = remaining;
  return { body };
}

/**
 * The draft that edits `schedule`, or why the form cannot: it writes one interval or some cron
 * expressions, so a schedule with calendars, exclusions or several intervals is edited elsewhere.
 * The input is never returned by the platform, so the person writes it again; what the form does
 * not show is carried, so that saving does not reset it.
 */
export function draftOf(schedule: Schedule): { draft: Draft } | { cannot: string } {
  const spec = (schedule.spec ?? {}) as Record<string, unknown>;
  const intervals = Array.isArray(spec['intervals']) ? (spec['intervals'] as { everyMs?: number; offsetMs?: number }[]) : [];
  const cron = Array.isArray(spec['cron']) ? (spec['cron'] as string[]) : [];
  const calendars = Array.isArray(spec['calendars']) ? spec['calendars'] : [];
  const exclusions = Array.isArray(spec['exclusions']) ? spec['exclusions'] : [];
  const oneInterval = intervals.length === 1 && cron.length === 0;
  const someCron = intervals.length === 0 && cron.length >= 1;
  if (calendars.length > 0 || exclusions.length > 0 || !(oneInterval || someCron)) {
    return { cannot: 'This schedule uses calendars, exclusions or several interval rules, which this form cannot write. Change it through the API.' };
  }
  const policies = schedule.policies;
  const action = schedule.action;
  const carry: Carry = {};
  if (policies?.catchupWindowMs !== undefined) carry.catchupWindowMs = policies.catchupWindowMs;
  if (policies?.bufferLimit !== undefined) carry.bufferLimit = policies.bufferLimit;
  if (spec['dst'] !== undefined && spec['dst'] !== null && typeof spec['dst'] === 'object') carry.dst = spec['dst'] as Record<string, unknown>;
  if (action?.versionPolicy !== undefined) carry.versionPolicy = action.versionPolicy;
  if (action?.versionId !== undefined) carry.versionId = action.versionId;
  if (action?.nominalTimeField !== undefined) carry.nominalTimeField = action.nominalTimeField;
  if (action?.runTimeoutMs !== undefined) carry.runTimeoutMs = action.runTimeoutMs;
  const draft: Draft = {
    ...EMPTY_DRAFT,
    name: schedule.name,
    workflowId: action?.workflowId ?? '',
    timezone: typeof spec['timezone'] === 'string' ? spec['timezone'] : 'UTC',
    jitterMs: String(typeof spec['jitterMs'] === 'number' ? spec['jitterMs'] : 0),
    startAt: typeof spec['startAt'] === 'string' ? spec['startAt'] : '',
    endAt: typeof spec['endAt'] === 'string' ? spec['endAt'] : '',
    overlap: policies?.overlap ?? 'SKIP',
    catchupMode: policies?.catchupMode ?? 'ONE',
    pauseOnFailure: policies?.pauseOnFailure ?? false,
    remainingActions: schedule.remainingActions === null || schedule.remainingActions === undefined ? '' : String(schedule.remainingActions),
    paused: schedule.state === 'PAUSED',
    note: schedule.note ?? '',
    businessIdPrefix: action?.businessIdPrefix ?? '',
    businessIdConflictPolicy: action?.businessIdConflictPolicy ?? DEFAULT_CONFLICT,
    businessIdReusePolicy: action?.businessIdReusePolicy ?? DEFAULT_REUSE,
    carry,
  };
  if (oneInterval) {
    const rule = intervals[0] ?? {};
    const ms = rule.everyMs ?? 0;
    const unit = unitOf(ms);
    if (rule.offsetMs !== undefined && rule.offsetMs !== 0) carry.offsetMs = rule.offsetMs;
    return { draft: { ...draft, when: 'INTERVAL', every: String(ms / UNITS[unit]), unit } };
  }
  return { draft: { ...draft, when: 'CRON', cron: cron.join('\n') } };
}

/** The largest unit that divides `ms` exactly. */
function unitOf(ms: number): Unit {
  return (['DAYS', 'HOURS', 'MINUTES', 'SECONDS'] as const).find((unit) => ms > 0 && ms % UNITS[unit] === 0) ?? 'SECONDS';
}

/** The spec in words, for a list and a heading. */
export function describeSpec(schedule: Pick<Schedule, 'spec'>): string {
  const spec = (schedule.spec ?? {}) as Record<string, unknown>;
  const parts: string[] = [];
  for (const rule of Array.isArray(spec['intervals']) ? (spec['intervals'] as { everyMs?: number }[]) : []) {
    const ms = rule.everyMs ?? 0;
    const unit = unitOf(ms);
    const count = ms / UNITS[unit];
    parts.push(`every ${count === 1 ? UNIT_LABEL[unit].replace(/s$/, '') : `${count} ${UNIT_LABEL[unit]}`}`);
  }
  for (const expression of Array.isArray(spec['cron']) ? (spec['cron'] as string[]) : []) parts.push(`cron ${expression}`);
  const calendars = Array.isArray(spec['calendars']) ? spec['calendars'].length : 0;
  if (calendars > 0) parts.push(calendars === 1 ? 'a calendar' : `${calendars} calendars`);
  if (parts.length === 0) return 'no time';
  const zone = typeof spec['timezone'] === 'string' ? spec['timezone'] : 'UTC';
  return `${parts.join(', ')} (${zone})`;
}

// --- states and outcomes ----------------------------------------------------------------------

export function scheduleState(schedule: Pick<Schedule, 'state' | 'exhausted'>): StatusView {
  if (schedule.exhausted) return { label: 'Exhausted', symbol: '◼', tone: 'skip', help: 'Every action it was allowed has been taken.' };
  if (schedule.state === 'PAUSED') return { label: 'Paused', symbol: '⏸', tone: 'waiting', help: 'No time is acted on while it is paused, and none is caught up afterwards.' };
  return { label: 'Active', symbol: '▶', tone: 'running', help: 'It starts a run at each time of its spec.' };
}

const OUTCOME: Record<string, StatusView> = {
  STARTED: { label: 'Started a run', symbol: '✓', tone: 'ok', help: 'A run was started for this time.' },
  BUFFERED: { label: 'Waiting for a run to end', symbol: '⏸', tone: 'waiting', help: 'The time waits for the open run to end.' },
  SKIPPED_OVERLAP: { label: 'Skipped: a run was open', symbol: '⤼', tone: 'skip', help: 'The overlap policy skipped this time.' },
  DROPPED_BUFFER_FULL: { label: 'Dropped: too many waiting', symbol: '⤼', tone: 'skip', help: 'The waiting times were at their limit.' },
  DROPPED_DELETED: { label: 'Dropped: schedule deleted', symbol: '⊘', tone: 'skip', help: 'The schedule was deleted while this time waited.' },
  COALESCED: { label: 'Coalesced', symbol: '⧉', tone: 'skip', help: 'Several late times were merged into one.' },
  MISSED_CATCHUP_WINDOW: { label: 'Missed', symbol: '⌛', tone: 'bad', help: 'The time was too late to catch up.' },
  ALREADY_ACTED: { label: 'Already acted on', symbol: '≡', tone: 'idle', help: 'The time was already acted on: a backfill found it done, or a run already held its business id. Nothing was started.' },
  AUTHORITY_REVOKED: { label: 'Not allowed', symbol: '✕', tone: 'bad', help: 'The person who authorized the schedule may no longer start the run.' },
  BUSINESS_ID_CONFLICT: { label: 'Business id conflict', symbol: '✕', tone: 'bad', help: 'The business id of the run was held by an open run, or remembered by a closed one, and the policy refused it. Nothing was started.' },
};

/** An outcome in words; one this version does not know is "another outcome". */
export function outcomeView(outcome: string): StatusView {
  return OUTCOME[outcome] ?? { label: 'Another outcome', symbol: '?', tone: 'idle', help: outcome };
}

export const OUTCOMES = Object.keys(OUTCOME);

const REASON: Record<string, string> = {
  TRACKING_LIMIT: 'The schedule already tracks as many open runs as it may.',
  NO_PUBLISHED_VERSION: 'The workflow has no published version to start.',
  AUTHORITY_REVOKED: 'The person who authorized the schedule lost a permission it needs.',
  OVERLAP: 'A run of the schedule was open.',
  BUFFER_FULL: 'The waiting times were at their limit.',
  BUSINESS_ID_IN_USE: 'The business id was held by an open run.',
  BUSINESS_ID_REUSE_REJECTED: 'A closed run still remembers the id and the reuse policy refuses it.',
};

export function reasonText(code: string | null | undefined): string {
  if (code === null || code === undefined || code === '') return '';
  return REASON[code] ?? code;
}

// --- a backfill ----------------------------------------------------------------------------------

/** A backfill over `[start, end)`: the end is not after now, and it is after the start. */
export function backfillOf(start: string, end: string, overlap: Overlap | '', now = new Date()): { body: { start: string; end: string; overlap?: Overlap } } | { problem: string } {
  const from = instant(start);
  const to = instant(end);
  if (from === undefined || to === undefined) return { problem: 'Write the start and the end as dates, such as 2026-12-01T09:00:00Z.' };
  if (Date.parse(to) <= Date.parse(from)) return { problem: 'The end must be after the start.' };
  if (Date.parse(to) > now.getTime()) return { problem: 'The end cannot be in the future: a backfill covers times that have passed.' };
  return { body: { start: from, end: to, ...(overlap === '' ? {} : { overlap }) } };
}

/** The reason of a pause or a resume, trimmed: 1 to 512 characters. */
export function noteProblem(note: string): string | undefined {
  const length = [...note.trim()].length;
  if (length < 1) return 'Say why, in a few words.';
  if (length > MAX_NOTE) return `Keep the note to ${MAX_NOTE} characters.`;
  return undefined;
}
