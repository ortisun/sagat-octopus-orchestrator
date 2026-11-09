import { screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { ScheduleActionRecord } from '../../api/endpoints';
import { BASE, ID, NS, backfill, bodyOf, decision, described, failure, json, keyOf, mountSchedules, schedule, writes } from '../../../test/schedules';
import { SchedulePage } from './SchedulePage';

type Options = { permissions?: string[]; routes?: Record<string, (init: RequestInit, url: string) => unknown> };
const mount = (options: Options = {}) => mountSchedules(<SchedulePage />, { path: `/schedules/${ID}`, route: '/schedules/:scheduleId', ...options });
const inDialog = () => within(screen.getByRole('dialog'));
const READ = ['schedule:read'];

describe('A schedule', () => {
  it('says what it is, when it acts next and what it holds, without its input', async () => {
    await mount({ routes: { [`GET ${BASE}/${ID}`]: () => described({ remainingActions: 5, counters: { STARTED: 12, SKIPPED_OVERLAP: 2, COALESCED: 0 } }) } });
    expect(await screen.findByRole('heading', { name: /nightly/ })).toBeVisible();
    expect(screen.getByText('Active')).toBeVisible();
    // The names of the workflows arrive on their own request.
    expect(await screen.findByRole('link', { name: 'nightly-report' })).toHaveAttribute('href', '/workflows/0194f5e2-7b1c-7000-8000-000000000001');
    expect(screen.getByText('every 2 hours (UTC)')).toBeVisible();
    expect(screen.getByText('The active version')).toBeVisible();
    expect(screen.getByText('Actions left').nextSibling).toHaveTextContent('5');
    expect(screen.getByText('Acts for').nextSibling).toHaveTextContent('alice');
    const next = screen.getByRole('heading', { name: 'Next times' }).nextElementSibling as HTMLElement;
    const times = within(next).getAllByRole('listitem');
    expect(times).toHaveLength(2);
    expect(times[1]).toHaveTextContent('moved by the jitter');
    expect(times[0]).not.toHaveTextContent('moved by the jitter');
    expect(screen.getByText('Started a run', { selector: 'dt' }).closest('div')).toHaveTextContent('12');
    expect(screen.queryByText('Coalesced', { selector: 'dt' })).not.toBeInTheDocument();
    expect(screen.getByText(/input of its runs is not shown/)).toBeVisible();
    expect(screen.getByText(/abababababababab…/)).toBeVisible();
    expect(document.body).not.toHaveTextContent('ab'.repeat(32));
  });

  it('asks the platform for its next times and no recent decisions, which it lists apart', async () => {
    const { platform } = await mount();
    await screen.findByRole('heading', { name: /nightly/ });
    const call = platform.calls.find((c) => c.url.includes(`${BASE}/${ID}?`));
    expect(call?.url).toContain('nextCount=10');
    expect(call?.url).toContain('recentCount=0');
  });

  it('says it is paused, why, and that no time is next', async () => {
    await mount({ routes: { [`GET ${BASE}/${ID}`]: () => described({ state: 'PAUSED', note: 'holiday', noteBy: 'bob', nextActions: [] }) } });
    expect(await screen.findByText('Paused', { selector: '.badge span' })).toBeVisible();
    expect(screen.getByText(/holiday — bob/)).toBeVisible();
    expect(screen.getByText(/None: it is paused/)).toBeVisible();
  });

  it('says it has no action left', async () => {
    await mount({ routes: { [`GET ${BASE}/${ID}`]: () => described({ exhausted: true, remainingActions: 0 }) } });
    expect(await screen.findByText(/every action it was allowed/)).toBeVisible();
    expect(screen.getByText(/None: it has no action left/)).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Pause' })).not.toBeInTheDocument();
  });

  it('links the runs it has open and counts the times that wait', async () => {
    await mount({ routes: { [`GET ${BASE}/${ID}`]: () => described({ openRuns: ['0194f5e2-7b1c-7000-8000-0000000000d4'], bufferedCount: 2 }) } });
    expect(await screen.findByText(/1 run it started is open/)).toBeVisible();
    // the identifiers are time-ordered: the start of one is the same for every run made within a minute, the end is what tells them apart
    expect(screen.getByRole('link', { name: '000000d4' })).toHaveAttribute('href', '/runs/0194f5e2-7b1c-7000-8000-0000000000d4');
    expect(screen.getByText(/2 times are waiting for a run to end/)).toBeVisible();
  });

  it('says a cron trigger owns it, and offers neither edit nor delete', async () => {
    await mount({ routes: { [`GET ${BASE}/${ID}`]: () => described({ origin: 'TRIGGER' }) } });
    expect(await screen.findByText(/cron trigger owns this schedule/)).toBeVisible();
    expect(screen.queryByRole('link', { name: 'Edit' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Delete' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Pause' })).toBeVisible();
  });

  it('says, for a schedule a cron trigger owns, that its controls wait for the operator and says the refusal in words', async () => {
    const { user, platform } = await mount({
      routes: {
        [`GET ${BASE}/${ID}`]: () => described({ origin: 'TRIGGER', triggerId: '0194f5e2-7b1c-7000-8000-0000000000e1' }),
        [`POST ${BASE}/${ID}/pause`]: () => failure(409, 'CONFLICT', 'SCHEDULE_TRIGGER_HANDOVER_PENDING'),
        [`POST ${BASE}/${ID}/trigger`]: () => failure(409, 'CONFLICT', 'SCHEDULE_TRIGGER_HANDOVER_PENDING'),
      },
    });
    expect(await screen.findByText(/once an operator has handed the cron triggers over/)).toBeVisible();
    await user.click(screen.getByRole('button', { name: 'Pause' }));
    await user.type(inDialog().getByRole('textbox'), 'maintenance');
    await user.click(inDialog().getByRole('button', { name: 'Confirm pause' }));
    expect(await inDialog().findByText(/Pausing the schedule failed/)).toBeVisible();
    expect(inDialog().getByText('SCHEDULE_TRIGGER_HANDOVER_PENDING', { selector: 'code' })).toBeVisible();
    expect(screen.queryByText('The schedule is paused.')).not.toBeInTheDocument();
    expect(writes(platform).map((call) => call.method)).toEqual(['POST']);
  });

  it('disables pause and run now of a schedule a cron trigger owns, with the reason, until the handover, and enables them after it', async () => {
    await mount({ routes: { [`GET ${BASE}/${ID}`]: () => described({ origin: 'TRIGGER', triggerId: '0194f5e2-7b1c-7000-8000-0000000000e1', controllable: false }) } });
    expect(await screen.findByText(/an operator has not handed the cron triggers over/)).toBeVisible();
    expect(screen.getByRole('button', { name: 'Pause' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Run now' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Pause' })).toHaveAttribute('title', expect.stringContaining('handed the cron triggers over'));
  });

  it('offers pause and run now of a schedule a cron trigger owns once the platform says the control is open', async () => {
    await mount({ routes: { [`GET ${BASE}/${ID}`]: () => described({ origin: 'TRIGGER', triggerId: '0194f5e2-7b1c-7000-8000-0000000000e1', controllable: true }) } });
    await screen.findByRole('heading', { name: /nightly/ });
    expect(screen.getByRole('button', { name: 'Pause' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Run now' })).toBeEnabled();
    expect(screen.queryByText(/an operator has not handed the cron triggers over/)).not.toBeInTheDocument();
  });

  it('says so when it is not there', async () => {
    await mount({ routes: { [`GET ${BASE}/${ID}`]: () => failure(404, 'NOT_FOUND') } });
    expect(await screen.findByText(/Loading the schedule failed/)).toBeVisible();
  });

  it('is not shown, and asks nothing of the platform, without schedule:read', async () => {
    const { platform } = await mount({ permissions: [] });
    expect(await screen.findByText(/schedule:read/)).toBeVisible();
    expect(platform.calls.filter((call) => call.url.includes('/schedules'))).toHaveLength(0);
  });
});

describe('What a person may do to a schedule', () => {
  const buttons = () => ['Pause', 'Resume', 'Run now', 'Delete'].filter((name) => screen.queryByRole('button', { name }));
  const edits = () => screen.queryByRole('link', { name: 'Edit' }) !== null;

  it('is everything, to somebody with every permission', async () => {
    await mount();
    await screen.findByRole('heading', { name: /nightly/ });
    expect(buttons()).toEqual(['Pause', 'Run now', 'Delete']);
    expect(edits()).toBe(true);
  });

  it('is resuming, not pausing, when it is paused', async () => {
    await mount({ routes: { [`GET ${BASE}/${ID}`]: () => described({ state: 'PAUSED' }) } });
    await screen.findByRole('heading', { name: /nightly/ });
    expect(buttons()).toEqual(['Resume', 'Run now', 'Delete']);
  });

  it.each([
    [['schedule:read'], [], false],
    [['schedule:read', 'schedule:operate'], ['Pause'], false],
    [['schedule:read', 'schedule:operate', 'run:start'], ['Pause', 'Run now'], false],
    [['schedule:read', 'schedule:write'], [], false],
    [['schedule:read', 'schedule:write', 'run:start'], ['Delete'], true],
  ])('is %j: %j, and editing %s', async (permissions, expected, editing) => {
    await mount({ permissions });
    await screen.findByRole('heading', { name: /nightly/ });
    expect(buttons()).toEqual(expected);
    expect(edits()).toBe(editing);
  });

  it('is resuming only with run:start, as the platform asks for it', async () => {
    await mount({ permissions: [...READ, 'schedule:operate'], routes: { [`GET ${BASE}/${ID}`]: () => described({ state: 'PAUSED' }) } });
    await screen.findByRole('heading', { name: /nightly/ });
    expect(buttons()).toEqual([]);
  });
});

describe('Pausing and resuming', () => {
  it('needs a reason, and posts it trimmed with a key', async () => {
    const { user, platform } = await mount({ routes: { [`POST ${BASE}/${ID}/pause`]: () => schedule({ state: 'PAUSED' }) } });
    await user.click(await screen.findByRole('button', { name: 'Pause' }));
    await user.click(inDialog().getByRole('button', { name: 'Confirm pause' }));
    expect(inDialog().getByRole('alert')).toHaveTextContent('Say why');
    expect(writes(platform)).toHaveLength(0);
    await user.type(inDialog().getByLabelText('Why'), '  holiday  ');
    await user.click(inDialog().getByRole('button', { name: 'Confirm pause' }));
    await waitFor(() => expect(writes(platform)).toHaveLength(1));
    expect(bodyOf(writes(platform)[0]!)).toEqual({ note: 'holiday' });
    expect(keyOf(writes(platform)[0]!)).toBeTruthy();
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });

  it('shows the schedule as the platform then describes it', async () => {
    let paused = false;
    const { user } = await mount({
      routes: {
        [`GET ${BASE}/${ID}`]: () => described({ state: paused ? 'PAUSED' : 'ACTIVE', nextActions: paused ? [] : described().nextActions }),
        [`POST ${BASE}/${ID}/pause`]: () => {
          paused = true;
          return schedule({ state: 'PAUSED' });
        },
      },
    });
    await user.click(await screen.findByRole('button', { name: 'Pause' }));
    await user.type(inDialog().getByLabelText('Why'), 'holiday');
    await user.click(inDialog().getByRole('button', { name: 'Confirm pause' }));
    expect(await screen.findByRole('button', { name: 'Resume' })).toBeVisible();
    expect(screen.getByText(/None: it is paused/)).toBeVisible();
  });

  it('puts focus on the control that takes the place of the one that opened the dialog, so a keyboard is not sent back to the top of the page: Resume after a pause, Pause after a resume', async () => {
    let state: 'ACTIVE' | 'PAUSED' = 'ACTIVE';
    const { user } = await mount({
      routes: {
        [`GET ${BASE}/${ID}`]: () => described({ state }),
        [`POST ${BASE}/${ID}/pause`]: () => {
          state = 'PAUSED';
          return schedule({ state });
        },
        [`POST ${BASE}/${ID}/resume`]: () => {
          state = 'ACTIVE';
          return schedule({ state });
        },
      },
    });
    await user.click(await screen.findByRole('button', { name: 'Pause' }));
    await user.type(inDialog().getByLabelText('Why'), 'holiday');
    await user.click(inDialog().getByRole('button', { name: 'Confirm pause' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Resume' })).toHaveFocus());
    await user.click(screen.getByRole('button', { name: 'Resume' }));
    await user.type(inDialog().getByLabelText('Why'), 'back');
    await user.click(inDialog().getByRole('button', { name: 'Confirm resume' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Pause' })).toHaveFocus());
  });

  it('moves focus only for the pause or the resume the person made: a pause that is not seen as one, and a change somebody else makes later, leave focus where it is', async () => {
    let state: 'ACTIVE' | 'PAUSED' = 'ACTIVE';
    const { user, app } = await mount({
      routes: {
        // the schedule is read as it was after the pause (it took its last action meanwhile, say): nothing flips on this read
        [`GET ${BASE}/${ID}`]: () => described({ state }),
        [`POST ${BASE}/${ID}/pause`]: () => schedule({ state: 'PAUSED' }),
      },
    });
    await user.click(await screen.findByRole('button', { name: 'Pause' }));
    await user.type(inDialog().getByLabelText('Why'), 'holiday');
    await user.click(inDialog().getByRole('button', { name: 'Confirm pause' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    // the person goes on to the link to edit it, and then another operator pauses the schedule, which the page reads on its round
    const edit = screen.getByRole('link', { name: 'Edit' });
    edit.focus();
    state = 'PAUSED';
    await app.queries.invalidateQueries({ queryKey: ['schedule', NS, ID], exact: true });
    expect(await screen.findByRole('button', { name: 'Resume' })).toBeVisible();
    expect(edit).toHaveFocus();
  });

  it('puts focus on the heading when the control that took the place of the one pressed is not for this person: a pause by somebody who may not start runs leaves no Resume', async () => {
    let state: 'ACTIVE' | 'PAUSED' = 'ACTIVE';
    const { user } = await mount({
      permissions: [...READ, 'schedule:operate'],
      routes: {
        [`GET ${BASE}/${ID}`]: () => described({ state }),
        [`POST ${BASE}/${ID}/pause`]: () => {
          state = 'PAUSED';
          return schedule({ state });
        },
      },
    });
    await user.click(await screen.findByRole('button', { name: 'Pause' }));
    await user.type(inDialog().getByLabelText('Why'), 'holiday');
    await user.click(inDialog().getByRole('button', { name: 'Confirm pause' }));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Pause' })).not.toBeInTheDocument());
    expect(screen.queryByRole('button', { name: 'Resume' })).not.toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole('heading', { level: 1 })).toHaveFocus());
  });

  it('resumes with a reason, and says no missed time is caught up', async () => {
    const { user, platform } = await mount({
      routes: { [`GET ${BASE}/${ID}`]: () => described({ state: 'PAUSED' }), [`POST ${BASE}/${ID}/resume`]: () => schedule() },
    });
    await user.click(await screen.findByRole('button', { name: 'Resume' }));
    expect(inDialog().getByText(/not caught up/)).toBeVisible();
    await user.type(inDialog().getByLabelText('Why'), 'back from holiday');
    await user.click(inDialog().getByRole('button', { name: 'Confirm resume' }));
    await waitFor(() => expect(writes(platform)).toHaveLength(1));
    expect(bodyOf(writes(platform)[0]!)).toEqual({ note: 'back from holiday' });
  });

  it('keeps the key of an identical retry and changes it with the reason', async () => {
    let fail = true;
    const { user, platform } = await mount({
      routes: { [`POST ${BASE}/${ID}/pause`]: () => (fail ? json(503, { code: 'UNAVAILABLE', message: 'Try again soon.', retryable: true }) : schedule({ state: 'PAUSED' })) },
    });
    await user.click(await screen.findByRole('button', { name: 'Pause' }));
    await user.type(inDialog().getByLabelText('Why'), 'holiday');
    await user.click(inDialog().getByRole('button', { name: 'Confirm pause' }));
    await inDialog().findByText(/Try again soon/);
    await user.click(inDialog().getByRole('button', { name: 'Confirm pause' }));
    await waitFor(() => expect(writes(platform)).toHaveLength(2));
    expect(keyOf(writes(platform)[0]!)).toBe(keyOf(writes(platform)[1]!));
    await user.type(inDialog().getByLabelText('Why'), ' week');
    fail = false;
    await user.click(inDialog().getByRole('button', { name: 'Confirm pause' }));
    await waitFor(() => expect(writes(platform)).toHaveLength(3));
    expect(keyOf(writes(platform)[2]!)).not.toBe(keyOf(writes(platform)[0]!));
  });
});

describe('Running a schedule now', () => {
  it('uses its own rule unless told otherwise, and says what was decided, with the run', async () => {
    const { user, platform } = await mount({ routes: { [`POST ${BASE}/${ID}/trigger`]: () => decision({ kind: 'TRIGGERED', origin: 'MANUAL' }) } });
    await user.click(await screen.findByRole('button', { name: 'Run now' }));
    expect(inDialog().getByText(/rule of the schedule applies: skip/)).toBeVisible();
    await user.click(inDialog().getByRole('button', { name: 'Run now' }));
    expect(await inDialog().findByText('Started a run.')).toBeVisible();
    expect(inDialog().getByRole('link', { name: 'Open the run' })).toHaveAttribute('href', '/runs/0194f5e2-7b1c-7000-8000-0000000000d4');
    expect(bodyOf(writes(platform)[0]!)).toEqual({});
  });

  it('can name another rule for this one time', async () => {
    const { user, platform } = await mount({ routes: { [`POST ${BASE}/${ID}/trigger`]: () => decision({ outcome: 'SKIPPED_OVERLAP', runId: null }) } });
    await user.click(await screen.findByRole('button', { name: 'Run now' }));
    await user.selectOptions(inDialog().getByLabelText('If a run is still open'), 'ALLOW_ALL');
    await user.click(inDialog().getByRole('button', { name: 'Run now' }));
    expect(await inDialog().findByText('Skipped: a run was open.')).toBeVisible();
    expect(inDialog().queryByRole('link', { name: 'Open the run' })).not.toBeInTheDocument();
    expect(bodyOf(writes(platform)[0]!)).toEqual({ overlap: 'ALLOW_ALL' });
  });

  it('warns that it runs even when the schedule is paused', async () => {
    const { user } = await mount({ routes: { [`GET ${BASE}/${ID}`]: () => described({ state: 'PAUSED' }) } });
    await user.click(await screen.findByRole('button', { name: 'Run now' }));
    expect(inDialog().getByText(/paused; this still runs it once/)).toBeVisible();
  });

  it('gets a new key each time the dialog is opened: running twice on purpose is two runs', async () => {
    const { user, platform } = await mount({ routes: { [`POST ${BASE}/${ID}/trigger`]: () => decision() } });
    for (let round = 0; round < 2; round++) {
      await user.click(await screen.findByRole('button', { name: 'Run now' }));
      await user.click(inDialog().getByRole('button', { name: 'Run now' }));
      await inDialog().findByText('Started a run.');
      await user.click(inDialog().getByRole('button', { name: 'Close' }));
      await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    }
    expect(writes(platform)).toHaveLength(2);
    expect(keyOf(writes(platform)[0]!)).not.toBe(keyOf(writes(platform)[1]!));
  });
});

describe('The backfills of a schedule and its decisions', () => {
  it('reads the decisions and the counters again when a backfill has made some, so the history under it is not older than the backfill above it', async () => {
    let state: 'RUNNING' | 'COMPLETED' = 'RUNNING';
    const { app, platform } = await mount({
      routes: {
        [`GET ${BASE}/${ID}/backfills`]: () => ({ items: [backfill({ state, counts: state === 'RUNNING' ? { STARTED: 1 } : { STARTED: 3 } })] }),
      },
    });
    const decisionReads = () => platform.calls.filter((call) => call.method === 'GET' && call.url.startsWith(`/v1${BASE}/${ID}/actions`)).length;
    const scheduleReads = () => platform.calls.filter((call) => call.method === 'GET' && call.url.startsWith(`/v1${BASE}/${ID}?`)).length;
    expect(await screen.findByText('1 started a run')).toBeVisible();
    await waitFor(() => expect(decisionReads()).toBe(1));
    expect(scheduleReads()).toBe(1);
    // the page follows an open backfill every three seconds: this is one of those reads, after it has gone on
    state = 'COMPLETED';
    await app.queries.invalidateQueries({ queryKey: ['schedule-backfills', NS, ID] });
    expect(await screen.findByText('3 started a run')).toBeVisible();
    await waitFor(() => expect(decisionReads()).toBe(2));
    await waitFor(() => expect(scheduleReads()).toBe(2));
  });
});

describe('Deleting a schedule', () => {
  it('says what stays and what goes, and deletes at the revision the person saw', async () => {
    const { user, platform } = await mount({
      routes: { [`GET ${BASE}/${ID}`]: () => described({ revision: 9 }), [`DELETE ${BASE}/${ID}`]: () => new Response(null, { status: 204 }) },
    });
    await user.click(await screen.findByRole('button', { name: 'Delete' }));
    expect(inDialog().getByText(/runs already started go on, and its history stays/)).toBeVisible();
    await user.click(inDialog().getByRole('button', { name: 'Confirm deletion' }));
    await screen.findByRole('heading', { name: 'Schedule list' });
    const [call] = writes(platform);
    expect(call!.method).toBe('DELETE');
    expect(call!.url).toContain('expectedRevision=9');
    expect(keyOf(call!)).toBeTruthy();
  });

  it('says so when the schedule changed since the person saw it', async () => {
    const { user } = await mount({ routes: { [`DELETE ${BASE}/${ID}`]: () => failure(409, 'CONFLICT', 'REVISION_CONFLICT') } });
    await user.click(await screen.findByRole('button', { name: 'Delete' }));
    await user.click(inDialog().getByRole('button', { name: 'Confirm deletion' }));
    expect(await inDialog().findByText(/changed since you opened it/)).toBeVisible();
  });

  it('reads the schedule again when it changed since the person saw it, so what the dialog tells them to look at is the new revision', async () => {
    let revision = 9;
    const { user, platform } = await mount({
      routes: {
        [`GET ${BASE}/${ID}`]: () => described({ revision }),
        [`DELETE ${BASE}/${ID}`]: () => {
          revision = 10;
          return failure(409, 'CONFLICT', 'REVISION_CONFLICT');
        },
      },
    });
    const reads = () => platform.calls.filter((call) => call.method === 'GET' && call.url.startsWith(`/v1${BASE}/${ID}?`)).length;
    await user.click(await screen.findByRole('button', { name: 'Delete' }));
    expect(reads()).toBe(1);
    await user.click(inDialog().getByRole('button', { name: 'Confirm deletion' }));
    expect(await inDialog().findByText(/changed since you opened it/)).toBeVisible();
    await waitFor(() => expect(reads()).toBe(2));
  });

  it('asks again at the revision the person was refused at, never at the one the page has read since: confirming twice deletes nothing the person did not see', async () => {
    let revision = 9;
    const { user, platform, app } = await mount({
      routes: {
        [`GET ${BASE}/${ID}`]: () => described({ revision }),
        [`DELETE ${BASE}/${ID}`]: () => {
          revision = 10;
          return failure(409, 'CONFLICT', 'REVISION_CONFLICT');
        },
      },
    });
    await user.click(await screen.findByRole('button', { name: 'Delete' }));
    await user.click(inDialog().getByRole('button', { name: 'Confirm deletion' }));
    expect(await inDialog().findByText(/changed since you opened it/)).toBeVisible();
    // the page behind the dialog has read the new revision by now
    await waitFor(() => expect(app.queries.isFetching()).toBe(0));
    await waitFor(() => expect(app.queries.getQueryData<{ revision: number }>(['schedule', NS, ID])?.revision).toBe(10));
    await user.click(inDialog().getByRole('button', { name: 'Confirm deletion' }));
    await waitFor(() => expect(writes(platform)).toHaveLength(2));
    expect(writes(platform).map((call) => new URL(call.url, 'http://studio').searchParams.get('expectedRevision'))).toEqual(['9', '9']);
  });
});

describe('The decisions of a schedule', () => {
  it('say what each time became, in words and shapes, and link its run', async () => {
    await mount({
      routes: {
        [`GET ${BASE}/${ID}/actions`]: () => ({
          items: [
            decision(),
            decision({ nominalTime: '2026-11-03T02:00:00Z', outcome: 'SKIPPED_OVERLAP', reason: 'OVERLAP', runId: null, decidedAt: '2026-11-03T02:00:01Z' }),
            decision({ nominalTime: '2026-11-02T02:00:00Z', outcome: 'COALESCED', summaryCount: 4, runId: null, kind: 'BACKFILL', decidedAt: '2026-11-02T02:00:01Z' }),
            decision({ nominalTime: '2026-11-01T02:00:00Z', outcome: 'SOMETHING_NEW' as ScheduleActionRecord['outcome'], runId: null, kind: 'TRIGGERED', decidedAt: '2026-11-01T02:00:01Z' }),
          ],
          nextCursor: null,
        }),
      },
    });
    const table = await screen.findByRole('table', { name: /decisions of the schedule/ });
    const rows = within(table).getAllByRole('row').slice(1);
    expect(rows).toHaveLength(4);
    expect(within(rows[0]!).getByText('Started a run')).toBeVisible();
    expect(within(rows[0]!).getByRole('link')).toHaveAttribute('href', '/runs/0194f5e2-7b1c-7000-8000-0000000000d4');
    expect(within(rows[1]!).getByText('Skipped: a run was open')).toBeVisible();
    expect(within(rows[1]!).getByText('A run of the schedule was open.')).toBeVisible();
    expect(within(rows[2]!).getByText(/4 times/)).toBeVisible();
    expect(within(rows[2]!).getByText('Backfill')).toBeVisible();
    expect(within(rows[3]!).getByText('Another outcome')).toBeVisible();
    expect(within(rows[3]!).getByText('Run now')).toBeVisible();
  });

  it('can be limited to one outcome, asked of the platform, and paged', async () => {
    let page = 0;
    const { user, platform } = await mount({
      routes: {
        [`GET ${BASE}/${ID}/actions`]: (_init, url) => {
          if (url.includes('outcome=SKIPPED_OVERLAP')) return { items: [decision({ outcome: 'SKIPPED_OVERLAP', runId: null })], nextCursor: page++ === 0 ? 'c1' : null };
          return { items: [decision()], nextCursor: null };
        },
      },
    });
    await screen.findByRole('table', { name: /decisions of the schedule/ });
    await user.selectOptions(screen.getByLabelText('Outcome'), 'SKIPPED_OVERLAP');
    await waitFor(() => expect(platform.calls.some((call) => call.url.includes('outcome=SKIPPED_OVERLAP'))).toBe(true));
    await user.click(await screen.findByRole('button', { name: 'Load more' }));
    await waitFor(() => expect(platform.calls.some((call) => call.url.includes('cursor=c1'))).toBe(true));
  });

  it('say so when there are none, or none of the outcome chosen', async () => {
    const { user } = await mount({ routes: { [`GET ${BASE}/${ID}/actions`]: () => ({ items: [], nextCursor: null }) } });
    expect(await screen.findByText('The schedule has not decided anything yet.')).toBeVisible();
    await user.selectOptions(screen.getByLabelText('Outcome'), 'COALESCED');
    expect(await screen.findByText('No decision with this outcome.')).toBeVisible();
  });
});

describe('The backfills of a schedule', () => {
  const NOW_PAST = { start: '2020-11-01T00:00:00Z', end: '2020-11-02T00:00:00Z' };

  it('say how far each is, and can be cancelled while open', async () => {
    const { user, platform } = await mount({
      routes: {
        [`GET ${BASE}/${ID}/backfills`]: () => ({ items: [backfill(), backfill({ backfillId: '0194f5e2-7b1c-7000-8000-0000000000e6', state: 'COMPLETED', counts: { STARTED: 24 } })] }),
        [`POST ${BASE}/${ID}/backfills/0194f5e2-7b1c-7000-8000-0000000000e5/cancel`]: () => backfill({ state: 'CANCELLED' }),
      },
    });
    const table = await screen.findByRole('table', { name: /latest backfills/ });
    const rows = within(table).getAllByRole('row').slice(1);
    expect(within(rows[0]!).getByText('Running')).toBeVisible();
    expect(within(rows[0]!).getByText('3 started a run, 1 already acted on')).toBeVisible();
    expect(within(rows[1]!).getByText('Completed')).toBeVisible();
    expect(within(rows[1]!).queryByRole('button', { name: 'Cancel' })).not.toBeInTheDocument();
    await user.click(within(rows[0]!).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(writes(platform)).toHaveLength(1));
    expect(keyOf(writes(platform)[0]!)).toBeTruthy();
  });

  it('are started over a range that has passed, with the key of an identical retry kept', async () => {
    let fail = true;
    const { user, platform } = await mount({
      routes: { [`POST ${BASE}/${ID}/backfills`]: () => (fail ? json(503, { code: 'UNAVAILABLE', message: 'Try again soon.', retryable: true }) : backfill()) },
    });
    await user.click(await screen.findByRole('button', { name: 'New backfill' }));
    await user.type(inDialog().getByLabelText('From'), NOW_PAST.start);
    await user.type(inDialog().getByLabelText('Up to, not including'), NOW_PAST.end);
    await user.click(inDialog().getByRole('button', { name: 'Start the backfill' }));
    await inDialog().findByText(/Try again soon/);
    fail = false;
    await user.click(inDialog().getByRole('button', { name: 'Start the backfill' }));
    await waitFor(() => expect(writes(platform)).toHaveLength(2));
    expect(keyOf(writes(platform)[0]!)).toBe(keyOf(writes(platform)[1]!));
    expect(bodyOf(writes(platform)[1]!)).toEqual({ start: '2020-11-01T00:00:00.000Z', end: '2020-11-02T00:00:00.000Z' });
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });

  it('refuse a range that is backwards or reaches into the future, before asking the platform', async () => {
    const { user, platform } = await mount();
    await user.click(await screen.findByRole('button', { name: 'New backfill' }));
    await user.type(inDialog().getByLabelText('From'), '2020-11-02T00:00:00Z');
    await user.type(inDialog().getByLabelText('Up to, not including'), '2020-11-01T00:00:00Z');
    await user.click(inDialog().getByRole('button', { name: 'Start the backfill' }));
    expect(inDialog().getByRole('alert')).toHaveTextContent('after the start');
    await user.clear(inDialog().getByLabelText('Up to, not including'));
    await user.type(inDialog().getByLabelText('Up to, not including'), '2999-01-01T00:00:00Z');
    await user.click(inDialog().getByRole('button', { name: 'Start the backfill' }));
    expect(inDialog().getByRole('alert')).toHaveTextContent('future');
    expect(writes(platform)).toHaveLength(0);
  });

  it('are listed to a reader, who has no way to start or cancel one', async () => {
    await mount({ permissions: READ, routes: { [`GET ${BASE}/${ID}/backfills`]: () => ({ items: [backfill()] }) } });
    await screen.findByRole('table', { name: /latest backfills/ });
    expect(screen.queryByRole('button', { name: 'New backfill' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Cancel' })).not.toBeInTheDocument();
  });

  it('say so when there are none', async () => {
    await mount();
    expect(await screen.findByText('No backfill yet.')).toBeVisible();
  });
});
