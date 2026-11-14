import { expect, test } from '@playwright/test';
import type { Locator, Page } from '@playwright/test';
import { env } from '../support/env';
import { expectAccessible } from '../support/accessibility';
import { activate, choose, closeWithEscape, inspectDialog, openDialog, openFromMenu, reach, signInByKeyboard, typeInto } from '../support/keyboard';
import { Api, IR_BASE } from '../support/platform';
import { failOnBrowserErrors } from '../support/studio';
import { ActivityFailure, startWorker } from '../support/worker';
import { cancelAfterwards, cancelLeftovers, namespaceIdOf, openWorkflow, ownNamespace, recordRefusals, setQuota, unique, withoutRefusals } from '../support/world';

/**
 * The batch operations of the Studio in a real browser, with the keyboard alone and the accessibility rules on every state
 * (T018-074, FR-018-047, SC-018-015). A batch job is made from the filters of the run list (the status, the workflow, the
 * origin and the start of the business id): there is no way to tick runs one by one, so the journeys choose a workflow of their own and every run of it is
 * what the job takes. The engine's batch pass is on in this platform (`SAGAT_BATCHES_ENABLED`, tests/e2e/workflow-studio).
 */
test.use({ viewport: { width: 1470, height: 900 } });

interface Job {
  batchId: string;
  state: string;
  counts: { selected: number; succeeded: number; skipped: number; failed: number; cancelled: number; deferred: number };
}

/** Starts `count` runs of a workflow that never ends on its own (nobody serves its step). */
async function openRuns(api: Api, workflowId: string, count: number): Promise<string[]> {
  const runs: string[] = [];
  for (let index = 0; index < count; index += 1) runs.push(await api.start(workflowId, { n: index }));
  cancelAfterwards(api, ...runs);
  return runs;
}

test.afterEach(cancelLeftovers);

/** The run list of the namespace, narrowed to one workflow by the keyboard: the runs the job will take are the rows on the screen. */
async function runsOf(page: Page, workflow: string, rows: number): Promise<void> {
  await openFromMenu(page, 'Runs', 'Runs');
  await choose(page, page.getByLabel('Workflow', { exact: true }), workflow);
  if (rows === 0) await expect(page.getByText('No run matches these filters.')).toBeVisible();
  else await expect(page.getByRole('table', { name: 'Runs, newest first' }).locator('tbody tr[data-index]')).toHaveCount(rows);
}

/** Opens the dialog of a batch job from the run list. */
async function openBatchDialog(page: Page): Promise<Locator> {
  return openDialog(page, page.getByRole('button', { name: 'Batch operation…' }), 'Batch operation');
}

const jobIdOf = (page: Page) => new URL(page.url()).pathname.split('/').at(-1)!;

test('BATCH a person selects runs by the filters of the list, counts them, starts a job, follows it to its end and reads what became of each run, with the keyboard alone', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const api = new Api('admin');
  const namespaceId = await namespaceIdOf(api, env.namespace);
  const name = unique('batch-cancel');
  const workflow = await openWorkflow(api, name);
  const runs = await openRuns(api, workflow.id, 3);

  await signInByKeyboard(page, env.tokens.admin, env.namespace);
  await runsOf(page, name, 3);
  await expectAccessible(page, 'the run list, narrowed to one workflow');

  // The dialog: focus goes in, the page behind is not for the keyboard, nothing is sent before the person has counted.
  const opener = page.getByRole('button', { name: 'Batch operation…' });
  const dialog = await openBatchDialog(page);
  await expect(dialog.getByLabel('Action')).toBeFocused();
  await expectAccessible(page, 'the batch dialog');
  await activate(page, dialog.getByRole('button', { name: 'Start the batch job' }));
  await expect(dialog.getByRole('alert')).toContainText('Count the runs first, so you know how many the job will take.');
  await expectAccessible(page, 'the batch dialog, not counted');
  await closeWithEscape(page, dialog, opener);

  // Count: the number and a sample, in words, with a link to a run of the sample.
  await openBatchDialog(page);
  await choose(page, dialog.getByLabel('Action'), 'Cancel');
  await activate(page, dialog.getByRole('button', { name: 'Count the runs' }));
  const counted = dialog.getByRole('status').filter({ hasText: 'runs match' });
  await expect(counted).toContainText(/^3 runs match\./);
  await expect(counted).toContainText('For example');
  await expect(counted.getByRole('link')).toHaveCount(3);
  await expectAccessible(page, 'the batch dialog, counted');

  // A reason that is too short is refused in words; a good one is kept with the job.
  await typeInto(page, dialog.getByLabel('Reason'), 'no');
  await activate(page, dialog.getByRole('button', { name: 'Start the batch job' }));
  await expect(dialog.getByRole('alert')).toContainText('Give a reason of at least 8 characters.');
  await expectAccessible(page, 'the batch dialog, reason too short');
  await typeInto(page, dialog.getByLabel('Reason'), 'the payment provider is down, stop what is in flight');
  const order = await inspectDialog(page, dialog);
  expect(order).toEqual(expect.arrayContaining(['combobox: Action', 'button: Count the runs', 'textbox: Reason', 'button: Start the batch job', 'button: Close']));
  await activate(page, dialog.getByRole('button', { name: 'Start the batch job' }));

  // The job: its page, followed while it works, ends in a state said in words and shape, and every run has its outcome.
  await expect(page.getByRole('heading', { level: 1, name: /^Batch job: Cancel/ })).toBeVisible();
  const batchId = jobIdOf(page);
  await expect(page.getByRole('heading', { level: 1 })).toContainText('Completed', { timeout: 60_000 });
  const job = (await api.call<Job>('GET', `/namespaces/${namespaceId}/batch-operations/${batchId}`, undefined, 200)).body;
  expect(job.counts).toMatchObject({ selected: 3, succeeded: 3, failed: 0, skipped: 0 });
  await expect(page.getByText('3 of 3 runs have an outcome (100%).')).toBeVisible();
  const counters = page.locator('dl.batch-counts');
  await expect(counters.locator('div').filter({ hasText: 'Succeeded' }).locator('dd')).toHaveText('3');
  await expect(counters.locator('div').filter({ hasText: 'Failed' }).locator('dd')).toHaveText('0');
  await expect(page.getByRole('button', { name: 'Cancel the job' })).toHaveCount(0);
  const items = page.getByRole('table', { name: 'The runs of the job and what became of each' });
  await expect(items.locator('tbody tr')).toHaveCount(3);
  await expect(items.getByText('Succeeded', { exact: true })).toHaveCount(3);
  for (const run of runs) await expect(items.getByRole('link', { name: run.slice(-8) })).toBeVisible();
  await expectAccessible(page, 'the batch job, completed');
  for (const run of runs) await api.until(run, 'CANCELLED');

  // The outcome filter: a state nothing is in is an empty state, in words.
  await choose(page, page.getByLabel('Outcome', { exact: true }), 'Failed');
  await expect(page.getByText('No run with this outcome.')).toBeVisible();
  await expectAccessible(page, 'the batch job, no run with the outcome');
  await choose(page, page.getByLabel('Outcome', { exact: true }), 'All outcomes');
  await expect(items.locator('tbody tr')).toHaveCount(3);

  // The list of the jobs of the namespace has it.
  await openFromMenu(page, 'Batches', 'Batch operations');
  const row = page.getByRole('row').filter({ hasText: batchId.slice(-8) });
  await expect(row).toContainText('Completed');
  await expect(row).toContainText('3 of 3');
  await expectAccessible(page, 'the batch jobs');
  await activate(page, row.getByRole('link', { name: batchId.slice(-8) }));
  await expect(page.getByRole('heading', { level: 1, name: /^Batch job: Cancel/ })).toBeVisible();
  expect(problems).toEqual([]);
});

test('BATCH a job made from the business id filter of the list takes exactly the runs started under that prefix, as the count said before it started (FR-018-016, G2027)', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const api = new Api('admin');
  const namespaceId = await namespaceIdOf(api, env.namespace);
  const workflow = await openWorkflow(api, unique('batch-bid'));
  // two runs under one prefix and one under another, all open: the prefix is unique to this journey, so no other journey's runs match
  const prefix = unique('lot');
  const inside = [await api.start(workflow.id, {}, { businessId: `${prefix}-a` }), await api.start(workflow.id, {}, { businessId: `${prefix}-b` })];
  const outside = await api.start(workflow.id, {}, { businessId: `${unique('lot')}-a` });
  cancelAfterwards(api, ...inside, outside);

  await signInByKeyboard(page, env.tokens.admin, env.namespace);
  await openFromMenu(page, 'Runs', 'Runs');
  await typeInto(page, page.getByLabel('Business id starts with'), `${prefix}-`);
  const rows = page.getByRole('table', { name: 'Runs, newest first' }).locator('tbody tr[data-index]');
  await expect(rows).toHaveCount(2);
  await expectAccessible(page, 'the run list, filtered by business id');

  // The dialog says which runs it takes, and the count is made with the prefix: two runs, the two of the list.
  const dialog = await openBatchDialog(page);
  await expect(dialog).toContainText(`the runs with business id starting with "${prefix}-"`);
  await activate(page, dialog.getByRole('button', { name: 'Count the runs' }));
  const counted = dialog.getByRole('status').filter({ hasText: 'runs match' });
  await expect(counted).toContainText(/^2 runs match\./);
  for (const run of inside) await expect(counted.getByRole('link', { name: run.slice(-8) })).toBeVisible();
  await expectAccessible(page, 'the batch dialog, counted by business id prefix');
  await typeInto(page, dialog.getByLabel('Reason'), 'cancel the lot that was ordered twice');
  await activate(page, dialog.getByRole('button', { name: 'Start the batch job' }));

  // The job selected those two runs and no other.
  await expect(page.getByRole('heading', { level: 1, name: /^Batch job: Cancel/ })).toBeVisible();
  await expect(page.getByRole('heading', { level: 1 })).toContainText('Completed', { timeout: 60_000 });
  const job = (await api.call<Job>('GET', `/namespaces/${namespaceId}/batch-operations/${jobIdOf(page)}`, undefined, 200)).body;
  expect(job.counts).toMatchObject({ selected: 2, succeeded: 2, failed: 0, skipped: 0 });
  const items = page.getByRole('table', { name: 'The runs of the job and what became of each' });
  await expect(items.locator('tbody tr')).toHaveCount(2);
  for (const run of inside) await expect(items.getByRole('link', { name: run.slice(-8) })).toBeVisible();
  await expect(items.getByRole('link', { name: outside.slice(-8) })).toHaveCount(0);
  for (const run of inside) await api.until(run, 'CANCELLED');
  expect((await api.run(outside)).status).not.toBe('CANCELLED');
  expect(problems).toEqual([]);
});

test('BATCH a job that is still at work is cancelled by the person who started it: what was applied stays, the rest is cancelled, and the job says so', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const api = new Api('admin');
  const namespaceId = await namespaceIdOf(api, env.namespace);
  const name = unique('batch-slow');
  const workflow = await openWorkflow(api, name);
  const runs = await openRuns(api, workflow.id, 4);

  await signInByKeyboard(page, env.tokens.admin, env.namespace);
  await runsOf(page, name, 4);
  await openBatchDialog(page);
  const dialog = page.getByRole('dialog', { name: 'Batch operation' });
  await choose(page, dialog.getByLabel('Action'), 'Pause');
  // one run every fifty seconds: the job is still at work when the person cancels it, however slow the machine
  await typeInto(page, dialog.getByLabel('Runs a second'), '0.02');
  await activate(page, dialog.getByRole('button', { name: 'Count the runs' }));
  await expect(dialog.getByRole('status').filter({ hasText: 'runs match' })).toContainText(/^4 runs match\./);
  await typeInto(page, dialog.getByLabel('Reason'), 'pause them one by one while the issuer is checked');
  await activate(page, dialog.getByRole('button', { name: 'Start the batch job' }));
  await expect(page.getByRole('heading', { level: 1, name: /^Batch job: Pause/ })).toBeVisible();
  const batchId = jobIdOf(page);
  await expect(page.getByText('This page follows the job while it works.')).toBeVisible();
  await expectAccessible(page, 'the batch job, at work');

  const cancel = page.getByRole('button', { name: 'Cancel the job' });
  const cancelDialog = await openDialog(page, cancel, 'Cancel the batch job');
  await expect(cancelDialog.getByLabel('Reason')).toBeFocused();
  await expect(cancelDialog).toContainText('What was already applied stays applied');
  expect(await inspectDialog(page, cancelDialog)).toEqual(expect.arrayContaining(['textbox: Reason', 'button: Confirm cancellation', 'button: Close']));
  await expectAccessible(page, 'the cancel dialog');
  await activate(page, cancelDialog.getByRole('button', { name: 'Confirm cancellation' }));
  await expect(cancelDialog.getByRole('alert')).toContainText('Give a reason of at least 8 characters.');
  await expectAccessible(page, 'the cancel dialog, no reason');
  await closeWithEscape(page, cancelDialog, cancel);
  await openDialog(page, cancel, 'Cancel the batch job');
  await typeInto(page, cancelDialog.getByLabel('Reason'), 'the issuer is back, the rest is not needed');
  await activate(page, cancelDialog.getByRole('button', { name: 'Confirm cancellation' }));
  await expect(cancelDialog).toBeHidden();
  // The button that opened the dialog is gone (a job being cancelled cannot be cancelled again): focus is not lost to the top
  // of the page but goes to the heading of the job, which says what it is now.
  await expect(page.getByRole('heading', { level: 1 })).toBeFocused();

  await expect(page.getByRole('heading', { level: 1 })).toContainText('Cancelled', { timeout: 60_000 });
  const job = (await api.call<Job>('GET', `/namespaces/${namespaceId}/batch-operations/${batchId}`, undefined, 200)).body;
  expect(job.state).toBe('CANCELLED');
  expect(job.counts.selected).toBe(4);
  expect(job.counts.cancelled).toBeGreaterThan(0);
  expect(job.counts.succeeded + job.counts.cancelled).toBe(4);
  const counters = page.locator('dl.batch-counts');
  await expect(counters.locator('div').filter({ hasText: 'Cancelled' }).locator('dd')).toHaveText(String(job.counts.cancelled));
  await expect(counters.locator('div').filter({ hasText: 'Succeeded' }).locator('dd')).toHaveText(String(job.counts.succeeded));
  await expect(page.getByRole('button', { name: 'Cancel the job' })).toHaveCount(0);
  await expect(page.getByRole('table', { name: 'The runs of the job and what became of each' }).getByText('Cancelled', { exact: true })).toHaveCount(job.counts.cancelled);
  await expectAccessible(page, 'the batch job, cancelled');
  // What was applied stays applied: the runs the job paused are paused, the others are as they were.
  const paused = await Promise.all(runs.map(async (run) => (await api.run(run)).paused === true));
  expect(paused.filter(Boolean)).toHaveLength(job.counts.succeeded);
  expect(problems).toEqual([]);
});

test('BATCH an action that does not apply to a run skips it with the reason in words, and a refusal of the platform is read in words: no run to take, a quota of zero', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const refusals = recordRefusals(page);
  const admin = new Api('admin');
  const own = await ownNamespace(admin, 'batch-refused');
  const api = new Api('admin', own.slug);
  const name = unique('batch-skip');
  const workflow = await openWorkflow(api, name);
  await openRuns(api, workflow.id, 2);
  const nothing = unique('batch-none');
  await openWorkflow(api, nothing);

  await signInByKeyboard(page, env.tokens.admin, own.slug);
  await openFromMenu(page, 'Batches', 'Batch operations');
  await expect(page.getByText('No batch job yet.')).toBeVisible();
  await expectAccessible(page, 'the batch jobs, none yet');

  // No run matches: counted as zero, and the job is not started.
  await runsOf(page, nothing, 0);
  await expectAccessible(page, 'the run list, no run matches');
  const dialog = await openBatchDialog(page);
  await activate(page, dialog.getByRole('button', { name: 'Count the runs' }));
  await expect(dialog.getByRole('status').filter({ hasText: 'run' })).toContainText(/^0 runs match\./);
  await typeInto(page, dialog.getByLabel('Reason'), 'nothing to do here, but ask anyway');
  await activate(page, dialog.getByRole('button', { name: 'Start the batch job' }));
  await expect(dialog.getByRole('alert')).toContainText('No run matches these filters: there is nothing to do.');
  await expectAccessible(page, 'the batch dialog, no run matches');
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();

  // A rerun of runs that have not ended: every item is skipped, and the table says why.
  await runsOf(page, name, 2);
  await openBatchDialog(page);
  await choose(page, dialog.getByLabel('Action'), 'Rerun');
  // more runs match than the job may take: said in words, and counted again when the limit changes
  await typeInto(page, dialog.getByLabel('Most runs'), '1');
  await activate(page, dialog.getByRole('button', { name: 'Count the runs' }));
  await expect(dialog.getByRole('status').filter({ hasText: 'runs match' })).toContainText('More than 1 runs match; the job takes the first 1.');
  await expectAccessible(page, 'the batch dialog, more runs match than the job takes');
  await typeInto(page, dialog.getByLabel('Most runs'), '');
  await expect(dialog.getByRole('status').filter({ hasText: 'runs match' })).toHaveCount(0);
  await activate(page, dialog.getByRole('button', { name: 'Count the runs' }));
  await expect(dialog.getByRole('status').filter({ hasText: 'runs match' })).toContainText(/^2 runs match\./);
  await typeInto(page, dialog.getByLabel('Reason'), 'start them again from their input');
  await activate(page, dialog.getByRole('button', { name: 'Start the batch job' }));
  await expect(page.getByRole('heading', { level: 1, name: /^Batch job: Rerun/ })).toBeVisible();
  await expect(page.getByRole('heading', { level: 1 })).toContainText('Completed', { timeout: 60_000 });
  const items = page.getByRole('table', { name: 'The runs of the job and what became of each' });
  await expect(items.locator('tbody tr')).toHaveCount(2);
  await expect(items.getByText('Skipped', { exact: true })).toHaveCount(2);
  await expect(items.getByText('The run had not ended.')).toHaveCount(2);
  await expect(page.locator('dl.batch-counts div').filter({ hasText: 'Skipped' }).locator('dd')).toHaveText('2');
  await expectAccessible(page, 'the batch job, every run skipped with its reason');
  await choose(page, page.getByLabel('Outcome', { exact: true }), 'Skipped');
  await expect(items.locator('tbody tr')).toHaveCount(2);

  // A step that none of the runs is waiting on: what became of each run, and why, is read from the table as the platform has it.
  await runsOf(page, name, 2);
  await openBatchDialog(page);
  await choose(page, dialog.getByLabel('Action'), 'Retry a step');
  await typeInto(page, dialog.getByLabel('Step', { exact: true }), 'wait');
  await activate(page, dialog.getByRole('button', { name: 'Count the runs' }));
  await expect(dialog.getByRole('status').filter({ hasText: 'runs match' })).toContainText(/^2 runs match\./);
  await typeInto(page, dialog.getByLabel('Reason'), 'ask for another attempt of the step');
  await activate(page, dialog.getByRole('button', { name: 'Start the batch job' }));
  await expect(page.getByRole('heading', { level: 1, name: /^Batch job: Retry a step/ })).toBeVisible();
  await expect(page.getByRole('heading', { level: 1 })).toContainText('Completed', { timeout: 60_000 });
  const retried = (await api.call<{ items: Array<{ state: string; reason?: string; errorCode?: string }> }>('GET', `/namespaces/${own.id}/batch-operations/${jobIdOf(page)}/items`, undefined, 200)).body.items;
  expect(retried).toMatchObject([{ state: 'SKIPPED', reason: 'NOT_WAITING' }, { state: 'SKIPPED', reason: 'NOT_WAITING' }]);
  await expect(items.locator('tbody tr')).toHaveCount(2);
  await expect(items.getByText('No step of the run was waiting for this.')).toHaveCount(2);
  await expectAccessible(page, 'the batch job, retry of a step');

  // A quota of zero: the job is refused with the platform's sentence, and nothing was started.
  await setQuota(admin, own.id, 'batch_jobs', 0);
  await runsOf(page, name, 2);
  await openBatchDialog(page);
  await activate(page, dialog.getByRole('button', { name: 'Count the runs' }));
  await expect(dialog.getByRole('status').filter({ hasText: 'runs match' })).toContainText(/^2 runs match\./);
  await typeInto(page, dialog.getByLabel('Reason'), 'one job more than the namespace may have');
  await activate(page, dialog.getByRole('button', { name: 'Start the batch job' }));
  const refusal = dialog.getByRole('alert').filter({ hasText: 'The batch operation failed.' });
  await expect(refusal).toBeVisible();
  await expect(refusal).toContainText('QUOTA_EXCEEDED');
  await expect(refusal).toContainText('BATCH_LIMIT');
  await expect(refusal).toContainText('ceiling is reached');
  await expectAccessible(page, 'the batch dialog, over the quota');
  const jobs = (await api.call<{ items: Job[] }>('GET', `/namespaces/${own.id}/batch-operations`, undefined, 200)).body.items;
  expect(jobs).toHaveLength(2);
  await reach(page, dialog.getByRole('button', { name: 'Close' }));
  await page.keyboard.press('Enter');
  await expect(dialog).toBeHidden();
  expect(withoutRefusals(problems, refusals, [429])).toEqual([]);
});

test('BATCH runs whose step has an outcome nobody knows are not retried blindly: their items fail, and the table gives the code the platform gave', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const api = new Api('admin');
  const namespaceId = await namespaceIdOf(api, env.namespace);
  const name = unique('batch-unknown');
  const activity = `${name}.pay`;
  const workflow = await api.publish(name, {
    ...IR_BASE,
    metadata: { name },
    entrypoint: 'pay',
    triggers: [{ kind: 'manual' }],
    nodes: [{ id: 'pay', type: 'activity', activity, timeoutMs: 5000, retry: { maxAttempts: 1 }, next: 'done' }, { id: 'done', type: 'end' }],
  });
  // the provider took the request and its answer was lost: nobody knows whether the payment was made
  const host = await startWorker(`${name}-worker`, {
    [activity]: () => { throw ActivityFailure.outcomeUnknown('the provider accepted the request but its answer was lost'); },
  });
  try {
    const runs = [await api.start(workflow.id, { n: 1 }), await api.start(workflow.id, { n: 2 })];
    cancelAfterwards(api, ...runs);
    for (const run of runs) await api.until(run, 'MANUAL_REVIEW');

    await signInByKeyboard(page, env.tokens.admin, env.namespace);
    await runsOf(page, name, 2);
    const dialog = await openBatchDialog(page);
    await choose(page, dialog.getByLabel('Action'), 'Retry a step');
    await expect(dialog.getByText('Steps whose outcome is unknown are never repeated blindly: those items fail.')).toBeVisible();
    await typeInto(page, dialog.getByLabel('Step', { exact: true }), 'pay');
    await activate(page, dialog.getByRole('button', { name: 'Count the runs' }));
    await expect(dialog.getByRole('status').filter({ hasText: 'runs match' })).toContainText(/^2 runs match\./);
    await typeInto(page, dialog.getByLabel('Reason'), 'ask for another attempt of the payment');
    await activate(page, dialog.getByRole('button', { name: 'Start the batch job' }));
    await expect(page.getByRole('heading', { level: 1, name: /^Batch job: Retry a step/ })).toBeVisible();
    await expect(page.getByRole('heading', { level: 1 })).toContainText('Completed', { timeout: 60_000 });

    const batchId = jobIdOf(page);
    const job = (await api.call<Job>('GET', `/namespaces/${namespaceId}/batch-operations/${batchId}`, undefined, 200)).body;
    expect(job.counts).toMatchObject({ selected: 2, failed: 2, succeeded: 0 });
    const failed = (await api.call<{ items: Array<{ state: string; errorCode?: string | null }> }>('GET', `/namespaces/${namespaceId}/batch-operations/${batchId}/items`, undefined, 200)).body.items;
    expect(failed).toHaveLength(2);
    const code = failed[0]!.errorCode;
    expect(code).toBeTruthy();
    expect(failed.map((item) => item.errorCode)).toEqual([code, code]);
    await expect(page.locator('dl.batch-counts div').filter({ hasText: 'Failed' }).locator('dd')).toHaveText('2');
    const items = page.getByRole('table', { name: 'The runs of the job and what became of each' });
    await expect(items.locator('tbody tr')).toHaveCount(2);
    await expect(items.getByText('Failed', { exact: true })).toHaveCount(2);
    await expect(items.locator('code', { hasText: code! })).toHaveCount(2);
    await expectAccessible(page, 'the batch job, every run failed with its code');
    await choose(page, page.getByLabel('Outcome', { exact: true }), 'Failed');
    await expect(items.locator('tbody tr')).toHaveCount(2);
    await choose(page, page.getByLabel('Outcome', { exact: true }), 'Succeeded');
    await expect(page.getByText('No run with this outcome.')).toBeVisible();
    expect(problems).toEqual([]);
  } finally {
    await host.stop();
  }
});
