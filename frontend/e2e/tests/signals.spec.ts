import { expect, test } from '@playwright/test';
import { env } from '../support/env';
import { Api, IR_BASE } from '../support/platform';
import { failOnBrowserErrors, goTo, signIn } from '../support/studio';

test.use({ trace: 'off' });
test('sends an explicit signal to a real waiting run and keeps the journal', async ({ page }) => {
  const errors = failOnBrowserErrors(page);
  const api = new Api();
  const name = `studio-signal-${Date.now()}`;
  const workflow = await api.publish(name, {
    ...IR_BASE, metadata: { name }, entrypoint: 'approval', triggers: [{ kind: 'manual' }],
    nodes: [{ id: 'approval', type: 'waitEvent', eventType: 'approval.decided', correlationExpr: '"studio-signal"', timeoutMs: 60000, next: 'done' }, { id: 'done', type: 'end' }],
  });
  const run = await api.start(workflow.id);
  await api.until(run, 'WAITING');
  await signIn(page, env.tokens.admin);
  await page.getByLabel('Namespace', { exact: true }).selectOption(env.namespace);
  await goTo(page, `/runs/${run}`);
  await page.getByRole('button', { name: 'Send signal', exact: true }).click();
  await page.getByLabel('Signal name', { exact: true }).fill('approval.decided');
  await page.getByLabel('Payload (JSON object)', { exact: true }).fill('{"approved":true}');
  await expect(page.getByText(/every waiting step/)).toBeVisible();
  await page.getByRole('button', { name: 'Confirm signal', exact: true }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await api.until(run, 'SUCCEEDED');
  await expect(page.getByRole('button', { name: 'Send signal', exact: true })).toHaveCount(0);
  const journal = await api.journal(run);
  expect(journal.some((event) => event.eventType === 'wait.satisfied' && event.stepId === 'approval')).toBe(true);
  expect(errors).toEqual([]);
});
