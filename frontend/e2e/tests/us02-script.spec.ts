import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { env } from '../support/env';
import { Api } from '../support/platform';
import { failOnBrowserErrors, signIn } from '../support/studio';

test.use({ viewport: { width: 1440, height: 900 } });

const SOURCE = 'export default async function handler(input: { n: number }) {\n  return { double: input.n * 2 };\n}\n';

async function typeInto(page: Page, label: string | RegExp, text: string): Promise<void> {
  const box = page.getByRole('textbox', { name: label });
  await box.fill(text);
}

test('US02 a developer edits the code of a node, tries it in the sandbox, publishes a version and uses it in a workflow that is only publishable because the code was built', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const api = new Api('developer');
  await signIn(page, env.tokens.developer);

  // A script: the editor is Monaco under the page's policy; the plain text box is the way for anyone who cannot use it.
  await page.getByRole('link', { name: 'Scripts' }).click();
  await page.getByRole('button', { name: 'New script' }).click();
  await page.getByRole('dialog').getByRole('textbox', { name: 'Name' }).fill('double-it');
  await page.getByRole('dialog').getByRole('button', { name: 'Create script' }).click();
  await page.waitForURL(/\/scripts\/[0-9a-f-]{36}$/);
  await expect(page.getByRole('heading', { name: 'double-it', level: 1 })).toBeVisible();
  await expect(page.getByRole('textbox', { name: /^Source of double-it/ })).toBeAttached({ timeout: 30_000 });

  await page.getByRole('region', { name: 'Source' }).getByRole('button', { name: 'Use a plain text box' }).click();
  await typeInto(page, /^Source of double-it/, SOURCE);
  await expect(page.getByText('All changes saved.')).toBeVisible({ timeout: 20_000 });

  // Not tried yet: the page says so, and does not claim a result it does not have.
  await expect(page.getByText('This draft has not been tried in the sandbox.')).toBeVisible();
  await page.getByRole('tabpanel', { name: 'Try it in the sandbox' }).getByRole('button', { name: 'Use a plain text box' }).click();
  await typeInto(page, 'Test input (JSON)', '{"n": 21}');
  await page.getByRole('button', { name: 'Run in the sandbox' }).click();
  const result = page.getByTestId('test-result');
  await expect(result).toContainText('Succeeded', { timeout: 120_000 });
  await expect(result).toContainText('"double": 42');

  // Publish a version; the platform builds it; the page shows the artifact digest a workflow pins.
  await page.getByRole('button', { name: 'Publish version…' }).click();
  await page.getByRole('dialog').getByRole('button', { name: /^Publish/ }).last().click();
  const digest = await page.getByText(/sha256:[0-9a-f]{64}/).first().innerText({ timeout: 180_000 });
  expect(digest.replace(/\s+/g, '')).toMatch(/^sha256:[0-9a-f]{64}$/);
  await page.getByRole('dialog').getByRole('button', { name: 'Close' }).click();

  // A workflow with a script node: it can be published because the code was built. The picker offers the version.
  await page.getByRole('link', { name: 'Workflows' }).click();
  await page.getByRole('button', { name: 'New workflow' }).click();
  await page.getByRole('dialog').getByRole('textbox', { name: 'Name' }).fill('scripted-flow');
  await page.getByRole('dialog').getByRole('button', { name: 'Create workflow' }).click();
  await page.waitForURL(/\/workflows\/[0-9a-f-]{36}$/);
  const id = page.url().split('/').pop() as string;
  await page.getByRole('button', { name: 'Add node' }).click();
  await page.getByRole('dialog', { name: 'Add a node' }).getByRole('button', { name: 'Script', exact: true }).click();
  await expect(page.getByRole('complementary', { name: 'Inspector' }).getByRole('heading', { level: 2 })).toContainText('Script');
  const inspector = page.getByRole('complementary', { name: 'Inspector' });

  // A digest nobody built is a problem on its node, in the platform's words, and publishing is blocked.
  await inspector.getByRole('textbox', { name: 'Artifact' }).fill(`sha256:${'a'.repeat(64)}`);
  await addEnd(page);
  await expect(page.getByRole('region', { name: 'Validation' })).toContainText('UNKNOWN_SCRIPT', { timeout: 30_000 });
  await expect(page.getByRole('button', { name: 'Publish…' })).toBeDisabled();

  // The picker offers what was built, and the node pins exactly that digest.
  await page.getByRole('button', { name: 'List' }).click();
  await page.getByRole('button', { name: 'Edit script_1' }).click();
  await inspector.getByRole('combobox', { name: 'Script', exact: true }).selectOption({ label: 'double-it (typescript)' });
  await inspector.getByRole('combobox', { name: 'Built version' }).selectOption({ label: 'v1 · typescript' });
  await expect(inspector.getByRole('textbox', { name: 'Artifact' })).toHaveValue(digest.replace(/\s+/g, ''), { timeout: 20_000 });
  await expect(page.getByTestId('validation-summary')).toContainText('0 problems', { timeout: 30_000 });

  // Publishing works, and what the platform stored pins the digest of the code that was built.
  await page.getByRole('button', { name: 'Publish…' }).click();
  await page.getByRole('dialog', { name: 'Publish workflow' }).getByRole('button', { name: 'Publish', exact: true }).click();
  await expect(page.getByText('Published version 1.').first()).toBeVisible();
  const stored = (await api.call<{ ir: { nodes: Array<{ type: string; artifactDigest?: string }> } }>('GET', `/workflows/${id}`, undefined, 200)).body;
  expect(stored.ir.nodes.find((n) => n.type === 'script')?.artifactDigest).toBe(digest.replace(/\s+/g, ''));
  expect(problems).toEqual([]);
});

async function addEnd(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Add node' }).click();
  await page.getByRole('dialog', { name: 'Add a node' }).getByRole('button', { name: 'End', exact: true }).click();
}
