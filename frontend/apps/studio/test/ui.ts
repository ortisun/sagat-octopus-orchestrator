import { within } from '@testing-library/react';
import type { UserEvent } from '@testing-library/user-event';
import { expect } from 'vitest';

/**
 * How a test operates the controls the interface module draws (ADR-0101, D2403). Kept in one place so
 * that a change of the design system's markup is one edit here, not one per test.
 */

type Name = string | RegExp;

/** The selector (a `combobox` that opens a `listbox`) with this label. */
export function selector(name: Name, container: HTMLElement = document.body): HTMLElement {
  return within(container).getByRole('combobox', { name });
}

/** The list a selector opens: its own, so that another selector's options are never taken for it. */
function listOf(control: HTMLElement): HTMLElement {
  const id = control.getAttribute('aria-controls');
  const list = id ? document.getElementById(id) : null;
  if (!list) throw new Error(`The selector ${control.outerHTML.slice(0, 120)} names no list (aria-controls)`);
  return list;
}

async function open(user: UserEvent, control: HTMLElement): Promise<HTMLElement> {
  if (control.getAttribute('aria-expanded') !== 'true') await user.click(control);
  expect(control, 'the selector opened').toHaveAttribute('aria-expanded', 'true');
  return listOf(control);
}

/** Choose `option` in the selector labelled `name`, as a person does: open it, then pick. */
export async function choose(user: UserEvent, name: Name | HTMLElement, option: Name): Promise<void> {
  const control = name instanceof HTMLElement ? name : selector(name);
  const list = await open(user, control);
  await user.click(await within(list).findByRole('option', { name: option }));
}

/** The options a selector offers, by their text, after opening it. It is left open. */
export async function optionsOf(user: UserEvent, name: Name | HTMLElement): Promise<string[]> {
  const control = name instanceof HTMLElement ? name : selector(name);
  const list = await open(user, control);
  return (await within(list).findAllByRole('option')).map((option) => option.textContent ?? '');
}

/** Choose the option whose value (not its label) is `value`: for a test that is about the value sent to the platform. */
export async function chooseValue(user: UserEvent, name: Name | HTMLElement, value: string): Promise<void> {
  const control = name instanceof HTMLElement ? name : selector(name);
  const list = await open(user, control);
  const options = await within(list).findAllByRole('option');
  const option = options.find((candidate) => candidate.querySelector(`[data-option-value="${CSS.escape(value)}"]`) !== null);
  if (!option) throw new Error(`No option with the value ${value}; the selector offers ${options.map((candidate) => candidate.textContent).join(', ')}`);
  await user.click(option);
}

/** The value (not the label) a selector holds; an empty string while nothing is chosen. */
export function valueOf(name: Name | HTMLElement): string {
  const control = name instanceof HTMLElement ? name : selector(name);
  return control.getAttribute('data-value') ?? control.closest('[data-value]')?.getAttribute('data-value') ?? '';
}

/** What a selector shows as chosen. */
export function chosen(name: Name | HTMLElement): string {
  const control = name instanceof HTMLElement ? name : selector(name);
  return control.textContent ?? '';
}

/** A control that is unavailable: focusable, announced as disabled (`aria-disabled`), or natively disabled. */
export function isUnavailable(element: HTMLElement): boolean {
  return element.getAttribute('aria-disabled') === 'true' || (element as HTMLButtonElement).disabled === true;
}

export function expectUnavailable(element: HTMLElement): void {
  expect(isUnavailable(element), `expected ${element.outerHTML.slice(0, 120)} to be unavailable`).toBe(true);
}

export function expectAvailable(element: HTMLElement): void {
  expect(isUnavailable(element), `expected ${element.outerHTML.slice(0, 120)} to be available`).toBe(false);
}

/** What the design system last announced assertively (an error of a field, for example). */
export function announcedAssertively(): string {
  return document.querySelector('[data-astryx-live-region="assertive"]')?.textContent ?? '';
}

/** What the design system last announced politely. */
export function announcedPolitely(): string {
  return document.querySelector('[data-astryx-live-region="polite"]')?.textContent ?? '';
}
