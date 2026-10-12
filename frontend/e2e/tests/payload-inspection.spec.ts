import { expect, test } from '@playwright/test';
import { env } from '../support/env';
import { Api, IR_BASE } from '../support/platform';
import { failOnBrowserErrors, goTo, signIn } from '../support/studio';

test.use({ viewport: { width: 1280, height: 800 } });

test('an operator inspects useful redacted input structure with the keyboard and no key or value reaches the browser', async ({ page }) => {
  const problems = failOnBrowserErrors(page);
  const api = new Api('admin');
  const name = `payload-inspection-${Date.now()}`;
  const workflow = await api.publish(name, {
    ...IR_BASE,
    metadata: { name },
    entrypoint: 'done',
    nodes: [{ id: 'done', type: 'end' }],
  });
  const canaries = {
    'ordinary-private-key': 'ORDINARY-SECRET-CANARY',
    'schema-public-name': { default: 18446744073709550000, examples: ['EXAMPLE-CANARY'] },
    truth: true,
    'unicode-key-☃': [null, 'UNICODE-VALUE-☃'],
  };
  const run = await api.start(workflow.id, canaries);

  await signIn(page, env.tokens.operator);
  await goTo(page, `/runs/${run}`);
  const action = page.getByRole('button', { name: 'Inspect redacted run input' });
  await expect(action).toBeVisible();
  await action.focus();
  const response = page.waitForResponse((candidate) => candidate.url().endsWith(`/v1/runs/${run}/payload-inspections`) && candidate.request().method() === 'POST');
  await action.press('Enter');
  const wire = await (await response).text();

  const tree = page.getByRole('tree', { name: 'Redacted payload structure' });
  await expect(tree).toBeVisible();
  await expect(tree).toContainText('Root');
  await expect(tree).toContainText('object');
  await expect(tree).toContainText('4 children');
  await expect(tree.getByRole('treeitem')).toHaveCount(10);
  await expect(page.getByText('Redacted structure only.')).toBeVisible();
  await expect(page.getByText('structure-only-v1')).toBeVisible();

  const dom = await page.locator('body').innerText();
  for (const canary of [
    'ordinary-private-key',
    'ORDINARY-SECRET-CANARY',
    'schema-public-name',
    'default',
    'examples',
    'EXAMPLE-CANARY',
    'truth',
    'unicode-key-☃',
    'UNICODE-VALUE-☃',
  ]) {
    expect(wire).not.toContain(canary);
    expect(dom).not.toContain(canary);
  }
  const inspector = page.getByRole('region', { name: 'Inspect redacted run input' });
  await expect(inspector.getByRole('button', { name: /copy|export|download/i })).toHaveCount(0);
  expect(problems).toEqual([]);
});
