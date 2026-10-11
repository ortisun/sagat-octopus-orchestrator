import { expect, test } from '@playwright/test';
import { failOnBrowserErrors, goTo, signIn } from '../support/studio';

function need(name: 'STUDIO_RUN_ID' | 'STUDIO_TOKEN'): string {
  const value = process.env[name];
  if (value === undefined || value === '') throw new Error(`${name} is required`);
  return value;
}

test.use({ viewport: { width: 1440, height: 900 } });

test('compose, CLI and TypeScript SDK produce a run visible throughout the real Studio', async ({ page }) => {
  const runId = need('STUDIO_RUN_ID');
  const token = need('STUDIO_TOKEN');
  const problems = failOnBrowserErrors(page);

  try {
    await signIn(page, token);
  } catch {
    // A failed locator.fill includes its argument in the error's call log. Do not retain the
    // original error/cause: both the console reporter and error-context.md receive this error.
    throw new Error('Studio sign-in failed. Check the sign-in form and development identity provider; credential details are withheld.');
  }
  await expect(page.getByLabel('Access token')).toHaveCount(0);
  const persisted = await page.evaluate(() => JSON.stringify({ local: { ...localStorage }, session: { ...sessionStorage }, cookie: document.cookie }));
  expect(persisted.includes(token)).toBe(false);
  expect(page.url().includes(token)).toBe(false);
  await goTo(page, `/runs/${runId}`);

  const run = page.locator('.rn-run');
  await expect(run.getByRole('heading', { level: 1 })).toContainText(runId);
  await expect(run.getByText('Succeeded', { exact: true }).first()).toBeVisible();

  const graph = run.getByRole('group', { name: 'Graph of the run' });
  await expect(graph).toBeVisible();
  await expect(graph).toContainText('compose');
  await expect(graph).toContainText('deliver');
  await expect(graph.getByText('Succeeded', { exact: true })).toHaveCount(2);

  await run.getByRole('tab', { name: 'Table' }).click();
  const rows = run.getByRole('table').getByRole('row');
  await expect(rows.filter({ hasText: 'compose' }).first()).toContainText('Succeeded');
  await expect(rows.filter({ hasText: 'deliver' }).first()).toContainText('Succeeded');
  await expect(rows).toHaveCount(3);

  await run.getByRole('tab', { name: 'Timeline' }).click();
  await expect(run.getByText(/All \d+ journal events are read/)).toBeVisible();
  const journal = run.getByRole('table', { name: 'Journal events, oldest first' });
  await expect(journal).toContainText('The run started');
  await expect(journal).toContainText('The run finished');
  await expect(journal).toContainText('compose');
  await expect(journal).toContainText('deliver');

  expect(problems).toEqual([]);
});
