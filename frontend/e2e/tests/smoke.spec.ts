import { expect, test } from '@playwright/test';
import { env } from '../support/env';
import { failOnBrowserErrors, signIn } from '../support/studio';

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

test('a token the platform does not accept is refused with a message and nothing is kept', async ({ page }) => {
  await page.goto('./');
  await page.getByLabel('Access token').fill('aaaa.bbbb.cccc');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('alert')).toBeVisible();
  await expect(page.getByLabel('Access token')).toBeVisible();
});
