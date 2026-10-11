import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { env } from '../support/env';
import { Api } from '../support/platform';
import { failOnBrowserErrors, signIn } from '../support/studio';

test.use({ viewport: { width: 1440, height: 900 } });

const editor = (page: Page) => page.getByRole('group', { name: 'Editor tools' });
const inspector = (page: Page) => page.getByRole('complementary', { name: 'Inspector' });

async function newWorkflow(page: Page, name: string): Promise<string> {
  await page.getByRole('link', { name: 'Workflows' }).click();
  await page.getByRole('button', { name: 'New workflow' }).click();
  await page.getByRole('dialog').getByRole('textbox', { name: 'Name' }).fill(name);
  await page.getByRole('dialog').getByRole('button', { name: 'Create workflow' }).click();
  await page.waitForURL(/\/workflows\/[0-9a-f-]{36}$/);
  await expect(page.getByRole('heading', { name, level: 1 })).toBeVisible();
  return page.url().split('/').pop() as string;
}

async function addNode(page: Page, kind: string): Promise<void> {
  await page.getByRole('button', { name: 'Add node' }).click();
  await page.getByRole('dialog', { name: 'Add a node' }).getByRole('button', { name: kind, exact: true }).click();
}

async function saved(page: Page): Promise<void> {
  await expect(editor(page)).toContainText(/Saved · revision \d+/, { timeout: 20_000 });
}

test('US01 an author builds a workflow without programming: the platform checks it as they work, it is saved, and publishing shows what changes', async ({ page }) => {
  const problems = failOnBrowserErrors(page);
  const api = new Api('developer');
  await signIn(page, env.tokens.developer);

  const id = await newWorkflow(page, 'invoice-flow');
  // Empty: the platform's own check says so, and publishing is blocked with the reason.
  await expect(page.getByText(/Publishing is blocked: the platform found \d+ problems? in this workflow/)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Publish…' })).toBeDisabled();

  // Two steps and an end, added from the keyboard-reachable menu; each is linked to the one it was added after.
  await addNode(page, 'Step');
  await inspector(page).getByRole('textbox', { name: 'Activity', exact: true }).fill('invoice.check');
  await inspector(page).getByRole('textbox', { name: 'Timeout amount' }).fill('5');
  await inspector(page).getByRole('combobox', { name: 'Timeout unit' }).selectOption('s');
  await addNode(page, 'Step');
  await inspector(page).getByRole('textbox', { name: 'Activity', exact: true }).fill('invoice.send');
  await inspector(page).getByRole('textbox', { name: 'Timeout amount' }).fill('10');
  await inspector(page).getByRole('combobox', { name: 'Timeout unit' }).selectOption('s');
  await addNode(page, 'End');

  // A link that would make a cycle is refused, with a message, and nothing changes.
  await page.getByRole('button', { name: 'List' }).click();
  await page.getByRole('button', { name: 'Edit activity_2' }).click();
  await inspector(page).getByRole('combobox', { name: 'Next' }).selectOption('activity_1');
  await expect(page.getByRole('status').filter({ hasText: /cycle/i }).first()).toBeVisible();
  await expect(inspector(page).getByRole('combobox', { name: 'Next' })).toHaveValue('end_1');

  // The platform finds nothing wrong; the advice about timeouts is gone because the author set them.
  await expect(page.getByTestId('validation-summary')).toHaveText('0 problems · 0 warnings · 0 pieces of advice', { timeout: 20_000 });
  await expect(page.getByRole('button', { name: 'Publish…' })).toBeEnabled();
  await saved(page);

  // What the platform stored is what the author built.
  const draft = (await api.call<{ ir: { entrypoint: string; nodes: Array<Record<string, unknown>> }; draftRevision: number }>('GET', `/workflows/${id}`, undefined, 200)).body;
  expect(draft.ir.entrypoint).toBe('activity_1');
  expect(draft.ir.nodes).toEqual([
    { id: 'activity_1', type: 'activity', activity: 'invoice.check', timeoutMs: 5000, next: 'activity_2' },
    { id: 'activity_2', type: 'activity', activity: 'invoice.send', timeoutMs: 10_000, next: 'end_1' },
    { id: 'end_1', type: 'end' },
  ]);

  // Publish: the dialog says what is new, then the version exists and is the active one.
  await page.getByRole('button', { name: 'Publish…' }).click();
  const dialog = page.getByRole('dialog', { name: 'Publish workflow' });
  await expect(dialog).toContainText('Added (3):');
  await expect(dialog).toContainText('Needs preparation (2)');
  await expect(dialog).toContainText('The platform found no breaking changes of the kinds it checks');
  await dialog.getByRole('textbox', { name: 'Release notes' }).fill('First version');
  await dialog.getByRole('button', { name: 'Publish', exact: true }).click();
  await expect(page.getByText('Published version 1.').first()).toBeVisible();
  const versions = (await api.call<{ items: Array<{ versionNumber: number }> }>('GET', `/workflows/${id}/versions`, undefined, 200)).body.items;
  expect(versions.map((v) => v.versionNumber)).toEqual([1]);

  // A change that breaks callers (an input that is now required) is named, and asks for a second confirmation.
  await page.getByRole('button', { name: 'Canvas' }).click();
  // From the keyboard, as the canvas documents it: focus the start card and press Enter.
  await page.getByRole('group', { name: /^Start:/ }).focus();
  await page.keyboard.press('Enter');
  await expect(inspector(page).getByRole('heading', { name: 'Workflow settings' })).toBeVisible();
  await inspector(page).getByRole('textbox', { name: 'Input schema' }).fill(JSON.stringify({ type: 'object', required: ['invoiceId'], properties: { invoiceId: { type: 'string' } } }));
  await inspector(page).getByRole('textbox', { name: 'Input schema' }).blur();
  await saved(page);
  await page.getByRole('button', { name: 'Publish…' }).click();
  const second = page.getByRole('dialog', { name: 'Publish workflow' });
  await expect(second).toContainText(/Breaking changes/i);
  await expect(second).toContainText(/invoiceId/);
  const publish = second.getByRole('button', { name: 'Publish', exact: true });
  await expect(publish).toBeDisabled();
  await second.getByRole('checkbox', { name: /I understand/ }).check();
  await expect(publish).toBeEnabled();
  await publish.click();
  await expect(page.getByText('Published version 2.').first()).toBeVisible();

  const diff = (await api.call<{ breaking: Array<{ code: string }> }>('GET', `/workflows/${id}/diff?from=1&to=2`, undefined, 200)).body;
  expect(diff.breaking.map((b) => b.code)).toEqual(['INPUT_REQUIRED_ADDED']);
  expect(problems).toEqual([]);
});

test('US01 two people editing the same draft: the second to save is shown the difference and nothing of the first is overwritten', async ({ browser, page }) => {
  const api = new Api('admin');
  const wf = await api.call<{ id: string }>('POST', '/workflows', {
    name: 'shared-flow',
    ir: { apiVersion: 'sagat-octopus-orchestrator.io/v1', kind: 'Workflow', metadata: { name: 'shared-flow' }, entrypoint: 'a', nodes: [{ id: 'a', type: 'activity', activity: 'shared.a', timeoutMs: 1000 }] },
  }, 201);
  const id = wf.body.id;

  await signIn(page, env.tokens.developer);
  await page.getByRole('link', { name: 'Workflows' }).click();
  await page.getByRole('link', { name: 'shared-flow' }).click();
  await expect(page.getByRole('heading', { name: 'shared-flow', level: 1 })).toBeVisible();
  await page.getByRole('button', { name: 'List' }).click();
  await page.getByRole('button', { name: 'Edit a' }).click();

  // Somebody else saves first.
  const other = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const otherPage = await other.newPage();
  await signIn(otherPage, env.tokens.admin);
  await otherPage.getByRole('link', { name: 'Workflows' }).click();
  await otherPage.getByRole('link', { name: 'shared-flow' }).click();
  await otherPage.getByRole('button', { name: 'List' }).click();
  await otherPage.getByRole('button', { name: 'Edit a' }).click();
  await inspector(otherPage).getByRole('textbox', { name: 'Activity', exact: true }).fill('shared.theirs');
  await saved(otherPage);

  // Our author edits what they opened and the save is refused as stale: a dialog, with the differences.
  await inspector(page).getByRole('textbox', { name: 'Activity', exact: true }).fill('shared.mine');
  const conflict = page.getByRole('dialog', { name: /Someone changed this draft/ });
  await expect(conflict).toBeVisible({ timeout: 20_000 });
  // The dialog says what differs (the node and the field), and offers the three ways out.
  await expect(conflict).toContainText(/Changed:\s*a\s*\(activity\)/);
  for (const way of ['Take theirs', 'Keep mine and save over theirs', 'Save mine as a copy']) {
    await expect(conflict.getByRole('button', { name: way })).toBeVisible();
  }
  const stored = (await api.call<{ ir: { nodes: Array<{ activity: string }> } }>('GET', `/workflows/${id}`, undefined, 200)).body;
  expect(stored.ir.nodes[0]?.activity).toBe('shared.theirs');

  // Take theirs: the page shows what is stored and nothing of ours was written.
  await conflict.getByRole('button', { name: /Take theirs/ }).click();
  await expect(conflict).toBeHidden();
  await page.getByRole('button', { name: 'Edit a' }).click();
  await expect(inspector(page).getByRole('textbox', { name: 'Activity', exact: true })).toHaveValue('shared.theirs');
  const after = (await api.call<{ ir: { nodes: Array<{ activity: string }> } }>('GET', `/workflows/${id}`, undefined, 200)).body;
  expect(after.ir.nodes[0]?.activity).toBe('shared.theirs');
  await other.close();
});
