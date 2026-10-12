import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { env } from '../support/env';
import { Api, paymentSaga } from '../support/platform';
import { failOnBrowserErrors, goTo, signIn } from '../support/studio';
import { ActivityFailure, startWorker } from '../support/worker';

test.use({ viewport: { width: 1440, height: 900 } });

const card = (page: Page, id: string) => page.getByRole('group', { name: new RegExp(`^${id.replace(/[~.]/g, '\\$&')}, `) });

test('US03 an operator opens a run that retried and took one branch: attempts, states, the undo and the journal show what happened, live, without reloading the graph', async ({ page }) => {
  test.setTimeout(180_000);
  const problems = failOnBrowserErrors(page);
  const api = new Api('admin');
  const wf = await api.publish('payment-us03', paymentSaga('payment-us03'));

  // The operator opens the run before anything has happened to it.
  const run = await api.start(wf.id, { paymentId: 'p-1', amount: 25 });
  await signIn(page, env.tokens.operator);
  const graphCalls: string[] = [];
  page.on('request', (request) => {
    const url = new URL(request.url());
    if (/\/v1\/runs\/[^/]+\/graph(\/changes)?$/.test(url.pathname)) graphCalls.push(`${request.method()} ${url.pathname.replace(run, ':run')}`);
  });
  await goTo(page, `/runs/${run}`);
  await expect(page.getByRole('heading', { level: 1 })).toContainText(run);
  await expect(card(page, 'validate')).toBeVisible();
  await expect(card(page, 'validate')).toContainText(/Ready|Pending|Not started/);
  await expect(page.getByRole('region', { name: 'Run detail' }).getByText('Live', { exact: true }).first()).toBeVisible({ timeout: 20_000 });
  // The graph opens at a readable scale; choose the overview before following every branch at once.
  await page.getByRole('button', { name: 'Fit View', exact: true }).click();

  // A worker comes: the first step fails once (retry), the payment is declined, the reservation is undone.
  const attempts = new Map<string, number>();
  const host = await startWorker('us03-worker', {
    'payment.validate': (_input, ctx) => {
      attempts.set('validate', ctx.attempt);
      if (ctx.attempt === 1) throw ActivityFailure.retryable('PROVIDER_BUSY', 'try again');
      return { valid: true };
    },
    'limit.reserve': () => ({ reservation: 'r-1' }),
    'payment.submit': () => { throw ActivityFailure.permanent('PAYMENT_DECLINED', 'declined by the issuer'); },
  }, { 'limit.release': () => ({ released: true }) });
  try {
    await api.until(run, 'COMPENSATED');
    // The page followed it: the states are the platform's, reached without reloading the page.
    await expect(page.getByRole('region', { name: 'Run detail' }).getByText('Compensated', { exact: true }).first()).toBeVisible({ timeout: 20_000 });
    await expect(card(page, 'validate')).toContainText('Succeeded');
    await expect(card(page, 'validate')).toContainText(/attempt 2/);
    await expect(card(page, 'check')).toContainText('Succeeded');
    await expect(card(page, 'review')).toContainText('Not started');
    await expect(card(page, 'reserve')).toContainText('Succeeded');
    await expect(card(page, 'submit')).toContainText('Failed');
    await expect(card(page, 'submit')).toContainText('PAYMENT_DECLINED');
    await expect(card(page, 'reserve~c')).toContainText('Succeeded');
    await expect(card(page, 'reserve~c')).toContainText(/Undo of reserve/);
    // The way not taken is told as omitted, in words and not only by its line.
    await expect(page.getByRole('img', { name: /check to review: If true, omitted/ })).toBeAttached();
    await expect(page.getByRole('img', { name: /check to settlement: If false, taken/ })).toBeAttached();
  } finally {
    await host.stop();
  }
  expect(attempts.get('validate')).toBe(2);

  // The graph was loaded once and then followed: no reload of the whole graph while the run moved.
  expect(graphCalls.filter((c) => c.endsWith('/graph'))).toHaveLength(1);

  // The same data as a table, for anyone who does not use the canvas.
  await page.getByRole('tab', { name: 'Table' }).click();
  const rows = page.getByRole('table').getByRole('row');
  await expect(rows.filter({ hasText: 'submit' }).first()).toContainText('Failed');
  await expect(rows.filter({ hasText: 'reserve~c' }).first()).toContainText('Succeeded');
  await expect(rows).toHaveCount(10); // the header and nine steps

  // The journal, in words, with the retry and the failure to be found.
  await page.getByRole('tab', { name: 'Timeline' }).click();
  const journal = await api.journal(run);
  await expect(page.getByText(`All ${journal.length} journal events are read`)).toBeVisible({ timeout: 20_000 });
  await expect(page.getByText('The step failed and a retry was scheduled')).toBeVisible();
  await page.getByRole('checkbox', { name: 'Only problems' }).check();
  await expect(page.getByText('The step failed and a retry was scheduled')).toBeVisible();
  await expect(page.getByText('The run started')).toBeHidden();
  // What the timeline says is the platform's journal: the retry, the compensation and the end of the run are in it.
  expect(journal.map((e) => e.eventType)).toEqual(expect.arrayContaining(['step.retry_scheduled', 'step.failed', 'run.finished']));
  expect(problems).toEqual([]);
});
