import { expect, test } from '@playwright/test';
import { env } from '../support/env';
import { failOnBrowserErrors, signIn } from '../support/studio';
import { choose, expectAvailable, expectValue } from '../support/ui';

test('creates a persisted namespace from its navigation page and explains duplicate names', async ({ page }) => {
  const errors = failOnBrowserErrors(page);
  const slug = `studio-ns-${Date.now()}`;
  await signIn(page, env.tokens.admin);
  await page.getByRole('link', { name: 'Namespaces', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Namespaces', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'New namespace', exact: true }).click();
  await expectAvailable(page.getByRole('combobox', { name: 'Workspace', exact: true }));
  await page.getByLabel('Namespace name').fill(slug);
  await choose(page.getByRole('combobox', { name: 'Environment', exact: true }), 'dev');
  await page.getByRole('button', { name: 'Create namespace', exact: true }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expectValue(page.getByRole('combobox', { name: 'Namespace', exact: true }), slug);
  await expect(page.getByRole('heading', { name: 'Namespaces', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'New namespace', exact: true }).click();
  await page.getByLabel('Namespace name').fill(slug);
  await page.getByRole('button', { name: 'Create namespace', exact: true }).click();
  await expect(page.getByRole('dialog').getByRole('alert').filter({ hasText: 'already exists' })).toBeVisible();
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  await page.getByRole('button', { name: 'Sign out', exact: true }).click();
  await signIn(page, env.tokens.admin);
  await page.getByRole('link', { name: 'Namespaces', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Namespaces', exact: true })).toBeVisible();
  // The selector lists it once: its options carry their slug whether or not the list is open.
  await expect(page.getByRole('navigation', { name: 'Main' }).locator(`[data-option-value="${slug}"]`)).toHaveCount(1);
  await choose(page.getByRole('combobox', { name: 'Namespace', exact: true }), env.namespace);
  await page.getByRole('button', { name: `Use namespace ${slug}`, exact: true }).click();
  await expectValue(page.getByRole('combobox', { name: 'Namespace', exact: true }), slug);
  await page.setViewportSize({ width: 375, height: 812 });
  await page.getByRole('button', { name: 'New namespace', exact: true }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  expect(errors.filter((error) => !error.includes('409'))).toEqual([]);
});
