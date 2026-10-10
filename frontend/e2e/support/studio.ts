import { expect } from '@playwright/test';
import type { Page } from '@playwright/test';

/** Signs in with a token, the way a user pastes one. */
export async function signIn(page: Page, token: string): Promise<void> {
  await page.goto('./');
  await page.getByLabel('Access token').fill(token);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible();
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
