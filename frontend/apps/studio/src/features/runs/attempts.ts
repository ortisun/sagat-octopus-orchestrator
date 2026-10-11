import { parseTime } from './time';

/** What the attempts of a step are told from: a journal event, or the light copy of it a live update carries. */
export interface HistoryEvent {
  seq: number;
  eventType: string;
  occurredAt: string;
  stepId?: string | null | undefined;
  attempt?: number | null | undefined;
  actor?: string | undefined;
}

export type AttemptOutcome =
  /** Leased or started, and nothing has ended it yet. */
  | 'running'
  | 'succeeded'
  /** The attempt failed and the platform scheduled another. */
  | 'retry-scheduled'
  /** The attempt failed and nothing is left to try. */
  | 'failed'
  | 'cancelled'
  /** The attempt ended in a way a person has to decide about. */
  | 'needs-review';

export interface Attempt {
  number: number;
  leasedAt?: string;
  startedAt?: string;
  endedAt?: string;
  /** From the start of the work (or the lease, when the worker never said it started) to the end. */
  durationMs?: number;
  outcome: AttemptOutcome;
  firstSeq: number;
  lastSeq: number;
}

/** Something that happened to a step that is not an attempt: a decision of an operator, a request to stop. */
export interface Mark {
  seq: number;
  eventType: string;
  at: string;
  actor?: string;
  /** The attempt it concerns, when the event says. */
  attempt?: number;
}

export interface StepHistory {
  stepId: string;
  attempts: Attempt[];
  marks: Mark[];
}

const MARKS = new Set(['step.retry_requested', 'step.skipped', 'step.resolved', 'step.cancel_requested', 'step.needs_review']);

/** Events in sequence order without repeats (a live update and the journal may both carry one). */
export function ordered<T extends { seq: number }>(events: readonly T[]): T[] {
  const sorted = [...events].sort((a, b) => a.seq - b.seq);
  return sorted.filter((event, i) => i === 0 || event.seq !== (sorted[i - 1] as T).seq);
}

function span(from: string | undefined, to: string): number | undefined {
  const start = parseTime(from);
  const end = parseTime(to);
  return start === undefined || end === undefined ? undefined : Math.max(0, end - start);
}

/**
 * The attempts of one step, told from the journal: when each was leased, started and ended, how
 * long it took and how it ended, plus what operators decided about the step. Nothing here is read
 * from a payload (there is none in the journal, G144).
 */
export function stepHistory(events: readonly HistoryEvent[], stepId: string): StepHistory {
  return buildHistory(
    stepId,
    ordered(events.filter((event) => event.stepId === stepId)),
  );
}

/** The history of every step the events name. */
export function allHistories(events: readonly HistoryEvent[]): Map<string, StepHistory> {
  const byStep = new Map<string, HistoryEvent[]>();
  for (const event of events) {
    if (event.stepId === null || event.stepId === undefined || event.stepId === '') continue;
    const list = byStep.get(event.stepId) ?? [];
    list.push(event);
    byStep.set(event.stepId, list);
  }
  return new Map([...byStep].map(([id, list]) => [id, buildHistory(id, ordered(list))]));
}

function buildHistory(stepId: string, events: readonly HistoryEvent[]): StepHistory {
  const attempts = new Map<number, Attempt>();
  const marks: Mark[] = [];
  let latest: Attempt | undefined;

  // `fresh`: the event starts an attempt (a lease), so one it does not number is a new one, not the last.
  const touch = (event: HistoryEvent, create: boolean, fresh = false): Attempt | undefined => {
    const wanted = typeof event.attempt === 'number' && event.attempt > 0 ? event.attempt : undefined;
    let attempt = wanted === undefined ? (fresh ? undefined : latest) : attempts.get(wanted);
    if (attempt === undefined && (create || wanted !== undefined)) {
      attempt = { number: wanted ?? (latest?.number ?? 0) + 1, outcome: 'running', firstSeq: event.seq, lastSeq: event.seq };
      attempts.set(attempt.number, attempt);
    }
    if (attempt !== undefined) {
      attempt.lastSeq = event.seq;
      if (latest === undefined || attempt.number >= latest.number) latest = attempt;
    }
    return attempt;
  };

  const end = (attempt: Attempt, event: HistoryEvent, outcome: AttemptOutcome) => {
    // The first thing that ends an attempt is what it ended with.
    if (attempt.endedAt !== undefined) return;
    attempt.endedAt = event.occurredAt;
    attempt.outcome = outcome;
    const duration = span(attempt.startedAt ?? attempt.leasedAt, event.occurredAt);
    if (duration !== undefined) attempt.durationMs = duration;
  };

  const mark = (event: HistoryEvent) => {
    const out: Mark = { seq: event.seq, eventType: event.eventType, at: event.occurredAt };
    if (event.actor !== undefined) out.actor = event.actor;
    if (typeof event.attempt === 'number' && event.attempt > 0) out.attempt = event.attempt;
    marks.push(out);
  };

  for (const event of events) {
    switch (event.eventType) {
      case 'step.leased': {
        const attempt = touch(event, true, true);
        if (attempt !== undefined) attempt.leasedAt ??= event.occurredAt;
        break;
      }
      case 'step.started': {
        const attempt = touch(event, true);
        if (attempt !== undefined) attempt.startedAt ??= event.occurredAt;
        break;
      }
      case 'step.completed': {
        const attempt = touch(event, true);
        if (attempt !== undefined) end(attempt, event, 'succeeded');
        break;
      }
      case 'step.failed': {
        const attempt = touch(event, true);
        if (attempt !== undefined) end(attempt, event, 'failed');
        break;
      }
      case 'step.retry_scheduled': {
        const attempt = touch(event, true);
        if (attempt !== undefined) end(attempt, event, 'retry-scheduled');
        break;
      }
      case 'step.cancelled': {
        // A step cancelled before any worker took it has no attempt to end: it is a note.
        const attempt = touch(event, false);
        if (attempt !== undefined && attempt.endedAt === undefined) end(attempt, event, 'cancelled');
        else mark(event);
        break;
      }
      case 'step.needs_review': {
        const attempt = touch(event, false);
        if (attempt !== undefined && attempt.endedAt === undefined) end(attempt, event, 'needs-review');
        mark(event);
        break;
      }
      default:
        if (MARKS.has(event.eventType)) mark(event);
    }
  }

  return { stepId, attempts: [...attempts.values()].sort((a, b) => a.number - b.number), marks };
}

/** How long an attempt that has not ended has been going, at `nowMs`. */
export function openDuration(attempt: Attempt, nowMs: number): number | undefined {
  if (attempt.endedAt !== undefined) return attempt.durationMs;
  const from = parseTime(attempt.startedAt ?? attempt.leasedAt);
  return from === undefined ? undefined : Math.max(0, nowMs - from);
}

const OUTCOME_WORDS: Record<AttemptOutcome, string> = {
  running: 'In progress',
  succeeded: 'Succeeded',
  'retry-scheduled': 'Failed, retry scheduled',
  failed: 'Failed',
  cancelled: 'Cancelled',
  'needs-review': 'Needs a decision',
};

export function outcomeWords(outcome: AttemptOutcome): string {
  return OUTCOME_WORDS[outcome];
}
