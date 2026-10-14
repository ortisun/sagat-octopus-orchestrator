import { WorkerHost } from '@sagat-octopus-orchestrator/sdk';
import { expect, test } from '@playwright/test';
import { env } from '../support/env';
import { Api } from '../support/platform';
import { failOnBrowserErrors, signIn } from '../support/studio';

test.use({ trace: 'off' });

test('drains and resumes an isolated worker identity through Operations', async ({ page }) => {
  const errors = failOnBrowserErrors(page);
  const api = new Api('admin');
  const suffix = Date.now().toString(36);
  const namespaceSlug = `operations-${suffix}`;
  const workerName = `operations-worker-${suffix}`;
  const workspaces = await api.call<{ items: Array<{ id: string }> }>('GET', '/workspaces?limit=1', undefined, 200);
  const workspace = workspaces.body.items[0];
  if (!workspace) throw new Error('the Studio journey needs a workspace');
  const created = await api.call<{ id: string }>('POST', `/workspaces/${workspace.id}/namespaces`, { slug: namespaceSlug, environment: 'dev' }, 201);
  const identityContext = await new Api('worker').call<{ subject: string }>('GET', '/auth/context', undefined, 200);
  const bindingPath = `/namespaces/${created.body.id}/worker-bindings/${encodeURIComponent(identityContext.body.subject)}`;
  await api.call('PUT', bindingPath, { activities: ['operations.fixture'] }, 200);
  const host = new WorkerHost({
    grpcUrl: env.grpcUrl,
    namespace: namespaceSlug,
    token: env.tokens.worker,
    workerName,
    maxInflight: 1,
    heartbeatIntervalMs: 500,
  });
  host.registerActivity('operations.fixture', () => ({ ready: true }));
  let subject: string | undefined;
  let started = false;

  try {
    await host.start();
    started = true;
    await expect.poll(async () => {
      const workers = await api.call<{ items: Array<{ name: string; subject: string }> }>('GET', `/namespaces/${created.body.id}/workers`, undefined, 200);
      subject = workers.body.items.find((item) => item.name === workerName)?.subject;
      return subject;
    }).toBeTruthy();

    await signIn(page, env.tokens.admin);
    await page.getByLabel('Namespace', { exact: true }).selectOption(namespaceSlug);
    await page.getByRole('link', { name: 'Operations', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Operations', exact: true })).toBeVisible();
    await expect(page.getByText('No alerts are recorded for this namespace.')).toBeVisible();
    const identity = page.getByRole('group', { name: `Worker identity ${subject}` });
    await expect(identity).toContainText(workerName);

    await page.getByRole('button', { name: `Drain worker identity ${subject}` }).click();
    await expect(page.getByRole('dialog')).toContainText(`every session of ${subject} in ${namespaceSlug}`);
    await expect(page.getByRole('dialog')).toContainText('does not cancel attempts already in flight');
    await page.getByLabel('Reason').fill('Isolated Operations browser journey');
    await page.getByRole('button', { name: 'Confirm drain' }).click();
    await expect(identity.getByText('Drained', { exact: true })).toBeVisible();
    await expect.poll(async () => {
      const workers = await api.call<{ drains: Array<{ subject: string }> }>('GET', `/namespaces/${created.body.id}/workers`, undefined, 200);
      return workers.body.drains.some((drain) => drain.subject === subject);
    }).toBe(true);

    await page.getByRole('button', { name: `Resume worker identity ${subject}` }).click();
    await page.getByLabel('Reason').fill('Isolated Operations browser journey complete');
    await page.getByRole('button', { name: 'Confirm resume' }).click();
    await expect(identity.getByText('Accepting claims', { exact: true })).toBeVisible();
    await page.setViewportSize({ width: 375, height: 812 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    expect(errors).toEqual([]);
  } finally {
    if (subject) {
      await api.call('POST', `/namespaces/${created.body.id}/workers/${encodeURIComponent(subject)}/undrain`, { reason: 'Guaranteed browser journey cleanup' }).catch(() => undefined);
    }
    if (started) await host.stop();
    await api.call('DELETE', bindingPath).catch(() => undefined);
  }
});
