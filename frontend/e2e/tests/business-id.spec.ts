import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { env } from '../support/env';
import { expectAccessible } from '../support/accessibility';
import { activate, choose, closeWithEscape, inspectDialog, landedOn, openDialog, openFromMenu, signInByKeyboard, typeInto } from '../support/keyboard';
import { Api } from '../support/platform';
import { failOnBrowserErrors } from '../support/studio';
import { cancelAfterwards, cancelLeftovers, endWorkflow, openWorkflow, recordRefusals, unique, withoutRefusals } from '../support/world';

/**
 * The business identifier of a start, in the Studio in a real browser, with the keyboard alone and the accessibility rules on
 * every state (T018-074, FR-018-047, SC-018-015): the field of the start dialog, what to do when an open run already has the
 * identifier (`FAIL`, `USE_EXISTING`) or a finished one still remembers it, the sentence of each refusal, and the filter of the
 * run list by identifier.
 */
test.use({ viewport: { width: 1470, height: 900 } });
test.afterEach(cancelLeftovers);

const runIdOf = (page: Page) => new URL(page.url()).pathname.split('/').at(-1)!;

/** The start dialog of the run list, opened with the keyboard. */
async function openStart(page: Page) {
  await openFromMenu(page, 'Runs', 'Runs');
  return openDialog(page, page.getByRole('button', { name: 'Start run…', exact: true }), 'Start a run');
}

test('BUSINESS ID a run is started with its own identifier: a bad one is refused in words, an open holder is refused or opened instead, a finished one is remembered, and the list is filtered by it', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const refusals = recordRefusals(page);
  const api = new Api('admin');
  const open = unique('bid-open');
  const done = unique('bid-done');
  await openWorkflow(api, open);
  const doneWorkflow = await endWorkflow(api, done);
  const holderId = unique('order');
  const rememberedId = unique('paid');
  // a run that finished a moment ago and still remembers its identifier
  const finished = await api.start(doneWorkflow.id, {}, { businessId: rememberedId });
  await api.until(finished, 'SUCCEEDED');

  await signInByKeyboard(page, env.tokens.admin, env.namespace);
  const dialog = await openStart(page);
  await expect(dialog.getByLabel('Workflow', { exact: true })).toBeFocused();
  await expectAccessible(page, 'the start dialog');
  // the two choices about the identifier are offered once there is one
  await expect(dialog.getByLabel('If an open run already has it')).toHaveCount(0);
  await choose(page, dialog.getByLabel('Workflow', { exact: true }), open);

  // A bad identifier is refused in words, before anything is sent.
  await typeInto(page, dialog.getByLabel('Business identifier (optional)'), 'two words');
  await expect(dialog.getByLabel('If an open run already has it')).toBeVisible();
  await expect(dialog.getByLabel('If a finished run still remembers it')).toBeVisible();
  await activate(page, dialog.getByRole('button', { name: 'Start run', exact: true }));
  await expect(dialog.getByRole('alert').filter({ hasText: 'Letters, digits and punctuation only: no space, no accent, no control character.' })).toBeVisible();
  await expectAccessible(page, 'the start dialog, a bad identifier');
  await closeWithEscape(page, dialog, page.getByRole('button', { name: 'Start run…', exact: true }));

  // A good one: the run is started with it, and the page of the run says so.
  await openDialog(page, page.getByRole('button', { name: 'Start run…', exact: true }), 'Start a run');
  await choose(page, dialog.getByLabel('Workflow', { exact: true }), open);
  await typeInto(page, dialog.getByLabel('Business identifier (optional)'), holderId);
  const order = await inspectDialog(page, dialog);
  expect(order).toEqual(expect.arrayContaining(['combobox: Workflow', 'textbox: Input (JSON object)', 'textbox: Business identifier (optional)', 'combobox: If an open run already has it', 'combobox: If a finished run still remembers it', 'button: Cancel', 'button: Start run']));
  await expectAccessible(page, 'the start dialog, with an identifier');
  await activate(page, dialog.getByRole('button', { name: 'Start run', exact: true }));
  await expect(dialog).toBeHidden();
  const holder = await landedOn(page);
  cancelAfterwards(api, holder);
  await expect(page.getByRole('heading', { level: 1 })).toContainText(holder);
  await expect(page.getByText('Business id', { exact: true })).toBeVisible();
  await expect(page.getByRole('main').getByText(holderId, { exact: true })).toBeVisible();
  expect((await api.call<{ businessId?: string }>('GET', `/runs/${holder}`, undefined, 200)).body.businessId).toBe(holderId);
  await expectAccessible(page, 'the run that holds the identifier');

  // The identifier is held by an open run: the default refuses, and the sentence says which identifier and why.
  await openStart(page);
  await choose(page, dialog.getByLabel('Workflow', { exact: true }), open);
  await typeInto(page, dialog.getByLabel('Business identifier (optional)'), holderId);
  await expect(dialog.getByLabel('If an open run already has it').locator('option:checked')).toHaveText('Refuse the start');
  await activate(page, dialog.getByRole('button', { name: 'Start run', exact: true }));
  const refusal = dialog.getByRole('alert').filter({ hasText: 'Starting the run failed.' });
  await expect(refusal).toBeVisible();
  await expect(refusal).toContainText('CONFLICT');
  await expect(refusal).toContainText('BUSINESS_ID_IN_USE');
  await expectAccessible(page, 'the start dialog, the identifier is held by an open run');
  // nothing was started
  expect((await api.call<{ items: unknown[] }>('GET', `/runs?businessIdPrefix=${encodeURIComponent(holderId)}`, undefined, 200)).body.items).toHaveLength(1);

  // The same start, asking to be taken to the open run instead: no second run, and the person lands on the holder.
  await choose(page, dialog.getByLabel('If an open run already has it'), 'Open that run instead');
  await activate(page, dialog.getByRole('button', { name: 'Start run', exact: true }));
  await expect(dialog).toBeHidden();
  await expect(page.getByRole('heading', { level: 1 })).toContainText(holder);
  expect(runIdOf(page)).toBe(holder);
  expect((await api.call<{ items: unknown[] }>('GET', `/runs?businessIdPrefix=${encodeURIComponent(holderId)}`, undefined, 200)).body.items).toHaveLength(1);

  // A finished run still remembers its identifier: refused when the person said so, started again when they did not.
  await openStart(page);
  await choose(page, dialog.getByLabel('Workflow', { exact: true }), done);
  await typeInto(page, dialog.getByLabel('Business identifier (optional)'), rememberedId);
  await choose(page, dialog.getByLabel('If a finished run still remembers it'), 'Refuse the start');
  await activate(page, dialog.getByRole('button', { name: 'Start run', exact: true }));
  const remembered = dialog.getByRole('alert').filter({ hasText: 'Starting the run failed.' });
  await expect(remembered).toBeVisible();
  await expect(remembered).toContainText('BUSINESS_ID_REUSE_REJECTED');
  await expectAccessible(page, 'the start dialog, a finished run remembers the identifier');
  await choose(page, dialog.getByLabel('If a finished run still remembers it'), 'Start a new run');
  await activate(page, dialog.getByRole('button', { name: 'Start run', exact: true }));
  await expect(dialog).toBeHidden();
  const again = await landedOn(page, holder, finished);
  await api.until(again, 'SUCCEEDED');

  // The run list filtered by identifier: the runs that have it are listed with it, and a prefix nothing has is an empty state.
  await openFromMenu(page, 'Runs', 'Runs');
  const filter = page.getByLabel('Business id starts with');
  await typeInto(page, filter, holderId);
  const rows = page.getByRole('table', { name: 'Runs, newest first' }).locator('tbody tr[data-index]');
  await expect(rows).toHaveCount(1);
  await expect(rows.first()).toContainText(holder);
  await expect(rows.first()).toContainText(holderId);
  await expectAccessible(page, 'the run list, filtered by business id');
  await typeInto(page, filter, rememberedId);
  await expect(rows).toHaveCount(2);
  await typeInto(page, filter, `${holderId}-nothing-has-this`);
  await expect(page.getByText('No run matches these filters.')).toBeVisible();
  await expectAccessible(page, 'the run list, no run has the identifier');
  expect(withoutRefusals(problems, refusals, [409])).toEqual([]);
});

test('START DELAY a run started with a delay is held and says until when, and is released when the time comes', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const api = new Api('admin');
  const name = unique('delay-target');
  await endWorkflow(api, name);

  await signInByKeyboard(page, env.tokens.admin, env.namespace);
  const dialog = await openStart(page);
  await choose(page, dialog.getByLabel('Workflow', { exact: true }), name);
  // a delay that is not a number is refused in words, before anything is sent (the unit is hours until it is chosen)
  await typeInto(page, dialog.getByLabel('Start after (optional)'), 'soon');
  await activate(page, dialog.getByRole('button', { name: 'Start run', exact: true }));
  await expect(dialog.getByRole('alert').filter({ hasText: 'Enter a whole number of hours, at least 1.' })).toBeVisible();
  await expectAccessible(page, 'the start dialog, a delay that is not a number');
  // one minute is the least a start can be delayed by
  await choose(page, dialog.getByLabel('Unit of the delay'), 'minutes');
  await typeInto(page, dialog.getByLabel('Start after (optional)'), '1');
  expect(await inspectDialog(page, dialog)).toEqual(expect.arrayContaining(['textbox: Start after (optional)', 'combobox: Unit of the delay', 'button: Start run']));
  const admitted = page.waitForResponse((response) => response.request().method() === 'POST' && new URL(response.url()).pathname === '/v1/runs');
  await activate(page, dialog.getByRole('button', { name: 'Start run', exact: true }));
  const response = await admitted;
  expect(response.status()).toBe(202);
  expect((response.request().postDataJSON() as { startDelayMs?: number }).startDelayMs).toBe(60_000);
  const run = await landedOn(page);

  // Held: the run says until when its first step is released, and nothing has run.
  const held = (await api.call<{ status: string; startDelayedUntil?: string }>('GET', `/runs/${run}`, undefined, 200)).body;
  expect(held.status).toBe('WAITING');
  expect(held.startDelayedUntil).toBeTruthy();
  const starts = page.locator('dl.rn-facts div').filter({ has: page.getByText('Starts at', { exact: true }) });
  await expect(starts).toHaveCount(1);
  await expect(starts.locator('time')).toHaveAttribute('datetime', held.startDelayedUntil!);
  await expect(page.getByText('Held by its start delay: nothing runs until the entry is released.')).toBeVisible();
  await expectAccessible(page, 'the run held by its start delay');
  // and the list of runs says it of the row
  await openFromMenu(page, 'Runs', 'Runs');
  const row = page.getByRole('table', { name: 'Runs, newest first' }).locator('tbody tr[data-index]').filter({ hasText: run });
  await expect(row).toContainText('Starts ');
  await expectAccessible(page, 'the run list, a run held by its start delay');

  // Released: the entry is let go when the time comes, the run runs and ends, and the page says it is no longer held.
  await activate(page, row.getByRole('link', { name: run }));
  await expect(page.getByRole('heading', { level: 1 })).toContainText(run);
  await api.until(run, 'SUCCEEDED', 150_000);
  await expect(starts).toHaveCount(0, { timeout: 60_000 });
  await expect(page.getByRole('region', { name: 'Run detail' }).getByText('Succeeded', { exact: true }).first()).toBeVisible({ timeout: 60_000 });
  await expectAccessible(page, 'the run, released and ended');
  expect(problems).toEqual([]);
});
