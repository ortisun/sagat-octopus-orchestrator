import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { BASE, ID, NS, WORKFLOW, bodyOf, described, failure, json, keyOf, mountSchedules, schedule, writes } from '../../../test/schedules';
import { ScheduleFormPage } from './ScheduleFormPage';

const create = (options: { permissions?: string[]; routes?: Record<string, (init: RequestInit, url: string) => unknown> } = {}) =>
  mountSchedules(<ScheduleFormPage />, { path: '/schedules/new', route: '/schedules/new', ...options });
const edit = (options: { permissions?: string[]; routes?: Record<string, (init: RequestInit, url: string) => unknown> } = {}) =>
  mountSchedules(<ScheduleFormPage />, { path: `/schedules/${ID}/edit`, route: '/schedules/:scheduleId/edit', ...options });

async function fillMinimum(user: Awaited<ReturnType<typeof create>>['user'], name = 'nightly') {
  await screen.findByRole('heading', { name: 'New schedule' });
  await user.type(screen.getByLabelText('Name'), name);
  await user.selectOptions(await screen.findByLabelText('Workflow'), WORKFLOW);
}

describe('Creating a schedule', () => {
  it('offers only the workflows that have a published version', async () => {
    await create();
    const select = await screen.findByLabelText('Workflow');
    await waitFor(() => expect(screen.getByRole('option', { name: 'nightly-report' })).toBeVisible());
    expect(screen.queryByRole('option', { name: 'unpublished' })).not.toBeInTheDocument();
    expect(select).toHaveValue('');
  });

  it('posts what was typed, once, and goes to the new schedule', async () => {
    const { user, platform } = await create({ routes: { [`POST ${BASE}`]: () => schedule({ scheduleId: ID }) } });
    await fillMinimum(user);
    fireEvent.change(screen.getByLabelText('Input'), { target: { value: '{"region":"eu"}' } });
    await user.clear(screen.getByLabelText('Every'));
    await user.type(screen.getByLabelText('Every'), '6');
    await user.selectOptions(screen.getByLabelText('Unit'), 'HOURS');
    await user.selectOptions(screen.getByLabelText('When a run is still open'), 'BUFFER_ONE');
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    await screen.findByRole('heading', { name: 'Schedule landing' });
    const [post] = writes(platform);
    expect(writes(platform)).toHaveLength(1);
    expect(post!.url).toBe(`/v1${BASE}`);
    expect(bodyOf(post!)).toEqual({
      name: 'nightly',
      spec: { intervals: [{ everyMs: 21_600_000 }], timezone: 'UTC', jitterMs: 0 },
      policies: { overlap: 'BUFFER_ONE', catchupMode: 'ONE', pauseOnFailure: false },
      action: { workflowId: WORKFLOW, versionPolicy: 'ACTIVE', input: { region: 'eu' } },
      paused: false,
    });
    expect(keyOf(post!)).toBeTruthy();
  });

  it('writes cron expressions when asked to repeat by them', async () => {
    const { user, platform } = await create({ routes: { [`POST ${BASE}`]: () => schedule() } });
    await fillMinimum(user);
    await user.selectOptions(screen.getByLabelText('Repeat by'), 'CRON');
    expect(screen.queryByLabelText('Every')).not.toBeInTheDocument();
    await user.type(screen.getByLabelText('Cron expressions'), '0 9 * * 1-5');
    await user.clear(screen.getByLabelText('Time zone'));
    await user.type(screen.getByLabelText('Time zone'), 'Europe/Lisbon');
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    await screen.findByRole('heading', { name: 'Schedule landing' });
    expect(bodyOf(writes(platform)[0]!)['spec']).toEqual({ cron: ['0 9 * * 1-5'], timezone: 'Europe/Lisbon', jitterMs: 0 });
  });

  it('refuses what is wrong, in words, before asking the platform', async () => {
    const { user, platform } = await create();
    await screen.findByRole('heading', { name: 'New schedule' });
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    expect(screen.getByRole('alert')).toHaveTextContent('Give the schedule a name');
    await user.type(screen.getByLabelText('Name'), 'nightly');
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    expect(screen.getByRole('alert')).toHaveTextContent('Choose the workflow');
    await user.selectOptions(await screen.findByLabelText('Workflow'), WORKFLOW);
    fireEvent.change(screen.getByLabelText('Input'), { target: { value: '[1]' } });
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    expect(screen.getByRole('alert')).toHaveTextContent('JSON object');
    expect(writes(platform)).toHaveLength(0);
  });

  it('keeps the key of an identical retry and changes it when the request changes', async () => {
    let fail = true;
    const { user, platform } = await create({
      routes: { [`POST ${BASE}`]: () => (fail ? json(503, { code: 'UNAVAILABLE', message: 'Try again soon.', retryable: true }) : schedule()) },
    });
    await fillMinimum(user);
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    await screen.findByText(/Try again soon/);
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    await waitFor(() => expect(writes(platform)).toHaveLength(2));
    expect(keyOf(writes(platform)[0]!)).toBe(keyOf(writes(platform)[1]!));
    await user.type(screen.getByLabelText('Name'), '-2');
    fail = false;
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    await waitFor(() => expect(writes(platform)).toHaveLength(3));
    expect(keyOf(writes(platform)[2]!)).not.toBe(keyOf(writes(platform)[0]!));
  });

  it('says what the platform refused', async () => {
    const { user } = await create({ routes: { [`POST ${BASE}`]: () => failure(422, 'VALIDATION_FAILED', 'SCHEDULE_SPEC_INVALID') } });
    await fillMinimum(user);
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    expect(await screen.findByText(/Creating the schedule failed/)).toBeVisible();
    expect(screen.getByText('SCHEDULE_SPEC_INVALID', { selector: 'code' })).toBeVisible();
  });

  it.each([
    [['schedule:read', 'run:start'], 'schedule:write'],
    [['schedule:read', 'schedule:write'], 'run:start'],
  ])('is not offered with %s: it needs %s', async (permissions, missing) => {
    const { platform } = await create({ permissions });
    expect(await screen.findByText(new RegExp(missing))).toBeVisible();
    expect(screen.queryByLabelText('Name')).not.toBeInTheDocument();
    expect(platform.calls.filter((call) => call.url.includes('/schedules'))).toHaveLength(0);
  });
});

describe('Editing a schedule', () => {
  it('fills the form with the schedule, except the input, which the platform never returns', async () => {
    await edit({ routes: { [`GET ${BASE}/${ID}`]: () => described({ note: 'for the report', remainingActions: 12 }) } });
    expect(await screen.findByRole('heading', { name: 'Edit nightly' })).toBeVisible();
    expect(screen.getByLabelText('Name')).toHaveValue('nightly');
    await waitFor(() => expect(screen.getByLabelText('Workflow')).toHaveValue(WORKFLOW));
    expect(screen.getByLabelText('Input')).toHaveValue('{}');
    expect(screen.getByText(/never returns the input of a schedule/)).toBeVisible();
    expect(screen.getByLabelText('Every')).toHaveValue('2');
    expect(screen.getByLabelText('Unit')).toHaveValue('HOURS');
    expect(screen.getByLabelText('Actions left')).toHaveValue('12');
    expect(screen.getByLabelText('Note')).toHaveValue('for the report');
  });

  it('saves against the revision it was opened at, with what the form does not show kept as it was', async () => {
    const { user, platform } = await edit({
      routes: {
        [`GET ${BASE}/${ID}`]: () =>
          described({
            revision: 7,
            spec: { intervals: [{ everyMs: 3_600_000, offsetMs: 900_000 }], timezone: 'UTC', jitterMs: 0 },
            policies: { overlap: 'BUFFER_ALL', catchupWindowMs: 60_000, catchupMode: 'ALL', pauseOnFailure: true, bufferLimit: 7 },
          }),
        [`PUT ${BASE}/${ID}`]: () => schedule({ revision: 8 }),
      },
    });
    await screen.findByRole('heading', { name: 'Edit nightly' });
    await waitFor(() => expect(screen.getByLabelText('Workflow')).toHaveValue(WORKFLOW));
    fireEvent.change(screen.getByLabelText('Input'), { target: { value: '{"k":1}' } });
    await user.click(screen.getByRole('button', { name: 'Save changes' }));
    await screen.findByRole('heading', { name: 'Schedule landing' });
    const [put] = writes(platform);
    expect(bodyOf(put!)).toMatchObject({
      expectedRevision: 7,
      spec: { intervals: [{ everyMs: 3_600_000, offsetMs: 900_000 }] },
      policies: { overlap: 'BUFFER_ALL', catchupWindowMs: 60_000, bufferLimit: 7, pauseOnFailure: true },
      action: { input: { k: 1 } },
    });
  });

  it('says so when somebody saved a newer revision meanwhile', async () => {
    const { user } = await edit({ routes: { [`PUT ${BASE}/${ID}`]: () => failure(409, 'CONFLICT', 'REVISION_CONFLICT') } });
    await screen.findByRole('heading', { name: 'Edit nightly' });
    await waitFor(() => expect(screen.getByLabelText('Workflow')).toHaveValue(WORKFLOW));
    await user.click(screen.getByRole('button', { name: 'Save changes' }));
    expect(await screen.findByText(/changed while you were editing/)).toBeVisible();
  });

  it('keeps asking at the revision the person opened: a second save after a conflict is refused again, never accepted at the other person\'s revision', async () => {
    let revision = 3;
    const { user, platform, app } = await edit({
      routes: {
        [`GET ${BASE}/${ID}`]: () => described({ revision }),
        [`PUT ${BASE}/${ID}`]: () => {
          // somebody else's save is what the platform answers with, and what a read of the schedule returns from now on
          revision = 4;
          return failure(409, 'CONFLICT', 'REVISION_CONFLICT');
        },
      },
    });
    await screen.findByRole('heading', { name: 'Edit nightly' });
    await waitFor(() => expect(screen.getByLabelText('Workflow')).toHaveValue(WORKFLOW));
    await user.click(screen.getByRole('button', { name: 'Save changes' }));
    expect(await screen.findByText(/changed while you were editing/)).toBeVisible();
    // whatever the page reads again after the conflict has been read by now
    await waitFor(() => expect(app.queries.isFetching()).toBe(0));
    await user.click(screen.getByRole('button', { name: 'Save changes' }));
    await waitFor(() => expect(writes(platform)).toHaveLength(2));
    expect(writes(platform).map((call) => bodyOf(call)['expectedRevision'])).toEqual([3, 3]);
    // and the copy the form holds is still the one it was opened with, not the one the other person saved
    expect(app.queries.getQueryData<{ revision: number }>(['schedule', NS, ID, 'form'])?.revision).toBe(3);
  });

  it('marks the page of the schedule as out of date when the revision is stale, so going back to it shows what the other person saved', async () => {
    const { user, app } = await edit({ routes: { [`PUT ${BASE}/${ID}`]: () => failure(409, 'CONFLICT', 'REVISION_CONFLICT') } });
    // what the page of the schedule read when it was opened: fresh for five seconds, and the platform has changed it since
    app.queries.setQueryData(['schedule', NS, ID], described({ revision: 3 }));
    await screen.findByRole('heading', { name: 'Edit nightly' });
    await waitFor(() => expect(screen.getByLabelText('Workflow')).toHaveValue(WORKFLOW));
    expect(app.queries.getQueryState(['schedule', NS, ID])?.isInvalidated).toBe(false);
    await user.click(screen.getByRole('button', { name: 'Save changes' }));
    expect(await screen.findByText(/changed while you were editing/)).toBeVisible();
    await waitFor(() => expect(app.queries.getQueryState(['schedule', NS, ID])?.isInvalidated).toBe(true));
  });

  it('is a fresh copy each time: what somebody saved meanwhile is what the next edit starts from', async () => {
    let name = 'nightly';
    let revision = 3;
    const { user } = await edit({ routes: { [`GET ${BASE}/${ID}`]: () => described({ name, revision }) } });
    expect(await screen.findByRole('heading', { name: 'Edit nightly' })).toBeVisible();
    await user.click(screen.getByRole('link', { name: 'Back to the schedule' }));
    await screen.findByRole('heading', { name: 'Schedule landing' });
    name = 'nightly-renamed';
    revision = 4;
    await user.click(screen.getByRole('link', { name: 'Edit again' }));
    expect(await screen.findByRole('heading', { name: 'Edit nightly-renamed' })).toBeVisible();
  });

  it('does not form what it cannot write: calendars, exclusions, several rules', async () => {
    await edit({ routes: { [`GET ${BASE}/${ID}`]: () => described({ spec: { calendars: [{ hour: [{ start: 3 }] }], timezone: 'UTC', jitterMs: 0 } }) } });
    expect(await screen.findByText(/calendars, exclusions or several interval rules/)).toBeVisible();
    expect(screen.queryByLabelText('Name')).not.toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Back to the schedule' })).toHaveAttribute('href', `/schedules/${ID}`);
  });

  it('does not edit a schedule a cron trigger owns', async () => {
    await edit({ routes: { [`GET ${BASE}/${ID}`]: () => described({ origin: 'TRIGGER' }) } });
    expect(await screen.findByText(/cron trigger owns this schedule/)).toBeVisible();
    expect(screen.queryByLabelText('Name')).not.toBeInTheDocument();
  });

  it('says so when the schedule is not there', async () => {
    await edit({ routes: { [`GET ${BASE}/${ID}`]: () => failure(404, 'NOT_FOUND') } });
    expect(await screen.findByText(/Loading the schedule failed/)).toBeVisible();
  });
});
