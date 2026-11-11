import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
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

describe('The business id of the runs of a schedule', () => {
  const PREFIX = 'Business id prefix';
  const OPEN_RUN = 'If an open run already has the id';
  const CLOSED_RUN = 'If a closed run still remembers the id';
  const refusedPrefix = () =>
    json(422, { code: 'VALIDATION_FAILED', message: 'The schedule is not valid.', retryable: false, details: { reason: 'SCHEDULE_SPEC_INVALID', field: '/action/businessIdPrefix' } });
  const keysOfAction = (platform: Parameters<typeof writes>[0]) => Object.keys(bodyOf(writes(platform)[0]!)['action'] as Record<string, unknown>).sort();

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('is optional: with no prefix the request has no key for it, and neither the policies nor an id are shown', async () => {
    const { user, platform } = await create({ routes: { [`POST ${BASE}`]: () => schedule() } });
    await fillMinimum(user);
    expect(screen.getByLabelText(PREFIX)).toHaveValue('');
    expect(screen.queryByLabelText(OPEN_RUN)).not.toBeInTheDocument();
    expect(screen.queryByLabelText(CLOSED_RUN)).not.toBeInTheDocument();
    expect(document.querySelector('code')).toBeNull();
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    await screen.findByRole('heading', { name: 'Schedule landing' });
    expect(keysOfAction(platform)).toEqual(['input', 'versionPolicy', 'workflowId']);
  });

  it('counts the characters, shows the policies with their defaults once a prefix is typed, and posts all three', async () => {
    const { user, platform } = await create({ routes: { [`POST ${BASE}`]: () => schedule() } });
    await fillMinimum(user);
    expect(screen.getByText(/0 of 160 characters/)).toBeVisible();
    await user.type(screen.getByLabelText(PREFIX), 'order');
    expect(screen.getByText(/5 of 160 characters/)).toBeVisible();
    // the defaults are what is chosen, and they are said
    expect(screen.getByLabelText(OPEN_RUN)).toHaveValue('FAIL');
    expect(screen.getByLabelText(CLOSED_RUN)).toHaveValue('ALLOW_DUPLICATE');
    expect(screen.getByRole('option', { name: 'Record a conflict and start nothing (default)' })).toBeInTheDocument();
    expect(screen.getByRole('option', { name: 'Start a new run (default)' })).toBeInTheDocument();
    expect(screen.getByRole('option', { name: 'Record that run and start nothing' })).toBeInTheDocument();
    expect(screen.getAllByRole('option', { name: /^Record a conflict and start nothing/ })).toHaveLength(2);
    const closed = within(screen.getByLabelText(CLOSED_RUN)).getAllByRole<HTMLOptionElement>('option');
    expect(closed.map((option) => option.value)).toEqual(['ALLOW_DUPLICATE', 'ALLOW_DUPLICATE_FAILED_ONLY', 'REJECT_DUPLICATE']);
    await user.selectOptions(screen.getByLabelText(OPEN_RUN), 'USE_EXISTING');
    await user.selectOptions(screen.getByLabelText(CLOSED_RUN), 'REJECT_DUPLICATE');
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    await screen.findByRole('heading', { name: 'Schedule landing' });
    expect(bodyOf(writes(platform)[0]!)['action']).toEqual({
      workflowId: WORKFLOW,
      versionPolicy: 'ACTIVE',
      input: {},
      businessIdPrefix: 'order',
      businessIdConflictPolicy: 'USE_EXISTING',
      businessIdReusePolicy: 'REJECT_DUPLICATE',
    });
  });

  it('writes the defaults too when the person leaves them, so that what is saved is what was shown', async () => {
    const { user, platform } = await create({ routes: { [`POST ${BASE}`]: () => schedule() } });
    await fillMinimum(user);
    await user.type(screen.getByLabelText(PREFIX), 'order-');
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    await screen.findByRole('heading', { name: 'Schedule landing' });
    expect(bodyOf(writes(platform)[0]!)['action']).toMatchObject({ businessIdPrefix: 'order-', businessIdConflictPolicy: 'FAIL', businessIdReusePolicy: 'ALLOW_DUPLICATE' });
  });

  it('previews the id of a new schedule with the clock to the second, and says the time is an example', async () => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date('2026-11-10T12:34:56.789Z'));
    const { user } = await create();
    await screen.findByRole('heading', { name: 'New schedule' });
    await user.type(screen.getByLabelText(PREFIX), 'order');
    expect(screen.getByText('order-2026-11-10T12:34:56Z', { selector: 'code' })).toBeVisible();
    expect(screen.getByText(/for example/i)).toBeVisible();
  });

  it('previews the id of an existing schedule with the next nominal time the platform gave, not the time moved by the jitter', async () => {
    const { user, platform } = await edit({
      routes: {
        [`GET ${BASE}/${ID}`]: () => described({ nextActions: [{ nominalTime: '2026-11-05T04:00:00Z', actualTime: '2026-11-05T04:00:07Z' }] }),
      },
    });
    await screen.findByRole('heading', { name: 'Edit nightly' });
    await user.type(screen.getByLabelText(PREFIX), 'nightly');
    expect(screen.getByText('nightly-2026-11-05T04:00:00Z', { selector: 'code' })).toBeVisible();
    expect(screen.getByText(/The next action would start its run/)).toBeVisible();
    expect(screen.queryByText(/for example/i)).not.toBeInTheDocument();
    // the time is the platform's: the form asks for the one next time and calculates none of its own
    expect(platform.calls.find((call) => call.url.includes(`${BASE}/${ID}?`))?.url).toContain('nextCount=1');
  });

  it('shows the shape with the clock when the schedule has no next time (it is paused)', async () => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date('2026-11-10T12:34:56.789Z'));
    const { user } = await edit({ routes: { [`GET ${BASE}/${ID}`]: () => described({ state: 'PAUSED', nextActions: [] }) } });
    await screen.findByRole('heading', { name: 'Edit nightly' });
    await user.type(screen.getByLabelText(PREFIX), 'order');
    expect(screen.getByText('order-2026-11-10T12:34:56Z', { selector: 'code' })).toBeVisible();
    expect(screen.getByText(/for example/i)).toBeVisible();
  });

  it('says in one sentence what the chosen policies do, and the sentence follows the choice', async () => {
    const { user } = await create();
    await screen.findByRole('heading', { name: 'New schedule' });
    await user.type(screen.getByLabelText(PREFIX), 'order');
    expect(screen.getByText(/^If an open run holds the id, the action records a business id conflict and starts nothing; .* the action starts a new run\.$/)).toBeVisible();
    await user.selectOptions(screen.getByLabelText(OPEN_RUN), 'USE_EXISTING');
    await user.selectOptions(screen.getByLabelText(CLOSED_RUN), 'ALLOW_DUPLICATE_FAILED_ONLY');
    expect(screen.getByText(/^If an open run holds the id, the action records that run as the answer and starts nothing; .* starts a new run unless that run succeeded/)).toBeVisible();
    expect(screen.queryByText(/the action starts a new run\.$/)).not.toBeInTheDocument();
  });

  it.each([
    ['has a space inside', 'an order', /no space/],
    ['has an accent', 'pedido-ç', /no accent/],
    ['is spaces alone, which would be dropped as no prefix', '   ', /Spaces alone/],
    ['is too long', 'a'.repeat(161), /At most 160 characters/],
  ])('refuses, on its own field and before asking the platform, a prefix that %s', async (_why, text, words) => {
    const { user, platform } = await create();
    await fillMinimum(user);
    fireEvent.change(screen.getByLabelText(PREFIX), { target: { value: text } });
    expect(screen.getByLabelText(PREFIX)).toBeInvalid();
    expect(screen.getByLabelText(PREFIX)).toHaveAccessibleDescription(words);
    // no id is previewed from what cannot be an id
    expect(document.querySelector('code')).toBeNull();
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    expect(screen.getAllByRole('alert').map((alert) => alert.textContent).join(' ')).toMatch(words);
    expect(writes(platform)).toHaveLength(0);
  });

  it('counts the characters of a prefix that is too long', async () => {
    await create();
    await screen.findByRole('heading', { name: 'New schedule' });
    fireEvent.change(screen.getByLabelText(PREFIX), { target: { value: 'a'.repeat(161) } });
    expect(screen.getByText(/161 of 160 characters/)).toBeVisible();
  });

  it('shows what the platform refused about the prefix on the field of the prefix, and takes it off when the prefix changes', async () => {
    let refuse = true;
    const { user } = await create({ routes: { [`POST ${BASE}`]: () => (refuse ? refusedPrefix() : schedule()) } });
    await fillMinimum(user);
    await user.type(screen.getByLabelText(PREFIX), 'order');
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    expect(await screen.findByText(/The platform refused this prefix/)).toBeVisible();
    expect(screen.getByLabelText(PREFIX)).toBeInvalid();
    // the generic banner stays, with the code, for whoever has to report it
    expect(screen.getByText('SCHEDULE_SPEC_INVALID', { selector: 'code' })).toBeVisible();
    await user.type(screen.getByLabelText(PREFIX), '-2');
    expect(screen.queryByText(/The platform refused this prefix/)).not.toBeInTheDocument();
    expect(screen.getByLabelText(PREFIX)).toBeValid();
    refuse = false;
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    await screen.findByRole('heading', { name: 'Schedule landing' });
  });

  it('does not put what the platform refused about another field on the prefix', async () => {
    const { user } = await create({
      routes: {
        [`POST ${BASE}`]: () => json(422, { code: 'VALIDATION_FAILED', message: 'The schedule is not valid.', retryable: false, details: { reason: 'SCHEDULE_SPEC_INVALID', field: '/spec/cron/0' } }),
      },
    });
    await fillMinimum(user);
    await user.type(screen.getByLabelText(PREFIX), 'order');
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    expect(await screen.findByText(/Creating the schedule failed/)).toBeVisible();
    expect(screen.queryByText(/The platform refused this prefix/)).not.toBeInTheDocument();
    expect(screen.getByLabelText(PREFIX)).toBeValid();
  });

  it('fills the form with the prefix and the policies of the schedule, and saves them back with the fields it does not show', async () => {
    const { user, platform } = await edit({
      routes: {
        [`GET ${BASE}/${ID}`]: () =>
          described({
            action: {
              workflowId: WORKFLOW,
              versionPolicy: 'ACTIVE',
              inputDigest: 'ab'.repeat(32),
              nominalTimeField: 'at',
              runTimeoutMs: 30_000,
              businessIdPrefix: 'order-',
              businessIdConflictPolicy: 'USE_EXISTING',
              businessIdReusePolicy: 'ALLOW_DUPLICATE_FAILED_ONLY',
            },
          }),
        [`PUT ${BASE}/${ID}`]: () => schedule({ revision: 4 }),
      },
    });
    await screen.findByRole('heading', { name: 'Edit nightly' });
    expect(screen.getByLabelText(PREFIX)).toHaveValue('order-');
    expect(screen.getByLabelText(OPEN_RUN)).toHaveValue('USE_EXISTING');
    expect(screen.getByLabelText(CLOSED_RUN)).toHaveValue('ALLOW_DUPLICATE_FAILED_ONLY');
    await waitFor(() => expect(screen.getByLabelText('Workflow')).toHaveValue(WORKFLOW));
    await user.click(screen.getByRole('button', { name: 'Save changes' }));
    await screen.findByRole('heading', { name: 'Schedule landing' });
    expect(bodyOf(writes(platform)[0]!)['action']).toEqual({
      workflowId: WORKFLOW,
      versionPolicy: 'ACTIVE',
      input: {},
      nominalTimeField: 'at',
      runTimeoutMs: 30_000,
      businessIdPrefix: 'order-',
      businessIdConflictPolicy: 'USE_EXISTING',
      businessIdReusePolicy: 'ALLOW_DUPLICATE_FAILED_ONLY',
    });
  });

  it('takes the prefix off a schedule when the field is emptied: nothing is written, not even an empty string', async () => {
    const { user, platform } = await edit({
      routes: {
        [`GET ${BASE}/${ID}`]: () => described({ action: { workflowId: WORKFLOW, versionPolicy: 'ACTIVE', businessIdPrefix: 'order-', businessIdConflictPolicy: 'USE_EXISTING', businessIdReusePolicy: 'REJECT_DUPLICATE' } }),
        [`PUT ${BASE}/${ID}`]: () => schedule({ revision: 4 }),
      },
    });
    await screen.findByRole('heading', { name: 'Edit nightly' });
    await waitFor(() => expect(screen.getByLabelText('Workflow')).toHaveValue(WORKFLOW));
    await user.clear(screen.getByLabelText(PREFIX));
    expect(screen.queryByLabelText(OPEN_RUN)).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Save changes' }));
    await screen.findByRole('heading', { name: 'Schedule landing' });
    expect(keysOfAction(platform)).toEqual(['input', 'versionPolicy', 'workflowId']);
  });

  it('is customer text: it is neither logged nor made into markup', async () => {
    const logged = (['log', 'info', 'warn', 'error', 'debug'] as const).map((method) => vi.spyOn(console, method).mockImplementation(() => undefined));
    const { user } = await create({ routes: { [`POST ${BASE}`]: () => refusedPrefix() } });
    await fillMinimum(user);
    fireEvent.change(screen.getByLabelText(PREFIX), { target: { value: '<img/src=x/onerror=alert(1)>' } });
    expect(document.querySelector('img')).toBeNull();
    expect(screen.getByText(/^<img\/src=x\/onerror=alert\(1\)>-\d{4}-/, { selector: 'code' })).toBeVisible();
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    await screen.findByText(/The platform refused this prefix/);
    expect(logged.flatMap((spy) => spy.mock.calls).map((call) => JSON.stringify(call)).join('\n')).not.toContain('onerror');
    expect(document.querySelector('img')).toBeNull();
  });
});
