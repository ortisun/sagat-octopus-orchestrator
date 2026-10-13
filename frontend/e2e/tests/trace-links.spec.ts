import { expect, test } from '@playwright/test';
import { env } from '../support/env';
import { Api, IR_BASE } from '../support/platform';
import { failOnBrowserErrors, goTo, signIn } from '../support/studio';

test.use({ trace: 'off' });

function oneStep(name: string): Record<string, unknown> {
  return {
    ...IR_BASE,
    metadata: { name },
    entrypoint: 'done',
    triggers: [{ kind: 'manual' }],
    nodes: [{ id: 'done', type: 'end' }],
  };
}

async function runWithTrace(name: string): Promise<{ runId: string; traceId: string }> {
  name = `${name}-${crypto.randomUUID()}`;
  const api = new Api('admin');
  const workflow = await api.publish(name, oneStep(name));
  const runId = await api.start(workflow.id);
  const run = (await api.call<{ traceId?: string }>('GET', `/runs/${runId}`, undefined, 200)).body;
  expect(run.traceId).toMatch(/^(?!0{32}$)[0-9a-f]{32}$/);
  return { runId, traceId: run.traceId as string };
}

test('a runtime trace template adds an explicit safe link without contacting the tracing tool', async ({ page }) => {
  const problems = failOnBrowserErrors(page);
  const { runId, traceId } = await runWithTrace('trace-link-configured');
  const template = 'https://traces.example.test/explore?trace={traceId}';
  let configReads = 0;
  const traceRequests: string[] = [];
  await page.route(`${env.origin}/studio/config.json`, async (route) => {
    configReads += 1;
    await route.fulfill({ json: { oidc: null, traceUrl: template } });
  });
  page.on('request', (request) => {
    if (request.url().startsWith('https://traces.example.test/')) traceRequests.push(request.url());
  });

  await signIn(page, env.tokens.operator);
  await goTo(page, `/runs/${runId}`);
  await page.getByText('Run details', { exact: true }).click();
  const link = page.getByRole('link', { name: 'Open trace' });
  await expect(link).toHaveAttribute('href', `https://traces.example.test/explore?trace=${traceId}`);
  await expect(link).toHaveAttribute('target', '_blank');
  await expect(link).toHaveAttribute('rel', /noopener/);
  await expect(link).toHaveAttribute('rel', /noreferrer/);
  await expect(page.getByRole('button', { name: 'Copy trace id' })).toBeVisible();
  expect(configReads).toBe(1);
  expect(traceRequests).toEqual([]);
  expect(problems).toEqual([]);
});

test('without runtime trace config the copy action remains and no link is invented', async ({ page }) => {
  const problems = failOnBrowserErrors(page);
  const { runId } = await runWithTrace('trace-link-copy-only');
  await page.route(`${env.origin}/studio/config.json`, (route) => route.fulfill({ json: { oidc: null } }));

  await signIn(page, env.tokens.operator);
  await goTo(page, `/runs/${runId}`);
  await page.getByText('Run details', { exact: true }).click();
  const copy = page.getByRole('button', { name: 'Copy trace id' });
  await expect(copy).toBeVisible();
  await expect(page.getByRole('link', { name: 'Open trace' })).toHaveCount(0);
  await copy.click();
  await expect(page.getByRole('status').filter({ hasText: /Copied trace id|Could not copy trace id/ }).first()).toBeVisible();
  expect(problems).toEqual([]);
});
