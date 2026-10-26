import { expect, test } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { env } from '../support/env';
import { failOnBrowserErrors, goTo, signIn } from '../support/studio';

test('the Studio is served with its policy, signs a user in and shows the shell of their namespace', async ({ page, request }) => {
  const problems = failOnBrowserErrors(page);

  // The headers that keep foreign scripts out (D143), as the browser receives them.
  const response = await request.get(`${env.origin}/studio/`);
  expect(response.status()).toBe(200);
  expect(response.headers()['content-security-policy']).toContain("script-src 'self'");
  expect(response.headers()['x-content-type-options']).toBe('nosniff');

  await signIn(page, env.tokens.admin);
  await expect(page.getByLabel('Namespace', { exact: true })).toHaveValue(env.namespace);
  await expect(page.getByLabel('Signed in as')).toContainText('alice');
  await page.getByRole('link', { name: 'Runs' }).click();
  await expect(page).toHaveURL(/\/studio\/runs$/);

  // The token is in memory only: a reload asks for it again, and nothing of it is in storage.
  const stored = await page.evaluate(() => JSON.stringify({ local: { ...localStorage }, session: { ...sessionStorage }, cookie: document.cookie }));
  expect(stored).not.toContain(env.tokens.admin.slice(0, 20));
  await page.reload();
  await expect(page.getByLabel('Access token')).toBeVisible();

  expect(problems).toEqual([]);
});

test('the menu follows the hierarchy: only the namespaces until one is chosen, then what lives in it', async ({ page }) => {
  const problems = failOnBrowserErrors(page);
  await signIn(page, env.tokens.admin, { namespace: null });
  const menu = page.getByRole('navigation', { name: 'Main' });
  const select = page.getByLabel('Namespace', { exact: true });

  // A first visit: the Studio chooses nothing, so the menu offers the tenant's namespaces and nothing of a namespace.
  await expect(select).toHaveValue('');
  await expect(menu.getByRole('link')).toHaveText(['Namespaces']);
  for (const name of ['Workflows', 'Scripts', 'Runs', 'Operations', 'Ad hoc database', 'Connections', 'Secrets', 'Integrations']) await expect(menu.getByRole('link', { name, exact: true })).toHaveCount(0);

  // A page of a namespace, reached by its address, says how to choose one.
  await goTo(page, '/runs');
  await expect(page.getByRole('heading', { name: 'Choose a namespace', level: 1 })).toBeVisible();

  // Choosing one opens its menu, under it, and the page the person was on.
  await select.selectOption(env.namespace);
  await expect(page.getByRole('list', { name: `In ${env.namespace}` }).getByRole('link')).toHaveText(['Workflows', 'Scripts', 'Runs', 'Operations', 'Ad hoc database', 'Connections', 'Secrets', 'Integrations']);
  await expect(page.getByRole('heading', { name: 'Choose a namespace', level: 1 })).toHaveCount(0);
  await expect(page).toHaveURL(/\/studio\/runs$/);

  expect(problems).toEqual([]);
});

test('a token the platform does not accept is refused with a message and nothing is kept', async ({ page }) => {
  await page.goto('./');
  await page.getByLabel('Access token').fill('aaaa.bbbb.cccc');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('alert')).toBeVisible();
  await expect(page.getByLabel('Access token')).toBeVisible();
});

test('a missing route stylesheet offers an explicit reload and the Studio recovers', async ({ page }) => {
  // Start on another route so workflows CSS has not been downloaded into this tab yet.
  await page.goto('./runs');
  await page.getByLabel('Access token').fill(env.tokens.admin);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible();
  await page.getByLabel('Namespace', { exact: true }).selectOption(env.namespace);

  let failedStylesheets = 0;
  await page.route('**/assets/workflows.*.css', async (route) => {
    failedStylesheets += 1;
    await route.fulfill({ status: 404, contentType: 'text/css', body: '' });
  });
  await page.getByRole('link', { name: 'Workflows' }).click();
  const heading = page.getByRole('heading', { name: 'Unable to open this page' });
  await expect(heading).toBeVisible();
  await expect(heading).toBeFocused();
  expect(failedStylesheets).toBeGreaterThan(0);
  await expect(page.getByText(/unsaved changes will be lost/i)).toBeVisible();
  await expect(page.getByText(/Unexpected Application Error|Hey developer|Unable to preload CSS/)).toHaveCount(0);
  await page.setViewportSize({ width: 320, height: 812 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(321);
  const accessibility = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa']).analyze();
  expect(accessibility.violations).toEqual([]);
  await page.screenshot({ path: 'test-results/route-load-recovery.png' });

  // Restoring the asset and choosing reload obtains a fresh entry document. No token persists.
  await page.unroute('**/assets/workflows.*.css');
  const problems = failOnBrowserErrors(page);
  await page.getByRole('button', { name: 'Reload Studio' }).click();
  await expect(page.getByLabel('Access token')).toBeVisible();
  await page.getByLabel('Access token').fill(env.tokens.admin);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('heading', { name: 'Workflows', exact: true })).toBeVisible();
  expect(problems).toEqual([]);
});
