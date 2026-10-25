import { expect } from '@playwright/test';
import type { Page } from '@playwright/test';

/**
 * Signs in with a token, the way a user pastes one, and then chooses the namespace the journey works in: the
 * Studio chooses none for anybody, and the menu of a namespace is there only once one is chosen. `namespace`
 * names it; left out, it is the first the tenant lists; `null` stops after signing in (a first visit).
 * A choice the person already made in this browser is kept.
 */
export async function signIn(page: Page, token: string, { namespace }: { namespace?: string | null } = {}): Promise<void> {
  await page.goto('./');
  await page.getByLabel('Access token').fill(token);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible();
  if (namespace === null) return;
  const select = page.getByLabel('Namespace', { exact: true });
  const listed = select.locator('option:not([value=""])');
  // A tenant with no namespace has nothing to choose from, and the journey goes on without one.
  if (!(await listed.first().waitFor({ state: 'attached', timeout: 5_000 }).then(() => true, () => false))) return;
  if ((await select.inputValue()) !== '') return;
  await select.selectOption(namespace ?? (await listed.first().getAttribute('value'))!);
  await expect(page.getByRole('list', { name: /^In / })).toBeVisible();
}

/** Fails the journey on a script error or a policy violation: a Studio that logs errors is not working. */
export function failOnBrowserErrors(page: Page): string[] {
  const problems: string[] = [];
  page.on('pageerror', (error) => problems.push(`pageerror: ${error.message}`));
  page.on('console', (message) => {
    if (message.type() === 'error') problems.push(`console.error: ${message.text()}`);
  });
  return problems;
}

/**
 * Goes to a page of the Studio the way a link does: through the router, so the token held in the page's memory
 * survives (a page load would ask for it again: it is never stored).
 */
export async function goTo(page: Page, path: string): Promise<void> {
  await page.evaluate((to) => {
    window.history.pushState({}, '', `/studio${to}`);
    window.dispatchEvent(new PopStateEvent('popstate'));
  }, path);
}
