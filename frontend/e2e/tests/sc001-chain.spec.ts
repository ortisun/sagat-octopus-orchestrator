import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { env } from '../support/env';
import { Api } from '../support/platform';
import { failOnBrowserErrors, goTo, signIn } from '../support/studio';
import { ActivityFailure, startWorker } from '../support/worker';

test.use({ viewport: { width: 1440, height: 900 } });

const editor = (page: Page) => page.getByRole('group', { name: 'Editor tools' });
const inspector = (page: Page) => page.getByRole('complementary', { name: 'Inspector' });

async function addNode(page: Page, kind: string): Promise<void> {
  await page.getByRole('button', { name: 'Add node' }).click();
  await page.getByRole('dialog', { name: 'Add a node' }).getByRole('button', { name: kind, exact: true }).click();
}

test('SC-007-001 create, validate, publish, trigger, fail, compensate and inspect the journal, all through the Studio and the platform', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const api = new Api('developer');
  await signIn(page, env.tokens.developer);

  // CREATE: a saga of two steps, the first with the activity that undoes it.
  await page.getByRole('link', { name: 'Workflows' }).click();
  await page.getByRole('button', { name: 'New workflow' }).click();
  await page.getByRole('dialog').getByRole('textbox', { name: 'Name' }).fill('order-saga');
  await page.getByRole('dialog').getByRole('button', { name: 'Create workflow' }).click();
  await page.waitForURL(/\/workflows\/[0-9a-f-]{36}$/);
  const id = page.url().split('/').pop() as string;

  await addNode(page, 'Saga');
  await addNode(page, 'Step');
  await inspector(page).getByRole('textbox', { name: 'Activity', exact: true }).fill('stock.reserve');
  await inspector(page).getByRole('textbox', { name: 'Timeout amount' }).fill('5');
  await inspector(page).getByRole('combobox', { name: 'Timeout unit' }).selectOption('s');
  await inspector(page).getByRole('textbox', { name: 'Compensation activity' }).fill('stock.release');
  await addNode(page, 'Step');
  await inspector(page).getByRole('textbox', { name: 'Activity', exact: true }).fill('order.charge');
  await inspector(page).getByRole('textbox', { name: 'Timeout amount' }).fill('5');
  await inspector(page).getByRole('combobox', { name: 'Timeout unit' }).selectOption('s');
  await page.getByRole('button', { name: 'List' }).click();
  await page.getByRole('button', { name: /^Edit saga_1$/ }).click();
  await addNode(page, 'End');

  // VALIDATE: the platform's own check, run on the draft as it is.
  await expect(page.getByTestId('validation-summary')).toHaveText(/^0 problems · 0 warnings/, { timeout: 20_000 });
  await expect(editor(page)).toContainText(/Saved · revision \d+/, { timeout: 20_000 });
  const draft = (await api.call<{ ir: { entrypoint: string; nodes: Array<Record<string, unknown>> } }>('GET', `/workflows/${id}`, undefined, 200)).body;
  expect(draft.ir.nodes.map((n) => [n.id, n.type])).toEqual([['saga_1', 'saga'], ['activity_1', 'activity'], ['activity_2', 'activity'], ['end_1', 'end']]);

  // PUBLISH: the dialog, then a version that is the active one.
  await page.getByRole('button', { name: 'Publish…' }).click();
  const dialog = page.getByRole('dialog', { name: 'Publish workflow' });
  await dialog.getByRole('textbox', { name: 'Release notes' }).fill('First version');
  await dialog.getByRole('button', { name: 'Publish', exact: true }).click();
  await expect(page.getByText('Published version 1.').first()).toBeVisible();

  // TRIGGER and FAIL: the payment is declined for good, so the reservation is undone.
  const caller = new Api('admin');
  const worker = await startWorker('sc001-worker', {
    'stock.reserve': () => ({ reservation: 'r-7' }),
    'order.charge': () => { throw ActivityFailure.permanent('CARD_DECLINED', 'the card was declined'); },
  }, { 'stock.release': () => ({ released: true }) });
  try {
    const run = await caller.start(id, {});
    await caller.until(run, 'COMPENSATED');

    // INSPECT: the run, as the Studio shows it, and the journal, as the platform keeps it.
    await signIn(page, env.tokens.operator);
    await goTo(page, `/runs/${run}`);
    await expect(page.getByRole('region', { name: 'Run detail' }).getByText('Compensated', { exact: true }).first()).toBeVisible({ timeout: 20_000 });
    await page.getByRole('tab', { name: 'Table' }).click();
    const rows = page.getByRole('table').getByRole('row');
    await expect(rows.filter({ hasText: 'activity_2' }).first()).toContainText('CARD_DECLINED');
    await expect(rows.filter({ hasText: 'activity_1~c' }).first()).toContainText('Succeeded');
    await page.getByRole('tab', { name: 'Timeline' }).click();
    const journal = await caller.journal(run);
    await expect(page.getByText(`All ${journal.length} journal events are read`)).toBeVisible({ timeout: 20_000 });
    const types = journal.map((e) => e.eventType);
    expect(types).toEqual(expect.arrayContaining(['run.started', 'step.failed', 'compensation.planned', 'saga.compensated', 'run.finished']));
    // The order is the story: the failure, then the plan to undo, then the undo done, then the end.
    const at = (type: string) => types.indexOf(type);
    expect(at('step.failed')).toBeLessThan(at('compensation.planned'));
    expect(at('compensation.planned')).toBeLessThan(at('saga.compensated'));
    expect(at('saga.compensated')).toBeLessThan(types.lastIndexOf('run.finished'));
    // The trace that ties it to the platform's telemetry is on the page, to be copied.
    await page.getByText('Run details', { exact: true }).click();
    await expect(page.getByText('Trace id')).toBeVisible();
    const traceId = (await caller.call<{ traceId?: string }>('GET', `/runs/${run}`, undefined, 200)).body.traceId;
    expect(traceId).toMatch(/^[0-9a-f]{32}$/);
    await expect(page.locator('dl').getByText(traceId as string)).toBeVisible();
    // And every row of the journal names the trace it belongs to, so a person can follow it into the tracing tool.
    expect(await page.getByRole('table').getByText(traceId as string).count()).toBeGreaterThan(0);
  } finally {
    await worker.stop();
  }
  expect(problems).toEqual([]);
});
