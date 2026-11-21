import { expect } from '@playwright/test';
import type { Locator, Page } from '@playwright/test';
import { choose } from './ui';

/** How the sign-in's three controls are operated: the default fills, clicks and selects; a journey with the keyboard alone passes its own (`support/keyboard.ts`). */
export interface SignInControls {
  type(field: Locator, text: string): Promise<void>;
  press(button: Locator): Promise<void>;
  choose(select: Locator, option: string): Promise<void>;
}

const direct: SignInControls = {
  type: (field, text) => field.fill(text),
  press: (button) => button.click(),
  // A selector of the interface module is a button that opens a list: `choose` opens it and picks the option by its value.
  choose: (select, option) => choose(select, option),
};

/**
 * Signs in with a token, the way a user pastes one, and then chooses the namespace the journey works in: the
 * Studio chooses none for anybody, and the menu of a namespace is there only once one is chosen. `namespace`
 * names it; left out, it is the first the tenant lists; `null` stops after signing in (a first visit).
 * A choice the person already made in this browser is kept. `controls` says how the three controls are operated.
 */
export async function signIn(page: Page, token: string, { namespace, controls = direct }: { namespace?: string | null; controls?: SignInControls } = {}): Promise<void> {
  await page.goto('./');
  await controls.type(page.getByLabel('Access token'), token);
  await controls.press(page.getByRole('button', { name: 'Sign in' }));
  await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible();
  if (namespace === null) return;
  const main = page.getByRole('navigation', { name: 'Main' });
  const select = main.getByRole('combobox', { name: 'Namespace', exact: true });
  // The selector lists the tenant's namespaces as options that carry their slug (`data-option-value`).
  const listed = main.locator('[data-option-value]');
  // A tenant with no namespace has nothing to choose from, and the journey goes on without one.
  if (!(await listed.first().waitFor({ state: 'attached', timeout: 5_000 }).then(() => true, () => false))) return;
  if (((await select.getAttribute('data-value')) ?? '') !== '') return;
  await controls.choose(select, namespace ?? (await listed.first().getAttribute('data-option-value'))!);
  // The menu of the chosen namespace: its heading, then what lives in it.
  await expect(main.getByText(/^In /)).toBeVisible();
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
