import { expect, test } from '@playwright/test';
import { env } from '../support/env';
import { expectAccessible } from '../support/accessibility';
import { activate, choose, closeWithEscape, inspectDialog, openDialog, openFromMenu, reach, signInByKeyboard, typeInto, walk } from '../support/keyboard';
import { Api } from '../support/platform';
import { failOnBrowserErrors } from '../support/studio';
import { endWorkflow, namespaceIdOf, ownNamespace, setQuota, recordRefusals, unique, withoutRefusals } from '../support/world';

/**
 * The schedules of the Studio in a real browser, with the keyboard alone and the accessibility rules on every state (T018-074,
 * FR-018-047, SC-018-015). What a person does is done with Tab and Enter; the platform is asked afterwards, over the REST API,
 * whether the Studio did what it said.
 */
test.use({ viewport: { width: 1470, height: 900 } });

interface ScheduleBody {
  scheduleId: string;
  state: string;
  revision: number;
}

test('SCHEDULES a person creates a schedule, finds it, pauses, resumes, runs it now, reads what it decided and deletes it, with the keyboard alone', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const api = new Api('admin');
  const target = unique('sched-target');
  await endWorkflow(api, target);
  const namespaceId = await namespaceIdOf(api, env.namespace);
  const name = unique('nightly-report');

  await signInByKeyboard(page, env.tokens.admin, env.namespace);
  await openFromMenu(page, 'Schedules', 'Schedules');
  await expectAccessible(page, 'the schedules list');

  // Empty state: a name filter nothing matches (the keyboard types into the field, the list asks once typing rests).
  const filter = page.getByLabel('Name starts with');
  await typeInto(page, filter, name);
  await expect(page.getByText('No schedule matches these filters.')).toBeVisible();
  await expectAccessible(page, 'the schedules list, nothing matches');
  await typeInto(page, filter, '');

  // The form: empty, refused in words, then filled.
  await activate(page, page.getByRole('link', { name: 'New schedule' }));
  await expect(page.getByRole('heading', { level: 1, name: 'New schedule' })).toBeVisible();
  await expectAccessible(page, 'the schedule form');
  await activate(page, page.getByRole('button', { name: 'Create schedule' }));
  await expect(page.getByRole('alert').filter({ hasText: 'Give the schedule a name of 1 to 128 characters.' })).toBeVisible();
  await expectAccessible(page, 'the schedule form, refused');

  await typeInto(page, page.getByLabel('Name', { exact: true }), name);
  await choose(page, page.getByLabel('Workflow', { exact: true }), target);
  await typeInto(page, page.getByLabel('Input', { exact: true }), '{"region":"eu"}');
  await typeInto(page, page.getByLabel('Every', { exact: true }), '90');
  await choose(page, page.getByLabel('Unit', { exact: true }), 'minutes');
  // Every named control of the form is reached, in an order that follows the page, and none needs a pointer.
  const order = await walk(page);
  expect(order).toEqual(expect.arrayContaining(['textbox: Name', 'combobox: Workflow', 'textbox: Input', 'textbox: Every', 'combobox: Unit', 'button: Create schedule']));
  await reach(page, page.getByLabel('Name', { exact: true }));
  await activate(page, page.getByRole('button', { name: 'Create schedule' }));

  // The page of the schedule: what the platform decided, the times it computed (the Studio computes none).
  await expect(page.getByRole('heading', { level: 1, name: new RegExp(`^${name}`) })).toBeVisible();
  const scheduleId = new URL(page.url()).pathname.split('/').at(-1)!;
  await expect(page.getByRole('heading', { level: 2, name: 'Next times' })).toBeVisible();
  await expect(page.getByRole('main').locator('ol > li')).toHaveCount(10);
  await expect(page.getByText('every 90 minutes (UTC)').first()).toBeVisible();
  await expect(page.getByRole('main').getByText('alice', { exact: true })).toBeVisible();
  await expectAccessible(page, 'the schedule');
  const read = async () => (await api.call<ScheduleBody>('GET', `/namespaces/${namespaceId}/schedules/${scheduleId}`, undefined, 200)).body;
  expect((await read()).state).toBe('ACTIVE');

  // It is in the list, and the list opens it.
  await openFromMenu(page, 'Schedules', 'Schedules');
  await typeInto(page, page.getByLabel('Name starts with'), name);
  const row = page.getByRole('row').filter({ hasText: name });
  await expect(row).toHaveCount(1);
  await expect(row).toContainText('Active');
  await expect(row).toContainText('every 90 minutes (UTC)');
  await expectAccessible(page, 'the schedules list, one schedule');
  await activate(page, row.getByRole('link', { name }));
  await expect(page.getByRole('heading', { level: 1, name: new RegExp(`^${name}`) })).toBeVisible();

  // Pause: the dialog takes focus, asks why, refuses an empty answer in words, and Escape gives focus back.
  const pause = page.getByRole('button', { name: 'Pause', exact: true });
  const pauseDialog = await openDialog(page, pause, 'Pause the schedule');
  await expect(pauseDialog.getByLabel('Why')).toBeFocused();
  await expectAccessible(page, 'the pause dialog');
  const named = await inspectDialog(page, pauseDialog);
  expect(named).toEqual(expect.arrayContaining(['textbox: Why', 'button: Confirm pause', 'button: Close']));
  await activate(page, pauseDialog.getByRole('button', { name: 'Confirm pause' }));
  await expect(pauseDialog.getByRole('alert')).toContainText('Say why, in a few words.');
  await expectAccessible(page, 'the pause dialog, refused');
  await closeWithEscape(page, pauseDialog, pause);
  await openDialog(page, pause, 'Pause the schedule');
  await typeInto(page, pauseDialog.getByLabel('Why'), 'the nightly report is rebuilt by hand this week');
  await reach(page, pauseDialog.getByRole('button', { name: 'Confirm pause' }));
  await page.keyboard.press('Enter');
  await expect(pauseDialog).toBeHidden();
  await expect(page.getByRole('heading', { level: 1 })).toContainText('Paused');
  await expect(page.getByText('the nightly report is rebuilt by hand this week')).toBeVisible();
  await expect(page.getByText('None: it is paused.')).toBeVisible();
  expect((await read()).state).toBe('PAUSED');
  // The button that opened the dialog is gone (a paused schedule is resumed, not paused): focus is not lost to the top of the
  // page but goes to the control that takes its place.
  await expect(page.getByRole('button', { name: 'Resume', exact: true })).toBeFocused();
  await expectAccessible(page, 'the schedule, paused');

  // Resume.
  const resumeDialog = await openDialog(page, page.getByRole('button', { name: 'Resume', exact: true }), 'Resume the schedule');
  expect(await inspectDialog(page, resumeDialog)).toEqual(expect.arrayContaining(['textbox: Why', 'button: Confirm resume', 'button: Close']));
  await typeInto(page, resumeDialog.getByLabel('Why'), 'the report is automatic again');
  await activate(page, resumeDialog.getByRole('button', { name: 'Confirm resume' }));
  await expect(resumeDialog).toBeHidden();
  await expect(page.getByRole('heading', { level: 1 })).toContainText('Active');
  await expect(page.getByRole('main').locator('ol > li')).toHaveCount(10);
  expect((await read()).state).toBe('ACTIVE');
  await expect(page.getByRole('button', { name: 'Pause', exact: true })).toBeFocused();

  // Run now: one time is decided at once, and the answer is in words with a way to the run it made.
  const triggerDialog = await openDialog(page, page.getByRole('button', { name: 'Run now', exact: true }), 'Run the schedule now');
  expect(await inspectDialog(page, triggerDialog)).toEqual(expect.arrayContaining(['combobox: If a run is still open', 'button: Run now', 'button: Close']));
  await expectAccessible(page, 'the run-now dialog');
  await activate(page, triggerDialog.getByRole('button', { name: 'Run now', exact: true }));
  await expect(triggerDialog.getByRole('status')).toContainText('Started a run.');
  await expectAccessible(page, 'the run-now dialog, answered');
  await activate(page, triggerDialog.getByRole('button', { name: 'Close' }));
  await expect(triggerDialog).toBeHidden();
  // the button that opened it is still there, and holds focus again
  await expect(page.getByRole('button', { name: 'Run now', exact: true })).toBeFocused();

  // The history: the decision, its kind and its run.
  const history = page.getByRole('table', { name: 'The decisions of the schedule, newest first' });
  await expect(history.getByRole('row').nth(1)).toContainText('Run now');
  await expect(history.getByRole('row').nth(1)).toContainText('Started a run');
  await expectAccessible(page, 'the schedule, with its history');
  const started = (await api.call<{ items: Array<{ runId?: string | null }> }>('GET', `/namespaces/${namespaceId}/schedules/${scheduleId}/actions`, undefined, 200)).body.items[0]?.runId;
  expect(started).toBeTruthy();
  await activate(page, history.getByRole('link', { name: started!.slice(-8) }));
  await expect(page.getByRole('heading', { level: 1 })).toContainText(started!);
  await api.until(started!, 'SUCCEEDED');
  await page.goBack();
  await expect(page.getByRole('heading', { level: 1, name: new RegExp(`^${name}`) })).toBeVisible();

  // Delete: the dialog says what happens, and the schedule is gone from the platform and from the list.
  const deleteDialog = await openDialog(page, page.getByRole('button', { name: 'Delete', exact: true }), 'Delete the schedule');
  await expect(deleteDialog).toContainText('will stop acting');
  expect(await inspectDialog(page, deleteDialog)).toEqual(expect.arrayContaining(['button: Confirm deletion', 'button: Close']));
  await expectAccessible(page, 'the delete dialog');
  await activate(page, deleteDialog.getByRole('button', { name: 'Confirm deletion' }));
  await expect(page.getByRole('heading', { level: 1, name: 'Schedules' })).toBeVisible();
  expect((await api.call('GET', `/namespaces/${namespaceId}/schedules/${scheduleId}`)).status).toBe(404);
  await typeInto(page, page.getByLabel('Name starts with'), name);
  await expect(page.getByText('No schedule matches these filters.')).toBeVisible();
  expect(problems).toEqual([]);
});

test('SCHEDULES a refusal of the platform is read in words: a name in use, a quota of zero, a stale revision', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const refusals = recordRefusals(page);
  const admin = new Api('admin');
  const own = await ownNamespace(admin, 'sched-refused');
  const api = new Api('admin', own.slug);
  const target = unique('sched-target');
  const workflow = await endWorkflow(api, target);
  const dup = unique('already-here');
  const base = `/namespaces/${own.id}/schedules`;

  // A namespace nobody has made a schedule in: the empty state of the list, in words.
  await signInByKeyboard(page, env.tokens.admin, own.slug);
  await openFromMenu(page, 'Schedules', 'Schedules');
  await expect(page.getByText('No schedule yet.')).toBeVisible();
  await expectAccessible(page, 'the schedules list, empty');

  // This one repeats by cron expressions in a time zone (the other journey's by an interval).
  const create = async (name: string) => {
    await activate(page, page.getByRole('link', { name: 'New schedule' }));
    await expect(page.getByRole('heading', { level: 1, name: 'New schedule' })).toBeVisible();
    await typeInto(page, page.getByLabel('Name', { exact: true }), name);
    await choose(page, page.getByLabel('Workflow', { exact: true }), target);
    await choose(page, page.getByLabel('Repeat by'), 'Cron expressions');
    await typeInto(page, page.getByLabel('Cron expressions', { exact: true }), '0 9 * * 1-5');
    await typeInto(page, page.getByLabel('Time zone'), 'Europe/Lisbon');
    await activate(page, page.getByRole('button', { name: 'Create schedule' }));
  };
  await create(dup);
  await expect(page.getByRole('heading', { level: 1, name: new RegExp(`^${dup}`) })).toBeVisible();
  const scheduleId = new URL(page.url()).pathname.split('/').at(-1)!;
  const made = await api.call<ScheduleBody>('GET', `${base}/${scheduleId}`, undefined, 200);

  // A name in use: the platform's sentence, its code, and the form still holds what was typed.
  await openFromMenu(page, 'Schedules', 'Schedules');
  await expect(page.getByRole('row').filter({ hasText: dup })).toHaveCount(1);
  await create(dup);
  const refusal = page.getByRole('alert').filter({ hasText: 'Creating the schedule failed.' });
  await expect(refusal).toBeVisible();
  await expect(refusal).toContainText('CONFLICT');
  await expect(page.getByLabel('Name', { exact: true })).toHaveValue(dup);
  await expectAccessible(page, 'the schedule form, a name in use');

  // A quota of zero: the sentence names the ceiling and what to do about it.
  await setQuota(admin, own.id, 'schedules', 0);
  await typeInto(page, page.getByLabel('Name', { exact: true }), unique('one-too-many'));
  await activate(page, page.getByRole('button', { name: 'Create schedule' }));
  await expect(refusal).toContainText('QUOTA_EXCEEDED');
  await expect(refusal).toContainText('SCHEDULE_LIMIT');
  await expect(refusal).toContainText('ceiling is reached');
  await expectAccessible(page, 'the schedule form, over the quota');

  // A stale revision: another session changes the schedule while this form is open.
  await openFromMenu(page, 'Schedules', 'Schedules');
  await activate(page, page.getByRole('row').filter({ hasText: dup }).getByRole('link', { name: dup }));
  await expect(page.getByRole('heading', { level: 1, name: new RegExp(`^${dup}`) })).toBeVisible();
  await activate(page, page.getByRole('link', { name: 'Edit', exact: true }));
  await expect(page.getByRole('heading', { level: 1, name: `Edit ${dup}` })).toBeVisible();
  await expect(page.getByLabel('Cron expressions', { exact: true })).toHaveValue('0 9 * * 1-5');
  await expect(page.getByLabel('Time zone')).toHaveValue('Europe/Lisbon');
  const theirs = await api.call<ScheduleBody>('PUT', `${base}/${made.body.scheduleId}`, {
    name: dup, spec: { cron: ['0 8 * * *'], timezone: 'Europe/Lisbon' }, action: { workflowId: workflow.id, input: {} }, expectedRevision: made.body.revision,
  }, 200);
  expect(theirs.body.revision).toBe(made.body.revision + 1);
  await typeInto(page, page.getByLabel('Cron expressions', { exact: true }), '0 10 * * 1-5');
  await activate(page, page.getByRole('button', { name: 'Save changes' }));
  const stale = page.getByRole('alert').filter({ hasText: 'The schedule changed while you were editing.' });
  await expect(stale).toBeVisible();
  await expect(page.getByRole('alert').filter({ hasText: 'REVISION_CONFLICT' })).toBeVisible();
  await expectAccessible(page, 'the schedule form, a stale revision');
  // Pressing Save again does not get past the conflict: the form still asks at the revision it was opened at, so the platform
  // refuses again and the other session's change is still there (it is never accepted at the revision the platform now has).
  const savedAgain = page.waitForResponse((response) => response.request().method() === 'PUT' && response.url().endsWith(`/schedules/${made.body.scheduleId}`));
  await activate(page, page.getByRole('button', { name: 'Save changes' }));
  const again = await savedAgain;
  expect(again.status()).toBe(409);
  expect((again.request().postDataJSON() as { expectedRevision: number }).expectedRevision).toBe(made.body.revision);
  await expect(stale).toBeVisible();
  expect((await api.call<ScheduleBody>('GET', `${base}/${made.body.scheduleId}`, undefined, 200)).body.revision).toBe(theirs.body.revision);
  // The way out is a link back to the schedule, which shows what the other session saved.
  await activate(page, page.getByRole('link', { name: 'Back to the schedule' }));
  await expect(page.getByRole('heading', { level: 1, name: new RegExp(`^${dup}`) })).toBeVisible();
  await expect(page.getByText('cron 0 8 * * * (Europe/Lisbon)').first()).toBeVisible();
  // and the form, opened again, starts from that revision, not from the one the person had
  await activate(page, page.getByRole('link', { name: 'Edit', exact: true }));
  await expect(page.getByLabel('Cron expressions', { exact: true })).toHaveValue('0 8 * * *');

  expect(withoutRefusals(problems, refusals, [409, 429])).toEqual([]);
});

test('SCHEDULES a schedule a cron trigger owns shows its controls off, with the reason, until the operator hands the triggers over', async ({ page }) => {
  test.setTimeout(180_000);
  const problems = failOnBrowserErrors(page);
  const admin = new Api('admin');
  const own = await ownNamespace(admin, 'sched-trigger');
  const api = new Api('admin', own.slug);
  // Publishing a workflow with a cron trigger makes the schedule that owns it; the e2e platform has not handed the triggers over.
  const cron = unique('cron-owner');
  await endWorkflow(api, cron, { triggers: [{ kind: 'cron', cron: '0 0 1 1 *', timezone: 'UTC' }] });

  await signInByKeyboard(page, env.tokens.admin, own.slug);
  await openFromMenu(page, 'Schedules', 'Schedules');
  const owned = page.getByRole('row').filter({ hasText: 'cron trigger' });
  await expect(owned).toHaveCount(1);
  await expectAccessible(page, 'the schedules list, with a schedule of a cron trigger');
  await activate(page, owned.getByRole('link').first());
  await expect(page.getByRole('heading', { level: 1 })).toContainText('Active');
  const controls = page.getByRole('group', { name: 'Actions on the schedule' });
  await expect(controls.getByRole('button', { name: 'Pause' })).toBeDisabled();
  await expect(controls.getByRole('button', { name: 'Run now' })).toBeDisabled();
  await expect(controls.getByRole('note')).toContainText('an operator has not handed the cron triggers over');
  await expect(controls.getByRole('link', { name: 'Edit' })).toHaveCount(0);
  await expect(controls.getByRole('button', { name: 'Delete' })).toHaveCount(0);
  await expect(page.getByText('A cron trigger owns this schedule')).toBeVisible();
  await expectAccessible(page, 'the schedule of a cron trigger');
  expect(problems).toEqual([]);
});

test('SCHEDULES a backfill acts on the times of a past range: asked for in a dialog, followed to its end in words, its decisions listed; asked again, it finds every time already acted on', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const admin = new Api('admin');
  const own = await ownNamespace(admin, 'sched-backfill');
  const api = new Api('admin', own.slug);
  const target = await endWorkflow(api, unique('backfill-target'));
  const name = unique('hourly');
  const base = `/namespaces/${own.id}/schedules`;
  // An hourly schedule, and the three whole hours that ended last: three times, whatever the minute the journey runs in. (The Studio
  // has no preview of how many times a range holds, G1966, so there is none to look at before asking.)
  await api.call('POST', base, { name, spec: { intervals: [{ everyMs: 3_600_000 }], timezone: 'UTC' }, action: { workflowId: target.id, input: {} } }, 201);
  const hour = 3_600_000;
  const end = new Date(Math.floor(Date.now() / hour) * hour);
  const start = new Date(end.getTime() - 3 * hour);
  const iso = (at: Date) => at.toISOString().replace('.000Z', 'Z');

  await signInByKeyboard(page, env.tokens.admin, own.slug);
  await openFromMenu(page, 'Schedules', 'Schedules');
  await activate(page, page.getByRole('row').filter({ hasText: name }).getByRole('link', { name }));
  await expect(page.getByRole('heading', { level: 1, name: new RegExp(`^${name}`) })).toBeVisible();
  const backfills = page.getByRole('region', { name: 'Backfills' });
  await expect(backfills.getByText('No backfill yet.')).toBeVisible();
  await expectAccessible(page, 'the schedule, no backfill yet');

  // The dialog: what is wrong with a range is said in words, before anything is sent.
  const opener = backfills.getByRole('button', { name: 'New backfill', exact: true });
  const dialog = await openDialog(page, opener, 'Backfill a range');
  expect(await inspectDialog(page, dialog)).toEqual(expect.arrayContaining(['textbox: From', 'textbox: Up to, not including', 'combobox: If a run is still open', 'button: Start the backfill', 'button: Close']));
  await expectAccessible(page, 'the backfill dialog');
  await activate(page, dialog.getByRole('button', { name: 'Start the backfill' }));
  await expect(dialog.getByRole('alert')).toContainText('Write the start and the end as dates, such as 2026-12-01T09:00:00Z.');
  await expectAccessible(page, 'the backfill dialog, no range');
  await typeInto(page, dialog.getByLabel('From'), iso(start));
  await typeInto(page, dialog.getByLabel('Up to, not including'), iso(new Date(Date.now() + hour)));
  await activate(page, dialog.getByRole('button', { name: 'Start the backfill' }));
  await expect(dialog.getByRole('alert')).toContainText('The end cannot be in the future: a backfill covers times that have passed.');
  await closeWithEscape(page, dialog, opener);

  // The range: the backfill is accepted, goes on, and ends having started a run for each time.
  const ask = async () => {
    await openDialog(page, opener, 'Backfill a range');
    await typeInto(page, dialog.getByLabel('From'), iso(start));
    await typeInto(page, dialog.getByLabel('Up to, not including'), iso(end));
    await activate(page, dialog.getByRole('button', { name: 'Start the backfill' }));
    await expect(dialog).toBeHidden();
  };
  await ask();
  const table = backfills.getByRole('table', { name: 'The latest backfills of the schedule' });
  await expect(table.getByRole('row').filter({ hasText: '3 started a run' })).toContainText('Completed', { timeout: 60_000 });
  await expectAccessible(page, 'the schedule, a backfill that ended');
  const first = (await api.call<{ items: Array<{ state: string; counts: Record<string, number> }> }>('GET', `${base}/${new URL(page.url()).pathname.split('/').at(-1)}/backfills`, undefined, 200)).body.items;
  expect(first).toMatchObject([{ state: 'COMPLETED', counts: { STARTED: 3 } }]);

  // Its decisions are listed with it, the page having read them again when the backfill went on.
  const history = page.getByRole('table', { name: 'The decisions of the schedule, newest first' });
  const decided = history.getByRole('row').filter({ hasText: 'Backfill' }).filter({ hasText: 'Started a run' });
  await expect(decided).toHaveCount(3);
  await expectAccessible(page, 'the schedule, with the decisions of the backfill');

  // The same range again: no time is given a second run, and the backfill says so.
  await ask();
  await expect(table.getByRole('row').filter({ hasText: '3 already acted on' })).toContainText('Completed', { timeout: 60_000 });
  await expect(table.getByRole('row')).toHaveCount(3);
  await expectAccessible(page, 'the schedule, a second backfill that found every time acted on');
  expect(problems).toEqual([]);
});
