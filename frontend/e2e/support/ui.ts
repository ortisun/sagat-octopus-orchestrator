import { expect } from '@playwright/test';
import type { Locator, Page } from '@playwright/test';

/**
 * How a journey operates the controls of the Studio's interface module (Architecture decision 0101, Decision 2403). The design
 * system draws a selector as a button that opens a list, and marks an unavailable control with the
 * native attribute or with `aria-disabled`, by its own choice per release: journeys go through these
 * helpers so that neither detail is written in a journey.
 */

/** Choose an option of a selector: by the value the platform receives (a string) or by the label a person reads. */
export async function choose(control: Locator, option: string | { label: string }): Promise<void> {
  const page = control.page();
  await control.click();
  const item =
    typeof option === 'string'
      ? page.getByRole('option').filter({ has: page.locator(`[data-option-value=${JSON.stringify(option)}]`) })
      : page.getByRole('option', { name: option.label, exact: true });
  await item.click();
}

/** The value (not the label) a selector holds. */
export async function expectValue(control: Locator, value: string): Promise<void> {
  await expect(control).toHaveAttribute('data-value', value);
}

const unavailable = (element: Element) => element.getAttribute('aria-disabled') === 'true' || (element as HTMLButtonElement).disabled === true;

/** The control is there and cannot be used. */
export async function expectUnavailable(control: Locator): Promise<void> {
  await expect(control).toBeVisible();
  await expect.poll(() => control.evaluate(unavailable)).toBe(true);
}

/** The control is there and can be used. */
export async function expectAvailable(control: Locator): Promise<void> {
  await expect(control).toBeVisible();
  await expect.poll(() => control.evaluate(unavailable)).toBe(false);
}

/**
 * The field says what is wrong with it: the control is marked invalid and the message is one of the
 * elements that describe it (`aria-describedby`). It is not an alert of its own; the design system
 * also repeats the words in a live region for a moment, so a journey that looked for the words alone
 * could find them twice.
 */
export async function expectFieldError(control: Locator, message?: string | RegExp): Promise<void> {
  await expect(control).toHaveAttribute('aria-invalid', 'true');
  const said = () =>
    control.evaluate((element) =>
      (element.getAttribute('aria-describedby') ?? '')
        .split(/\s+/)
        .map((id) => (id === '' ? null : document.getElementById(id)))
        .filter((described): described is HTMLElement => described !== null && described.matches('[data-type="error"]') && described.getClientRects().length > 0)
        .map((described) => described.textContent ?? '')
        .join(' ')
        .trim(),
    );
  if (message === undefined) await expect.poll(said).not.toBe('');
  else if (typeof message === 'string') await expect.poll(said).toContain(message);
  else await expect.poll(said).toMatch(message);
}

/**
 * The value of a named fact of a page (the interface module's `Facts`, a description list of the design system): the `dd` that
 * follows the `dt` whose whole text is `term`, inside `scope`. The design system puts a term and its value side by side in the
 * list, or wraps each pair in an element of its own when the facts are laid out in columns: the value follows its term either way.
 */
export function factValue(scope: Page | Locator, term: string): Locator {
  const whole = new RegExp(`^\\s*${term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*$`);
  return scope.locator('dt').filter({ hasText: whole }).locator('xpath=following-sibling::dd[1]');
}
