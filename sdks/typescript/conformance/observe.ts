// What the conformance runner prints for a schedule (tests/contract/sdk-conformance/README.md). It lives apart from
// the runner, which starts a scenario as soon as it is loaded, so that a unit test can feed it a typed answer.
import type { Schedule, ScheduleActionRecord, ScheduleDescription } from '../src/index.js';

type Obj = Record<string, any>;

export const iso = (date: Date | null): string | null => (date ? date.toISOString() : null);

// The feature 018 results. Instants that depend on when the scenario ran are written as instants (the harness
// compares them as `<time>`); the ones a fixed spec determines are written as epoch milliseconds, so that a wrong
// parse of an instant shows.
export const epoch = (date: Date | null): number | null => (date ? date.getTime() : null);

/**
 * The action of a schedule as the server keeps it. The business-id keys are printed only when the server sent them
 * (absent is absent, never null), so the observation of a schedule without a prefix is what it always was.
 */
export function scheduleActionJson(action: Schedule['action']): Obj {
  const { businessIdPrefix, businessIdConflictPolicy, businessIdReusePolicy, ...rest } = action;
  return {
    ...rest,
    ...(businessIdPrefix !== null ? { businessIdPrefix } : {}),
    ...(businessIdConflictPolicy !== null ? { businessIdConflictPolicy } : {}),
    ...(businessIdReusePolicy !== null ? { businessIdReusePolicy } : {}),
  };
}

export function scheduleJson(s: Schedule): Obj {
  return {
    scheduleId: s.scheduleId, name: s.name, origin: s.origin, state: s.state, revision: s.revision, exhausted: s.exhausted,
    action: scheduleActionJson(s.action), note: s.note, noteBy: s.noteBy, remainingActions: s.remainingActions, nextActionEpochMs: epoch(s.nextActionAt),
    createdAt: iso(s.createdAt), updatedAt: iso(s.updatedAt),
  };
}

export function actionJson(a: ScheduleActionRecord): Obj {
  return {
    kind: a.kind, origin: a.origin, revision: a.revision, outcome: a.outcome, reason: a.reason, runId: a.runId, backfillId: a.backfillId,
    nominalTime: iso(a.nominalTime), actualTime: iso(a.actualTime), summaryCount: a.summaryCount,
    summaryLastNominalTime: iso(a.summaryLastNominalTime), decidedAt: iso(a.decidedAt),
  };
}

export function descriptionJson(d: ScheduleDescription): Obj {
  return {
    ...scheduleJson(d),
    nextActions: d.nextActions.map((n) => ({ nominalTimeEpochMs: n.nominalTime.getTime(), actualTimeEpochMs: n.actualTime.getTime() })),
    recentActions: d.recentActions.map(actionJson), openRuns: d.openRuns, bufferedCount: d.bufferedCount, counters: d.counters,
  };
}
