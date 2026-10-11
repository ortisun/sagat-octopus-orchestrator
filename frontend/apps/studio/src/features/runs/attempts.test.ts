import { describe, expect, it } from 'vitest';
import { allHistories, openDuration, ordered, outcomeWords, stepHistory } from './attempts';
import type { HistoryEvent } from './attempts';

let seq = 0;
/** An event `s` seconds after 10:00:00 on the day of the fixture. */
function ev(type: string, step: string | null, s: number, attempt?: number, actor?: string): HistoryEvent {
  seq += 1;
  const at = new Date(Date.UTC(2026, 9, 10, 10, 0, s)).toISOString();
  return { seq, eventType: type, occurredAt: at, stepId: step, attempt: attempt ?? null, ...(actor ? { actor } : {}) };
}

describe('stepHistory', () => {
  it('tells one attempt that worked: when it started, ended, how long it took', () => {
    seq = 0;
    const history = stepHistory([ev('step.leased', 'pay', 0, 1), ev('step.started', 'pay', 1, 1), ev('step.completed', 'pay', 4, 1)], 'pay');
    expect(history.attempts).toHaveLength(1);
    const [a] = history.attempts;
    expect(a).toMatchObject({ number: 1, outcome: 'succeeded', durationMs: 3000 });
    expect(a?.leasedAt).toBe('2026-10-10T10:00:00.000Z');
    expect(a?.startedAt).toBe('2026-10-10T10:00:01.000Z');
    expect(a?.endedAt).toBe('2026-10-10T10:00:04.000Z');
  });

  it('tells retries: each attempt has its own outcome and duration, the last one worked', () => {
    seq = 0;
    const history = stepHistory(
      [
        ev('step.leased', 'pay', 0, 1),
        ev('step.started', 'pay', 1, 1),
        ev('step.retry_scheduled', 'pay', 3, 1),
        ev('step.retry_due', 'pay', 10, 1),
        ev('step.leased', 'pay', 10, 2),
        ev('step.started', 'pay', 11, 2),
        ev('step.retry_scheduled', 'pay', 12, 2),
        ev('step.leased', 'pay', 30, 3),
        ev('step.started', 'pay', 30, 3),
        ev('step.completed', 'pay', 31, 3),
      ],
      'pay',
    );
    expect(history.attempts.map((a) => [a.number, a.outcome, a.durationMs])).toEqual([
      [1, 'retry-scheduled', 2000],
      [2, 'retry-scheduled', 1000],
      [3, 'succeeded', 1000],
    ]);
  });

  it('tells a step that failed for good', () => {
    seq = 0;
    const history = stepHistory([ev('step.leased', 'pay', 0, 1), ev('step.started', 'pay', 0, 1), ev('step.failed', 'pay', 9, 1)], 'pay');
    expect(history.attempts[0]).toMatchObject({ outcome: 'failed', durationMs: 9000 });
  });

  it('measures from the lease when the worker never said it started (a lost lease)', () => {
    seq = 0;
    const history = stepHistory([ev('step.leased', 'pay', 0, 1), ev('step.retry_scheduled', 'pay', 30, 1)], 'pay');
    expect(history.attempts[0]).toMatchObject({ outcome: 'retry-scheduled', durationMs: 30_000 });
    expect(history.attempts[0]?.startedAt).toBeUndefined();
  });

  it('leaves an attempt that has not ended running, with no duration, until you ask at a given time', () => {
    seq = 0;
    const history = stepHistory([ev('step.leased', 'pay', 0, 1), ev('step.started', 'pay', 2, 1)], 'pay');
    const [a] = history.attempts;
    expect(a).toMatchObject({ outcome: 'running' });
    expect(a?.durationMs).toBeUndefined();
    expect(openDuration(a!, Date.UTC(2026, 9, 10, 10, 0, 12))).toBe(10_000);
  });

  it('records the decisions of operators next to the attempts, and the attempt they started', () => {
    seq = 0;
    const history = stepHistory(
      [
        ev('step.leased', 'pay', 0, 1),
        ev('step.started', 'pay', 0, 1),
        ev('step.failed', 'pay', 5, 1),
        ev('step.needs_review', 'pay', 5, 1),
        ev('step.retry_requested', 'pay', 60, undefined, 'user:alice'),
        ev('step.leased', 'pay', 61, 2),
        ev('step.started', 'pay', 61, 2),
        ev('step.completed', 'pay', 62, 2),
      ],
      'pay',
    );
    expect(history.attempts.map((a) => [a.number, a.outcome])).toEqual([
      [1, 'failed'],
      [2, 'succeeded'],
    ]);
    expect(history.marks.map((m) => [m.eventType, m.actor])).toEqual([
      ['step.needs_review', undefined],
      ['step.retry_requested', 'user:alice'],
    ]);
  });

  it('a step cancelled while an attempt runs ends that attempt as cancelled', () => {
    seq = 0;
    const history = stepHistory([ev('step.leased', 'wait', 0, 1), ev('step.started', 'wait', 0, 1), ev('step.cancel_requested', 'wait', 5, 1), ev('step.cancelled', 'wait', 6, 1)], 'wait');
    expect(history.attempts[0]).toMatchObject({ outcome: 'cancelled', durationMs: 6000 });
    expect(history.marks.map((m) => m.eventType)).toEqual(['step.cancel_requested']);
  });

  it('a step cancelled before any worker took it has no attempt, only a note', () => {
    seq = 0;
    const history = stepHistory([ev('step.cancelled', 'later', 6)], 'later');
    expect(history.attempts).toEqual([]);
    expect(history.marks.map((m) => m.eventType)).toEqual(['step.cancelled']);
  });

  it('keeps a compensation apart from the step it undoes', () => {
    seq = 0;
    const events = [
      ev('step.leased', 'reserve', 0, 1),
      ev('step.completed', 'reserve', 2, 1),
      ev('step.leased', 'reserve~c', 20, 1),
      ev('step.started', 'reserve~c', 20, 1),
      ev('step.retry_scheduled', 'reserve~c', 22, 1),
      ev('step.leased', 'reserve~c', 30, 2),
      ev('step.completed', 'reserve~c', 33, 2),
    ];
    expect(stepHistory(events, 'reserve').attempts).toHaveLength(1);
    const undo = stepHistory(events, 'reserve~c');
    expect(undo.attempts.map((a) => [a.number, a.outcome])).toEqual([
      [1, 'retry-scheduled'],
      [2, 'succeeded'],
    ]);
  });

  it('numbers an attempt the event does not number: a lease is always a new attempt', () => {
    seq = 0;
    const history = stepHistory([ev('step.leased', 'pay', 0), ev('step.retry_scheduled', 'pay', 3), ev('step.leased', 'pay', 9), ev('step.completed', 'pay', 10)], 'pay');
    expect(history.attempts.map((a) => [a.number, a.outcome])).toEqual([
      [1, 'retry-scheduled'],
      [2, 'succeeded'],
    ]);
  });

  it('keeps how an attempt ended: a later end event for the same attempt does not rewrite it', () => {
    seq = 0;
    const history = stepHistory([ev('step.leased', 'pay', 0, 1), ev('step.completed', 'pay', 3, 1), ev('step.failed', 'pay', 9, 1), ev('step.cancelled', 'pay', 12, 1)], 'pay');
    expect(history.attempts).toHaveLength(1);
    expect(history.attempts[0]).toMatchObject({ outcome: 'succeeded', durationMs: 3000, endedAt: '2026-10-10T10:00:03.000Z' });
  });

  it('counts an event once however many times it arrives, and in sequence order however it arrives', () => {
    seq = 0;
    const a = ev('step.leased', 'pay', 0, 1);
    const b = ev('step.started', 'pay', 1, 1);
    const c = ev('step.completed', 'pay', 3, 1);
    const history = stepHistory([c, a, b, b, a], 'pay');
    expect(history.attempts).toHaveLength(1);
    expect(history.attempts[0]).toMatchObject({ outcome: 'succeeded', durationMs: 2000, firstSeq: a.seq, lastSeq: c.seq });
    expect(ordered([c, a, b, b]).map((e) => e.seq)).toEqual([a.seq, b.seq, c.seq]);
  });

  it('does not invent a duration from a time it cannot read', () => {
    seq = 0;
    const bad: HistoryEvent = { seq: 1, eventType: 'step.leased', occurredAt: 'not a time', stepId: 'pay', attempt: 1 };
    const done: HistoryEvent = { seq: 2, eventType: 'step.completed', occurredAt: '2026-10-10T10:00:00Z', stepId: 'pay', attempt: 1 };
    const [a] = stepHistory([bad, done], 'pay').attempts;
    expect(a?.outcome).toBe('succeeded');
    expect(a?.durationMs).toBeUndefined();
  });

  it('ignores events of other steps, of the run, and kinds that are not about attempts', () => {
    seq = 0;
    const history = stepHistory([ev('run.started', null, 0), ev('step.leased', 'other', 0, 1), ev('step.routed', 'pay', 1), ev('condition.evaluated', 'pay', 1)], 'pay');
    expect(history.attempts).toEqual([]);
    expect(history.marks).toEqual([]);
  });
});

describe('allHistories', () => {
  it('groups the events by step, leaving out those that name none', () => {
    seq = 0;
    const map = allHistories([ev('run.started', null, 0), ev('step.leased', 'a', 0, 1), ev('step.completed', 'a', 1, 1), ev('step.leased', 'b', 1, 1), ev('step.leased', 'b~c', 2, 1)]);
    expect([...map.keys()].sort()).toEqual(['a', 'b', 'b~c']);
    expect(map.get('a')?.attempts[0]?.outcome).toBe('succeeded');
    expect(map.get('b')?.attempts[0]?.outcome).toBe('running');
  });
});

describe('outcomeWords', () => {
  it('has words for every outcome', () => {
    for (const outcome of ['running', 'succeeded', 'retry-scheduled', 'failed', 'cancelled', 'needs-review'] as const) {
      expect(outcomeWords(outcome)).not.toBe('');
    }
  });
});
