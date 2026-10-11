import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { env } from '../support/env';
import { Api, paymentSaga } from '../support/platform';
import { failOnBrowserErrors, goTo, signIn } from '../support/studio';
import { ActivityFailure, startWorker } from '../support/worker';

test.use({ viewport: { width: 1440, height: 900 } });

/** WCAG 2.2 AA is the target (FR-007-008): the automated rules for A and AA of 2.0, 2.1 and 2.2. */
async function expectAccessible(page: Page, where: string): Promise<void> {
  const result = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa']).analyze();
  const found = result.violations.map((v) => `${v.id} (${v.impact}): ${v.nodes.slice(0, 3).map((n) => n.target.join(' ')).join(' | ')}`);
  expect(found, `${where}: accessibility violations`).toEqual([]);
}

test('A11Y the pages of the Studio have no automated WCAG 2.2 AA violation, with the canvas and with the table', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const api = new Api('admin');
  const wf = await api.publish('payment-a11y', paymentSaga('payment-a11y'));
  const run = await api.start(wf.id, { paymentId: 'p-3', amount: 25 });
  const host = await startWorker('a11y-worker', {
    'payment.validate': () => ({ valid: true }),
    'limit.reserve': () => ({ reservation: 'r-1' }),
    'payment.submit': () => { throw ActivityFailure.permanent('PAYMENT_DECLINED', 'declined by the issuer'); },
  }, { 'limit.release': () => ({ released: true }) });
  try {
    await api.until(run, 'COMPENSATED');
  } finally {
    await host.stop();
  }

  await page.goto('./');
  await expectAccessible(page, 'the sign-in page');
  await signIn(page, env.tokens.operator);

  await page.getByRole('link', { name: 'Workflows' }).click();
  await expect(page.getByRole('heading', { name: 'Workflows', level: 1 })).toBeVisible();
  await expectAccessible(page, 'the workflows list');

  await page.getByRole('link', { name: 'payment-a11y' }).click();
  await expect(page.getByRole('heading', { name: 'payment-a11y', level: 1 })).toBeVisible();
  await expectAccessible(page, 'the workflow editor, canvas');
  await page.getByRole('button', { name: 'List' }).click();
  await expectAccessible(page, 'the workflow editor, list');

  await page.getByRole('link', { name: 'Runs' }).click();
  await expect(page.getByRole('heading', { name: 'Runs', level: 1 })).toBeVisible();
  await expectAccessible(page, 'the runs list');

  await goTo(page, `/runs/${run}`);
  await expect(page.getByText('Compensated', { exact: true }).first()).toBeVisible({ timeout: 20_000 });
  await expectAccessible(page, 'the run, graph');
  await page.getByRole('tab', { name: 'Table' }).click();
  await expectAccessible(page, 'the run, table');
  await page.getByRole('tab', { name: 'Timeline' }).click();
  await expect(page.getByText(/journal events are read/)).toBeVisible({ timeout: 20_000 });
  await expectAccessible(page, 'the run, timeline');

  await page.getByRole('link', { name: 'Scripts' }).click();
  await expect(page.getByRole('heading', { name: 'Scripts', level: 1 })).toBeVisible();
  await expectAccessible(page, 'the scripts list');
  expect(problems).toEqual([]);
});

test('A11Y a dialog and a menu are operated with the keyboard alone: focus goes in, stays in, and comes back', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const api = new Api('admin');
  const wf = await api.publish('payment-a11y-keys', paymentSaga('payment-a11y-keys'));
  // No worker: the first step waits, so the run is live and can be paused.
  const run = await api.start(wf.id, { paymentId: 'p-4', amount: 25 });
  await signIn(page, env.tokens.operator);
  await goTo(page, `/runs/${run}`);
  await expect(page.getByRole('heading', { level: 1 })).toContainText(run);

  // The disclosure opens with Enter, shows its list, and Escape closes it with focus back on its button.
  const trigger = page.getByRole('button', { name: 'Run actions' });
  await trigger.focus();
  await page.keyboard.press('Enter');
  await expect(trigger).toHaveAttribute('aria-expanded', 'true');
  await expectAccessible(page, 'the run actions menu, open');
  await page.keyboard.press('Escape');
  await expect(trigger).toHaveAttribute('aria-expanded', 'false');
  await expect(trigger).toBeFocused();

  // A decision dialog: opened from the keyboard, focus moves into it and cannot leave it, Escape closes it, focus returns.
  await page.keyboard.press('Enter');
  await page.getByRole('button', { name: 'Pause run' }).focus();
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('textbox', { name: 'Reason' })).toBeFocused();
  await expectAccessible(page, 'the decision dialog');
  // The page behind a modal dialog is inert: Tab cycles inside the dialog (and may pass through the browser's own
  // controls, where the page has no focused element), but never lands on something of the page behind it.
  const behind = () => dialog.evaluate((el) => document.activeElement !== null && document.activeElement !== document.body && !el.contains(document.activeElement));
  for (let i = 0; i < 10; i += 1) {
    await page.keyboard.press('Tab');
    expect(await behind(), `Tab ${i + 1} moved focus to the page behind the dialog`).toBe(false);
  }
  for (let i = 0; i < 10; i += 1) {
    await page.keyboard.press('Shift+Tab');
    expect(await behind(), `Shift+Tab ${i + 1} moved focus to the page behind the dialog`).toBe(false);
  }
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(trigger).toBeFocused();
  expect(problems).toEqual([]);
});
