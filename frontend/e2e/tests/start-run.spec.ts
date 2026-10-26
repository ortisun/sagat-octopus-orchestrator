import { expect, test } from '@playwright/test';
import { env } from '../support/env';
import { Api, IR_BASE } from '../support/platform';
import { failOnBrowserErrors, goTo, signIn } from '../support/studio';

test('Start run validates input and opens the durable run created by the real API', async ({ page }) => {
  const problems = failOnBrowserErrors(page);
  const api = new Api('admin');
  const name = 'start-from-studio';
  const workflow = await api.publish(name, {
    ...IR_BASE, metadata: { name }, entrypoint: 'done', nodes: [{ id: 'done', type: 'end' }],
  });
  await signIn(page, env.tokens.admin);
  await goTo(page, '/runs');
  await page.getByRole('button', { name: 'Start run…', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Start a run' });
  await dialog.getByLabel('Workflow', { exact: true }).selectOption(workflow.id);
  await dialog.getByLabel('Input (JSON object)').fill('{');
  await dialog.getByRole('button', { name: 'Start run', exact: true }).click();
  await expect(dialog.getByRole('alert')).toContainText('Enter a valid JSON object.');
  const input = { orderId: 'studio-order' };
  await dialog.getByLabel('Input (JSON object)').fill(JSON.stringify(input));
  const admitted = page.waitForResponse((response) => response.request().method() === 'POST' && new URL(response.url()).pathname === '/v1/runs');
  await dialog.getByRole('button', { name: 'Start run', exact: true }).click();
  const response = await admitted;
  expect(response.status()).toBe(202);
  expect(response.request().postDataJSON()).toEqual({ workflowId: workflow.id, input });
  const run = await response.json() as { runId: string; workflowId: string; startedBy: string };
  expect(run.workflowId).toBe(workflow.id);
  expect(run.startedBy).toBe('alice');
  await expect(page).toHaveURL(new RegExp(`/studio/runs/${run.runId}$`));
  await api.until(run.runId, 'SUCCEEDED');
  await expect(page.getByRole('heading', { level: 1 })).toContainText(run.runId);
  expect(problems).toEqual([]);
});
