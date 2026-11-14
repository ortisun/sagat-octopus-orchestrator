import { expect } from '@playwright/test';
import type { CDPSession, Locator, Page } from '@playwright/test';
import { signIn } from './studio';
import type { SignInControls } from './studio';

/**
 * The keyboard-only vocabulary of the journeys of feature 018 (schedules, batch operations, reset, business identifiers).
 *
 * A journey that uses these never moves a pointer: it Tabs to a control (the real key, one press at a time, so the order of
 * the page is what is tested), and acts on it with Enter, Space or the arrows, the way a person who does not use a mouse does.
 * Text goes in with `insertText` (what a paste or an input method does), never with `fill`, so the field has to have focus.
 *
 * What is proved about a native `<select>` (`choose`): that the keyboard reaches it, in the order of the page, and that it has an
 * accessible name. What is not: how the browser operates a closed select by keyboard (type-ahead, arrows, the popup) is the
 * browser's own behaviour, differs by platform (an arrow opens the popup on macOS and changes the value on Linux) and keeps its
 * state for a second between keys, which is not the Studio's to prove and would make a journey depend on the speed of the
 * machine; the value is set with `selectOption`, which moves no pointer and fires what a choice fires.
 */

/** Is the element the one holding focus? */
const focused = (target: Locator) => target.evaluate((element) => element === document.activeElement);

/** The direction of Tab to `target` from where focus is now, read from the order of the document (`true` for forward). */
const forwardTo = (target: Locator) =>
  target.evaluate((element) => {
    const from = document.activeElement;
    if (from === null || from === document.body) return true;
    return (from.compareDocumentPosition(element) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;
  });

/**
 * Tabs (or Shift+Tabs) to `target`, one real key press at a time, and fails if the page cannot reach it: that is a defect
 * of the page (an element that needs a pointer), not of the journey.
 */
export async function reach(page: Page, target: Locator, { max = 120 }: { max?: number } = {}): Promise<void> {
  await expect(target).toBeVisible();
  for (let step = 0; step <= max; step += 1) {
    if (await focused(target)) return;
    await page.keyboard.press((await forwardTo(target)) ? 'Tab' : 'Shift+Tab');
  }
  throw new Error(`the keyboard cannot reach ${String(target)} within ${max} presses of Tab`);
}

/** Tabs to a control and presses Enter on it (a link, a button, a form's submit). */
export async function activate(page: Page, target: Locator, key: 'Enter' | 'Space' = 'Enter'): Promise<void> {
  await reach(page, target);
  await page.keyboard.press(key);
}

/** Tabs to a text field and replaces what it holds with `text`. */
export async function typeInto(page: Page, target: Locator, text: string): Promise<void> {
  await reach(page, target);
  await page.keyboard.press('ControlOrMeta+a');
  if (text === '') await page.keyboard.press('Delete');
  else await page.keyboard.insertText(text);
  await expect(target).toHaveValue(text);
}

/**
 * Chooses the option whose text is `label` in a native select: waits until the select is enabled and has the option (a page builds
 * them from what the platform answers: the workflows of the namespace, the namespaces of the tenant), Tabs to the select and checks
 * that it has an accessible name, then sets the value with `selectOption`, retried until the select says it holds it (see the
 * header for what that proves).
 */
export async function choose(page: Page, select: Locator, label: string): Promise<void> {
  await expect(select).toBeEnabled();
  await expect(select.locator('option', { hasText: label })).not.toHaveCount(0);
  await reach(page, select);
  const node = await focusedNode(page);
  expect(node?.role, `the focused element ${String(select)} is a select`).toMatch(/combobox|listbox/);
  expect(node?.name.trim(), `the select ${String(select)} has no accessible name`).not.toBe('');
  await expect(async () => {
    await select.selectOption({ label }, { timeout: 5_000 });
    await expect(select.locator('option:checked')).toHaveText(label, { timeout: 2_000 });
  }).toPass({ timeout: 30_000 });
}

/** Tabs to a checkbox and sets it with Space. */
export async function setChecked(page: Page, box: Locator, checked: boolean): Promise<void> {
  await reach(page, box);
  if ((await box.isChecked()) !== checked) await page.keyboard.press('Space');
  if (checked) await expect(box).toBeChecked();
  else await expect(box).not.toBeChecked();
}

/**
 * Tabs into the group of radio buttons `target` belongs to, then moves with the arrows until `target` is the chosen one. Tab
 * enters a group on the radio that is chosen, or on its first (forward) or last (backward) when none is: any radio of the group
 * is a way in.
 */
export async function chooseRadio(page: Page, target: Locator): Promise<void> {
  await expect(target).toBeVisible();
  if (await target.isChecked()) return;
  const group = page.locator(`input[type="radio"][name="${(await target.getAttribute('name')) ?? ''}"]`);
  const size = await group.count();
  const inside = () => group.evaluateAll((radios) => radios.some((radio) => radio === document.activeElement));
  for (let step = 0; step <= 120 && !(await inside()); step += 1) await page.keyboard.press((await forwardTo(group.first())) ? 'Tab' : 'Shift+Tab');
  expect(await inside(), 'the keyboard reaches the group of radio buttons').toBe(true);
  for (let step = 0; step < size && !(await focused(target)); step += 1) await page.keyboard.press('ArrowDown');
  await expect(target).toBeFocused();
  // an arrow chooses the radio it moves to; a radio that Tab entered on is chosen with Space
  if (!(await target.isChecked())) await page.keyboard.press('Space');
  await expect(target).toBeChecked();
}

/** One session of the browser's debugging protocol for a page, for as long as the page lives (one per Tab press would be a hundred a walk). */
const sessions = new WeakMap<Page, CDPSession>();
async function protocol(page: Page): Promise<CDPSession> {
  let session = sessions.get(page);
  if (session === undefined) {
    session = await page.context().newCDPSession(page);
    sessions.set(page, session);
  }
  return session;
}

/** What the browser's accessibility tree says of the focused element: its role and its accessible name. */
async function focusedNode(page: Page): Promise<{ role: string; name: string; tag: string } | undefined> {
  const session = await protocol(page);
  const { result } = await session.send('Runtime.evaluate', { expression: 'document.activeElement' });
  if (result.objectId === undefined || result.subtype === 'null') return undefined;
  const tag = await page.evaluate(() => document.activeElement?.tagName.toLowerCase() ?? '');
  if (tag === 'body' || tag === 'html') return undefined;
  const tree = await session.send('Accessibility.getPartialAXTree', { objectId: result.objectId, fetchRelatives: false });
  const node = tree.nodes.find((candidate) => candidate.ignored !== true);
  return { role: String(node?.role?.value ?? ''), name: String(node?.name?.value ?? ''), tag };
}

/**
 * Tabs once around `scope` (the whole page when left out) and returns `role: name` of every element reached, in order. Fails
 * when one of them has no accessible name, or when focus leaves `scope` (the page behind a modal dialog is not for the keyboard).
 */
export async function walk(page: Page, scope?: Locator, { max = 120 }: { max?: number } = {}): Promise<string[]> {
  const reached: string[] = [];
  const seen = new Set<string>();
  for (let step = 0; step < max; step += 1) {
    await page.keyboard.press('Tab');
    const node = await focusedNode(page);
    // the browser's own controls hold focus for a press between the end of the page and its start
    if (node === undefined) continue;
    if (scope !== undefined) {
      const inside = await scope.evaluate((root) => root.contains(document.activeElement));
      expect(inside, `Tab ${step + 1} moved focus out of the scope to ${node.role} "${node.name}"`).toBe(true);
    }
    const id = await page.evaluate(() => {
      const element = document.activeElement as HTMLElement | null;
      if (element === null) return '';
      const path: string[] = [];
      for (let at: Element | null = element; at !== null && at !== document.body; at = at.parentElement) path.push(`${at.tagName}${Array.prototype.indexOf.call(at.parentElement?.children ?? [], at)}`);
      return path.join('<');
    });
    if (seen.has(id)) return reached;
    seen.add(id);
    expect(node.name.trim(), `the ${node.role} (${node.tag}) reached with Tab number ${step + 1} has no accessible name`).not.toBe('');
    reached.push(`${node.role}: ${node.name}`);
  }
  throw new Error(`the Tab order did not come back to its start within ${max} presses: ${reached.join(' > ')}`);
}

/**
 * What a modal dialog owes a person at a keyboard, proved once the dialog's content is there: Tab and Shift+Tab pressed more
 * times than the dialog has controls never put focus on the page behind it (a trap that leaked at an end would show), and the
 * Tab order of the dialog is walked, every element reached having an accessible name. Returns `role: name` of what was reached.
 */
export async function inspectDialog(page: Page, dialog: Locator): Promise<string[]> {
  const controls = await dialog.evaluate((root) => root.querySelectorAll('a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])').length);
  const outside = () => dialog.evaluate((root) => document.activeElement !== null && document.activeElement !== document.body && !root.contains(document.activeElement));
  for (const key of ['Tab', 'Shift+Tab']) {
    for (let step = 0; step < controls + 3; step += 1) {
      await page.keyboard.press(key);
      expect(await outside(), `${key} number ${step + 1} moved focus to the page behind the dialog`).toBe(false);
    }
  }
  return walk(page, dialog);
}

/** The controls of the sign-in operated by the keyboard (the token goes in a text area, where Enter is a new line: the way on is Tab to the button and Enter). */
export function signInControls(page: Page): SignInControls {
  return {
    type: (field, text) => typeInto(page, field, text),
    press: (button) => activate(page, button),
    choose: (select, option) => choose(page, select, option),
  };
}

/** Signs in with the keyboard alone and chooses the namespace (`support/studio.ts`, the way every journey signs in). */
export async function signInByKeyboard(page: Page, token: string, namespace: string): Promise<void> {
  await signIn(page, token, { namespace, controls: signInControls(page) });
}

/**
 * The identifier at the end of the address once the Studio has moved to a page that is not one of `except` (a dialog hides
 * before the router has moved, so the address read at that moment can still be the one of the page the person left).
 */
export async function landedOn(page: Page, ...except: string[]): Promise<string> {
  const id = () => {
    const last = new URL(page.url()).pathname.split('/').at(-1) ?? '';
    return /^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$/.test(last) && !except.includes(last) ? last : '';
  };
  await expect.poll(id, { message: 'the Studio moves to the page of the new run', timeout: 60_000 }).not.toBe('');
  return id();
}

/** Opens a page of the namespace from the menu: Tab to its link and Enter, and wait for the page's own heading. */
export async function openFromMenu(page: Page, item: string, heading: string | RegExp): Promise<void> {
  await activate(page, page.getByRole('navigation', { name: 'Main' }).getByRole('link', { name: item, exact: true }));
  await expect(page.getByRole('heading', { level: 1, name: heading })).toBeVisible();
}

/**
 * Presses `opener` with the keyboard and checks what a modal dialog owes a person who cannot see: focus moved into it, and
 * the dialog has a name. Returns the dialog.
 */
export async function openDialog(page: Page, opener: Locator, name: string | RegExp): Promise<Locator> {
  await activate(page, opener);
  const dialog = page.getByRole('dialog', { name });
  await expect(dialog).toBeVisible();
  await expect.poll(() => dialog.evaluate((root) => root.contains(document.activeElement)), { message: 'focus moves into the dialog that opened' }).toBe(true);
  return dialog;
}

/** Escape closes the dialog and focus goes back to the control that opened it. */
export async function closeWithEscape(page: Page, dialog: Locator, opener: Locator): Promise<void> {
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(opener).toBeFocused();
}
