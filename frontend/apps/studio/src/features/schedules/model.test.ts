import { describe, expect, it } from 'vitest';
import type { Schedule } from '../../api/endpoints';
import { EMPTY_DRAFT, MAX_NOTE, OUTCOMES, backfillOf, describeSpec, draftOf, noteProblem, outcomeView, reasonText, requestOf, scheduleState } from './model';
import type { Draft } from './model';

const WORKFLOW = '0194f5e2-7b1c-7000-8000-000000000001';
const VALID: Draft = { ...EMPTY_DRAFT, name: 'nightly', workflowId: WORKFLOW };

const problemOf = (result: ReturnType<typeof requestOf>): string | undefined => ('problem' in result ? result.problem : undefined);
const bodyOf = (result: ReturnType<typeof requestOf>) => {
  if (!('body' in result)) throw new Error(`refused: ${result.problem}`);
  return result.body;
};

function schedule(over: Partial<Schedule> = {}): Schedule {
  return {
    scheduleId: '0194f5e2-7b1c-7000-8000-0000000000aa',
    name: 'nightly',
    origin: 'API',
    state: 'ACTIVE',
    revision: 3,
    spec: { intervals: [{ everyMs: 7_200_000, offsetMs: 0 }], timezone: 'UTC', jitterMs: 0 },
    policies: { overlap: 'SKIP', catchupWindowMs: 31_622_400_000, catchupMode: 'ONE', pauseOnFailure: false, bufferLimit: 100 },
    action: { workflowId: WORKFLOW, versionPolicy: 'ACTIVE', inputDigest: 'ab'.repeat(32) },
    createdAt: '2026-11-01T00:00:00Z',
    ...over,
  };
}

describe('the request a form makes', () => {
  it('writes an interval in milliseconds, whatever the unit', () => {
    expect(bodyOf(requestOf({ ...VALID, every: '2', unit: 'HOURS' })).spec).toMatchObject({ intervals: [{ everyMs: 7_200_000 }], timezone: 'UTC', jitterMs: 0 });
    expect(bodyOf(requestOf({ ...VALID, every: '90', unit: 'SECONDS' })).spec).toMatchObject({ intervals: [{ everyMs: 90_000 }] });
    expect(bodyOf(requestOf({ ...VALID, every: '3', unit: 'DAYS' })).spec).toMatchObject({ intervals: [{ everyMs: 259_200_000 }] });
    expect(bodyOf(requestOf({ ...VALID, every: '15', unit: 'MINUTES' })).spec).toMatchObject({ intervals: [{ everyMs: 900_000 }] });
  });

  it('writes cron expressions one to a line, ignoring blank lines', () => {
    const body = bodyOf(requestOf({ ...VALID, when: 'CRON', cron: ' 0 9 * * 1-5 \n\n30 18 * * * \n', timezone: ' Europe/Lisbon ' }));
    expect(body.spec).toEqual({ cron: ['0 9 * * 1-5', '30 18 * * *'], timezone: 'Europe/Lisbon', jitterMs: 0 });
    expect(body.spec).not.toHaveProperty('intervals');
  });

  it('refuses no cron expression and more than ten', () => {
    expect(problemOf(requestOf({ ...VALID, when: 'CRON', cron: '  \n ' }))).toMatch(/one to ten cron/);
    expect(problemOf(requestOf({ ...VALID, when: 'CRON', cron: Array.from({ length: 11 }, () => '* * * * *').join('\n') }))).toMatch(/one to ten cron/);
    expect(problemOf(requestOf({ ...VALID, when: 'CRON', cron: Array.from({ length: 10 }, () => '* * * * *').join('\n') }))).toBeUndefined();
  });

  it('refuses an interval that is not a whole number of at least one', () => {
    for (const every of ['', '0', '-1', '1.5', 'x', '1e3', '99999999999999999999']) {
      expect(problemOf(requestOf({ ...VALID, every })), every).toMatch(/whole number/);
    }
    expect(problemOf(requestOf({ ...VALID, every: '9007199254740991', unit: 'DAYS' }))).toMatch(/too long/);
  });

  it('needs a name of 1 to 128 characters, counted as people count them', () => {
    expect(problemOf(requestOf({ ...VALID, name: '   ' }))).toMatch(/name/);
    expect(problemOf(requestOf({ ...VALID, name: 'a'.repeat(129) }))).toMatch(/name/);
    expect(problemOf(requestOf({ ...VALID, name: '🐙'.repeat(128) }))).toBeUndefined();
    expect(bodyOf(requestOf({ ...VALID, name: '  nightly  ' })).name).toBe('nightly');
  });

  it('needs a workflow, by its id', () => {
    expect(problemOf(requestOf({ ...VALID, workflowId: '' }))).toMatch(/workflow/);
    expect(problemOf(requestOf({ ...VALID, workflowId: 'checkout' }))).toMatch(/workflow/);
  });

  it('takes an input that is a JSON object of at most 64 KiB', () => {
    expect(bodyOf(requestOf({ ...VALID, input: '{"region":"eu"}' })).action.input).toEqual({ region: 'eu' });
    for (const input of ['', '{', '[]', '3', '"x"', 'null']) {
      expect(problemOf(requestOf({ ...VALID, input })), input).toMatch(/input must be/);
    }
    expect(problemOf(requestOf({ ...VALID, input: '{"id": 9007199254740993}' }))).toMatch(/strings for exact integers/);
    expect(bodyOf(requestOf({ ...VALID, input: '{"id": "9007199254740993", "n": 12}' })).action.input).toEqual({ id: '9007199254740993', n: 12 });
    expect(problemOf(requestOf({ ...VALID, input: JSON.stringify({ blob: 'x'.repeat(64 * 1024) }) }))).toMatch(/64 KiB/);
    expect(problemOf(requestOf({ ...VALID, input: JSON.stringify({ blob: 'x'.repeat(64 * 1024 - 20) }) }))).toBeUndefined();
  });

  it('writes the start and the end as instants and needs the end after the start', () => {
    const spec = bodyOf(requestOf({ ...VALID, startAt: '2026-12-01T09:00:00+01:00', endAt: '2026-12-31T00:00:00Z' })).spec;
    expect(spec).toMatchObject({ startAt: '2026-12-01T08:00:00.000Z', endAt: '2026-12-31T00:00:00.000Z' });
    expect(problemOf(requestOf({ ...VALID, startAt: 'soon' }))).toMatch(/dates/);
    expect(problemOf(requestOf({ ...VALID, endAt: 'never' }))).toMatch(/dates/);
    expect(problemOf(requestOf({ ...VALID, startAt: '2026-12-01T00:00:00Z', endAt: '2026-12-01T00:00:00Z' }))).toMatch(/after the start/);
    expect(problemOf(requestOf({ ...VALID, startAt: '2026-12-02T00:00:00Z', endAt: '2026-12-01T00:00:00Z' }))).toMatch(/after the start/);
    expect(bodyOf(requestOf(VALID)).spec).not.toHaveProperty('startAt');
  });

  it('needs a time zone and a jitter in whole milliseconds', () => {
    expect(problemOf(requestOf({ ...VALID, timezone: ' ' }))).toMatch(/time zone/);
    expect(problemOf(requestOf({ ...VALID, jitterMs: '-5' }))).toMatch(/jitter/);
    expect(problemOf(requestOf({ ...VALID, jitterMs: '' }))).toMatch(/jitter/);
    expect(bodyOf(requestOf({ ...VALID, jitterMs: '2500' })).spec).toMatchObject({ jitterMs: 2500 });
  });

  it('limits the actions only when asked to, by a whole number of at least one', () => {
    expect(bodyOf(requestOf(VALID))).not.toHaveProperty('remainingActions');
    expect(bodyOf(requestOf({ ...VALID, remainingActions: ' 5 ' })).remainingActions).toBe(5);
    for (const remainingActions of ['0', '-1', '2.5', 'many']) {
      expect(problemOf(requestOf({ ...VALID, remainingActions })), remainingActions).toMatch(/at least 1/);
    }
  });

  it('carries the policies and the pause, and a note only when there is one', () => {
    const body = bodyOf(requestOf({ ...VALID, overlap: 'CANCEL_OTHER', catchupMode: 'ALL', pauseOnFailure: true, paused: true, note: '  until the migration ends  ' }));
    expect(body.policies).toEqual({ overlap: 'CANCEL_OTHER', catchupMode: 'ALL', pauseOnFailure: true });
    expect(body.paused).toBe(true);
    expect(body.note).toBe('until the migration ends');
    expect(bodyOf(requestOf({ ...VALID, note: '   ' }))).not.toHaveProperty('note');
    expect(problemOf(requestOf({ ...VALID, note: 'n'.repeat(MAX_NOTE + 1) }))).toMatch(/note/);
  });

  it('starts the active version of the workflow', () => {
    expect(bodyOf(requestOf(VALID)).action).toEqual({ workflowId: WORKFLOW, versionPolicy: 'ACTIVE', input: {} });
  });
});

describe('the draft that edits a schedule', () => {
  it('shows an interval in the largest unit that divides it', () => {
    const cases: [number, string, string][] = [
      [7_200_000, '2', 'HOURS'],
      [172_800_000, '2', 'DAYS'],
      [90_000, '90', 'SECONDS'],
      [120_000, '2', 'MINUTES'],
      [1_000, '1', 'SECONDS'],
    ];
    for (const [everyMs, every, unit] of cases) {
      const result = draftOf(schedule({ spec: { intervals: [{ everyMs }], timezone: 'UTC', jitterMs: 0 } }));
      expect(result, String(everyMs)).toMatchObject({ draft: { when: 'INTERVAL', every, unit } });
    }
  });

  it('shows cron expressions one to a line', () => {
    const result = draftOf(schedule({ spec: { cron: ['0 9 * * 1-5', '30 18 * * *'], timezone: 'Europe/Lisbon', jitterMs: 250 } }));
    expect(result).toMatchObject({ draft: { when: 'CRON', cron: '0 9 * * 1-5\n30 18 * * *', timezone: 'Europe/Lisbon', jitterMs: '250' } });
  });

  it('never fills the input, which the platform does not return, and says the schedule is paused', () => {
    const result = draftOf(schedule({ state: 'PAUSED', note: 'holiday', remainingActions: 4 }));
    expect(result).toMatchObject({ draft: { input: '{}', paused: true, note: 'holiday', remainingActions: '4', workflowId: WORKFLOW } });
  });

  it('refuses what the form cannot write: calendars, exclusions, several rules, none', () => {
    const calendar = { hour: [{ start: 3 }] };
    for (const spec of [
      { calendars: [calendar], timezone: 'UTC', jitterMs: 0 },
      { intervals: [{ everyMs: 60_000 }], exclusions: [calendar], timezone: 'UTC', jitterMs: 0 },
      { intervals: [{ everyMs: 60_000 }, { everyMs: 120_000 }], timezone: 'UTC', jitterMs: 0 },
      { intervals: [{ everyMs: 60_000 }], cron: ['* * * * *'], timezone: 'UTC', jitterMs: 0 },
      { cron: ['* * * * *'], calendars: [calendar], timezone: 'UTC', jitterMs: 0 },
      { timezone: 'UTC', jitterMs: 0 },
    ]) {
      expect(draftOf(schedule({ spec: spec })), JSON.stringify(spec)).toHaveProperty('cannot');
    }
  });

  it('saves what the form does not show as it was: the catch-up window, the buffer, the DST rule, the offset, the pinned version', () => {
    const versionId = '0194f5e2-7b1c-7000-8000-0000000000bb';
    const original = schedule({
      spec: { intervals: [{ everyMs: 3_600_000, offsetMs: 900_000 }], timezone: 'Europe/Lisbon', jitterMs: 0, dst: { gap: 'SHIFT_FORWARD', ambiguous: 'TWICE' } },
      policies: { overlap: 'BUFFER_ALL', catchupWindowMs: 60_000, catchupMode: 'ALL', pauseOnFailure: true, bufferLimit: 7 },
      action: { workflowId: WORKFLOW, versionPolicy: 'PINNED', versionId, inputDigest: 'cd'.repeat(32), nominalTimeField: 'at', runTimeoutMs: 30_000 },
    });
    const result = draftOf(original);
    if (!('draft' in result)) throw new Error(result.cannot);
    const body = bodyOf(requestOf({ ...result.draft, input: '{"k":1}' }));
    expect(body.spec).toEqual({
      intervals: [{ everyMs: 3_600_000, offsetMs: 900_000 }],
      timezone: 'Europe/Lisbon',
      jitterMs: 0,
      dst: { gap: 'SHIFT_FORWARD', ambiguous: 'TWICE' },
    });
    expect(body.policies).toEqual({ overlap: 'BUFFER_ALL', catchupWindowMs: 60_000, catchupMode: 'ALL', pauseOnFailure: true, bufferLimit: 7 });
    expect(body.action).toEqual({ workflowId: WORKFLOW, versionPolicy: 'PINNED', versionId, input: { k: 1 }, nominalTimeField: 'at', runTimeoutMs: 30_000 });
  });

  it('writes back the schedule it was made from, apart from its input', () => {
    for (const spec of [
      { intervals: [{ everyMs: 86_400_000 }], timezone: 'UTC', jitterMs: 5, startAt: '2026-12-01T00:00:00.000Z', endAt: '2027-12-01T00:00:00.000Z' },
      { cron: ['*/5 * * * *'], timezone: 'Asia/Tokyo', jitterMs: 0 },
    ]) {
      const result = draftOf(schedule({ spec: spec }));
      if (!('draft' in result)) throw new Error(result.cannot);
      expect(bodyOf(requestOf(result.draft)).spec).toEqual(spec);
    }
  });
});

describe('a schedule in words', () => {
  it('describes an interval, cron expressions and calendars with the time zone', () => {
    expect(describeSpec(schedule())).toBe('every 2 hours (UTC)');
    expect(describeSpec(schedule({ spec: { intervals: [{ everyMs: 3_600_000 }], timezone: 'UTC', jitterMs: 0 } }))).toBe('every hour (UTC)');
    expect(describeSpec(schedule({ spec: { intervals: [{ everyMs: 86_400_000 }], timezone: 'UTC', jitterMs: 0 } }))).toBe('every day (UTC)');
    expect(describeSpec(schedule({ spec: { cron: ['0 9 * * 1'], timezone: 'Europe/Lisbon', jitterMs: 0 } }))).toBe('cron 0 9 * * 1 (Europe/Lisbon)');
    expect(describeSpec(schedule({ spec: { calendars: [{}, {}], cron: ['* * * * *'], timezone: 'UTC', jitterMs: 0 } }))).toBe('cron * * * * *, 2 calendars (UTC)');
    expect(describeSpec({ spec: undefined })).toBe('no time');
  });
});

describe('the state of a schedule', () => {
  it('is told by a word and a shape, and exhausted outranks paused', () => {
    expect(scheduleState({ state: 'ACTIVE' })).toMatchObject({ label: 'Active', tone: 'running' });
    expect(scheduleState({ state: 'PAUSED' })).toMatchObject({ label: 'Paused', tone: 'waiting' });
    expect(scheduleState({ state: 'PAUSED', exhausted: true })).toMatchObject({ label: 'Exhausted' });
    expect(scheduleState({ state: 'ACTIVE', exhausted: false })).toMatchObject({ label: 'Active' });
    for (const state of ['ACTIVE', 'PAUSED'] as const) {
      const view = scheduleState({ state });
      expect(view.symbol.length).toBeGreaterThan(0);
      expect(view.help.length).toBeGreaterThan(0);
    }
  });
});

describe('the outcome of a decision', () => {
  it('has a word, a shape and a help for every outcome the contract names, each different', () => {
    expect(OUTCOMES.length).toBe(10);
    const labels = new Set<string>();
    for (const outcome of OUTCOMES) {
      const view = outcomeView(outcome);
      expect(view.label, outcome).not.toBe('Another outcome');
      expect(view.symbol.length, outcome).toBeGreaterThan(0);
      expect(view.help.length, outcome).toBeGreaterThan(0);
      labels.add(view.label);
    }
    expect(labels.size).toBe(OUTCOMES.length);
  });

  it('says "another outcome" for one this version does not know, and keeps its name as the help', () => {
    expect(outcomeView('FROM_THE_FUTURE')).toMatchObject({ label: 'Another outcome', tone: 'idle', help: 'FROM_THE_FUTURE' });
  });

  it('explains a closed reason code, shows an unknown one as it is, and nothing for none', () => {
    expect(reasonText('TRACKING_LIMIT')).toMatch(/open runs/);
    expect(reasonText('SOMETHING_NEW')).toBe('SOMETHING_NEW');
    expect(reasonText(null)).toBe('');
    expect(reasonText(undefined)).toBe('');
    expect(reasonText('')).toBe('');
  });
});

describe('a backfill', () => {
  const NOW = new Date('2026-11-04T12:00:00Z');

  it('covers the times from the start up to, not including, the end', () => {
    expect(backfillOf('2026-11-01T00:00:00Z', '2026-11-02T00:00:00Z', '', NOW)).toEqual({ body: { start: '2026-11-01T00:00:00.000Z', end: '2026-11-02T00:00:00.000Z' } });
    expect(backfillOf('2026-11-01T00:00:00Z', '2026-11-02T00:00:00Z', 'ALLOW_ALL', NOW)).toEqual({
      body: { start: '2026-11-01T00:00:00.000Z', end: '2026-11-02T00:00:00.000Z', overlap: 'ALLOW_ALL' },
    });
  });

  it('refuses an end that is not after the start or that has not come yet, and dates that are not dates', () => {
    const problem = (start: string, end: string) => {
      const result = backfillOf(start, end, '', NOW);
      return 'problem' in result ? result.problem : undefined;
    };
    expect(problem('2026-11-02T00:00:00Z', '2026-11-02T00:00:00Z')).toMatch(/after the start/);
    expect(problem('2026-11-03T00:00:00Z', '2026-11-02T00:00:00Z')).toMatch(/after the start/);
    expect(problem('2026-11-01T00:00:00Z', '2026-11-04T12:00:00.001Z')).toMatch(/future/);
    expect(problem('2026-11-01T00:00:00Z', '2026-11-04T12:00:00Z')).toBeUndefined();
    expect(problem('', '2026-11-02T00:00:00Z')).toMatch(/dates/);
    expect(problem('2026-11-01T00:00:00Z', 'yesterday')).toMatch(/dates/);
  });
});

describe('the reason of a pause or a resume', () => {
  it('is 1 to 512 characters once trimmed', () => {
    expect(noteProblem('')).toMatch(/why/);
    expect(noteProblem('   ')).toMatch(/why/);
    expect(noteProblem('holiday')).toBeUndefined();
    expect(noteProblem('🐙'.repeat(MAX_NOTE))).toBeUndefined();
    expect(noteProblem('x'.repeat(MAX_NOTE + 1))).toMatch(/512/);
  });
});
