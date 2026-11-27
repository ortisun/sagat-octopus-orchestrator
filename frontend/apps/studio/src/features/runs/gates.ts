import type { Run } from '../../api/types';
import { clip, parseTime } from './time';

/**
 * The run-level gates of feature 022 in words: the concurrency permit a run holds or waits for (`Run.concurrency`,
 * FR-022-022) and the rate-limit slot it was granted or waits for (`Run.rateLimit`, FR-022-048). Only what the platform
 * sent is shown: an absent position or estimate is said absent, never computed here, and a value this Studio does not
 * know is shown as unsupported (FR-007-010, decision D022-150 items 4 to 6).
 */

type RunConcurrency = NonNullable<Run['concurrency']>;
type RunRateLimit = NonNullable<Run['rateLimit']>;

export interface GateView {
  heading: string;
  state: string;
  waiting: boolean;
  sinceLabel: string;
  since: string | undefined;
  /** When a waiting run fails, and the code it fails with. */
  deadline: string | null | undefined;
  deadlineCode: string;
  group: string;
  pool: string;
  key: string;
  limit?: string;
  position?: string;
  /** The platform's estimate; `null` when it gave none. */
  estimate?: string | null;
  /**
   * No estimate because the caller may not see the key's label: the platform withholds it from such a caller, which is
   * not the same as an estimate it could not compute (security review of 2026-11-24, nit 2).
   */
  estimateWithheld?: boolean;
  /** A granted slot counts against the limit until then (its grant plus the window). */
  countsUntil?: string;
  /** The values of a newer platform this Studio does not know, in words. */
  unsupported: string[];
}

export const HIDDEN_KEY = 'Hidden: you may not reveal personal data in this namespace, so the platform does not show the key label.';

const POOLS: Record<string, string> = { ORDINARY: 'Ordinary runs', DRAFT_TEST: 'Draft test runs' };
const WINDOW_MS: Record<string, number> = { second: 1_000, minute: 60_000, hour: 3_600_000 };
const WINDOW_WORDS: Record<string, string> = { second: 'a second', minute: 'a minute', hour: 'an hour' };

const MAX_VALUE = 40;

function isObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function text(value: unknown): string {
  if (typeof value === 'string') return clip(value, MAX_VALUE);
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  return clip(JSON.stringify(value) ?? '', MAX_VALUE);
}

/** `=<workflow>` and `=<workflow>/<node>` are the groups the platform derives; an explicit group is a name. */
export function groupWords(group: unknown): string {
  if (typeof group !== 'string') return 'unknown';
  const shown = clip(group, 140);
  const derived = /^=([^/]+)(?:\/(.+))?$/.exec(group);
  if (!derived) return shown;
  const [, workflow, step] = derived;
  return step === undefined ? `${shown} (the workflow ${clip(workflow ?? '', 64)} alone)` : `${shown} (the step ${clip(step, 64)} of ${clip(workflow ?? '', 64)} alone)`;
}

function keyWords(label: unknown, unsupported: string[]): string {
  if (label === null) return HIDDEN_KEY;
  if (typeof label === 'string') return clip(label, MAX_VALUE);
  unsupported.push('key label');
  return 'unknown';
}

function poolWords(pool: unknown, unsupported: string[]): string {
  if (typeof pool === 'string' && POOLS[pool]) return POOLS[pool];
  unsupported.push(`pool ${text(pool)}`);
  return `A pool this Studio does not know (${text(pool)})`;
}

function unreadable(heading: string, deadlineCode: string): GateView {
  return {
    heading,
    state: 'A gate this Studio cannot read',
    waiting: false,
    sinceLabel: 'Since',
    since: undefined,
    deadline: undefined,
    deadlineCode,
    group: 'unknown',
    pool: 'unknown',
    key: 'unknown',
    unsupported: ['the whole value'],
  };
}

const instant = (value: unknown): string | undefined => (typeof value === 'string' && parseTime(value) !== undefined ? value : undefined);

/** The concurrency permit of a run, or nothing when the run neither holds nor waits for one. */
export function gateWords(gate: RunConcurrency | undefined): GateView | undefined {
  if (gate === undefined) return undefined;
  const heading = 'Concurrency permit';
  const deadlineCode = 'CONCURRENCY_WAIT_TIMEOUT';
  const raw: unknown = gate;
  if (!isObject(raw)) return unreadable(heading, deadlineCode);
  const unsupported: string[] = [];
  const state = raw['state'];
  let words: string;
  let sinceLabel = 'Since';
  if (state === 'HOLDING') {
    words = 'Holds a concurrency permit';
    sinceLabel = 'Holding since';
  } else if (state === 'WAITING') {
    words = 'Waiting for a concurrency permit';
    sinceLabel = 'Waiting since';
  } else {
    words = `In a state this Studio does not know (${text(state)})`;
    unsupported.push(`state ${text(state)}`);
  }
  const pool = poolWords(raw['pool'], unsupported);
  return {
    heading,
    state: words,
    waiting: state === 'WAITING',
    sinceLabel,
    since: instant(raw['since']),
    deadline: instant(raw['queueDeadline']) ?? null,
    deadlineCode,
    group: groupWords(raw['group']),
    pool,
    key: keyWords(raw['keyLabel'], unsupported),
    unsupported,
  };
}

function positionWords(position: unknown): string {
  if (typeof position !== 'number' || !Number.isSafeInteger(position) || position < 1) return 'Not known at this read';
  return position === 1 ? 'Next in the queue' : `${position - 1} ahead of it in the queue`;
}

/** The rate-limit slot of a run, or nothing when the run neither has a slot that still counts nor waits for one. */
export function rateWords(slot: RunRateLimit | undefined): GateView | undefined {
  if (slot === undefined) return undefined;
  const heading = 'Rate-limit slot';
  const deadlineCode = 'RATE_LIMIT_WAIT_TIMEOUT';
  const raw: unknown = slot;
  if (!isObject(raw)) return unreadable(heading, deadlineCode);
  const unsupported: string[] = [];
  const state = raw['state'];
  let words: string;
  let sinceLabel = 'Since';
  if (state === 'GRANTED') {
    words = 'Granted a rate-limit slot';
    sinceLabel = 'Granted at';
  } else if (state === 'WAITING') {
    words = 'Waiting for a rate-limit slot';
    sinceLabel = 'Waiting since';
  } else {
    words = `In a state this Studio does not know (${text(state)})`;
    unsupported.push(`state ${text(state)}`);
  }
  const pool = poolWords(raw['pool'], unsupported);
  const per = raw['per'];
  const limit = typeof raw['limit'] === 'number' ? String(raw['limit']) : 'unknown';
  const known = typeof per === 'string' && WINDOW_WORDS[per] !== undefined;
  if (!known) unsupported.push(`window ${text(per)}`);
  const since = instant(raw['since']);
  const view: GateView = {
    heading,
    state: words,
    waiting: state === 'WAITING',
    sinceLabel,
    since,
    deadline: instant(raw['queueDeadline']) ?? null,
    deadlineCode,
    group: groupWords(raw['group']),
    pool,
    key: keyWords(raw['keyLabel'], unsupported),
    limit: known ? `At most ${limit} ${WINDOW_WORDS[per] ?? ''}` : `At most ${limit} in a window this Studio does not know (${text(per)})`,
    unsupported,
  };
  if (state === 'WAITING') {
    // the platform withholds the place, as the estimate, from a caller who may not see the key's label (decision D022-153)
    view.position = raw['position'] == null && raw['keyLabel'] === null ? 'Withheld: you may not reveal personal data in this namespace' : positionWords(raw['position']);
    view.estimate = instant(raw['estimatedGrantAt']) ?? null;
    view.estimateWithheld = view.estimate === null && raw['keyLabel'] === null;
  }
  if (state === 'GRANTED' && since !== undefined && known) {
    view.countsUntil = new Date((parseTime(since) ?? 0) + (WINDOW_MS[per] ?? 0)).toISOString();
  }
  return view;
}
