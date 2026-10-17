import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { env } from '../support/env';
import { Api, IR_BASE } from '../support/platform';
import { failOnBrowserErrors, goTo, signIn } from '../support/studio';

test.use({ viewport: { width: 1280, height: 800 } });

const PUBLIC = 'PUBLIC-DISCLOSURE-CANARY';
const INTERNAL = 'INTERNAL-DISCLOSURE-CANARY';
const PII = 'PII-DISCLOSURE-CANARY';
const SECRET = 'SECRET-DISCLOSURE-CANARY';

async function openValues(page: Page, run: string): Promise<string> {
  await goTo(page, `/runs/${run}`);
  const action = page.getByRole('button', { name: 'View permitted run input values' });
  await expect(action).toBeVisible();
  await action.focus();
  const response = page.waitForResponse((candidate) => candidate.url().endsWith(`/v1/runs/${run}/payload-disclosures`) && candidate.request().method() === 'POST');
  await action.press('Enter');
  return (await response).text();
}

async function browserStores(page: Page): Promise<string> {
  return page.evaluate(async () => {
    const local = Object.entries(localStorage).map(([key, value]) => `${key}:${value}`);
    const session = Object.entries(sessionStorage).map(([key, value]) => `${key}:${value}`);
    const databases: unknown[] = [];
    for (const info of await indexedDB.databases()) {
      const databaseName = info.name;
      if (!databaseName) continue;
      const database = await new Promise<IDBDatabase>((resolve, reject) => {
        const request = indexedDB.open(databaseName);
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      try {
        for (const name of Array.from(database.objectStoreNames)) {
          const records = await new Promise<unknown[]>((resolve, reject) => {
            const request = database.transaction(name, 'readonly').objectStore(name).getAll();
            request.onsuccess = () => resolve(request.result as unknown[]);
            request.onerror = () => reject(request.error);
          });
          databases.push({ database: info.name, store: name, records });
        }
      } finally {
        database.close();
      }
    }
    return JSON.stringify({ local, session, databases });
  });
}

async function revealValues(page: Page, run: string): Promise<string> {
  const reveal = page.getByRole('button', { name: 'Reveal personal data' });
  await expect(reveal).toBeVisible();
  await reveal.focus();
  await reveal.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Reveal personal data?' });
  await expect(dialog).toBeVisible();
  const response = page.waitForResponse((candidate) => candidate.url().endsWith(`/v1/runs/${run}/payload-disclosures`) && candidate.request().method() === 'POST');
  const confirm = dialog.getByRole('button', { name: 'Reveal personal data' });
  await confirm.focus();
  await confirm.press('Enter');
  const wire = await (await response).text();
  expect(wire).toContain(PII);
  expect(wire).not.toContain(SECRET);
  await expect(page.getByText(new RegExp(PII))).toBeVisible();
  return wire;
}

test('viewer, developer, operator and revealer see only the classified projection and revealed values remain ephemeral', async ({ page }) => {
  test.setTimeout(180_000);
  const problems = failOnBrowserErrors(page);
  const network: Array<Promise<string | null>> = [];
  page.on('response', (response) => {
    if (response.url().includes('/v1/') && response.headers()['content-type']?.includes('application/json')) {
      network.push(response.text().catch(() => null)); // A navigation may cancel an in-flight read.
    }
  });
  const admin = new Api('admin');
  const foreign = new Api('foreign');
  const foreignNamespaces = await foreign.call<{ items: Array<{ slug: string }> }>('GET', '/namespaces', undefined, 200);
  if (!foreignNamespaces.body.items.some((namespace) => namespace.slug === env.namespace)) {
    const workspace = await foreign.call<{ id: string }>('POST', '/workspaces', { slug: `disclosure-${Date.now()}` }, 201);
    await foreign.call('POST', `/workspaces/${workspace.body.id}/namespaces`, { slug: env.namespace, environment: 'dev' }, 201);
  }
  const name = `payload-disclosure-${Date.now()}`;
  const workflow = await admin.publish(name, {
    ...IR_BASE,
    metadata: { name },
    entrypoint: 'done',
    inputSchema: {
      type: 'object',
      additionalProperties: false,
      'x-sagat-classification': 'public',
      properties: {
        publicValue: { type: 'string' },
        internalValue: { type: 'string', 'x-sagat-classification': 'internal' },
        personalValue: { type: 'string', 'x-sagat-classification': 'pii' },
        secretValue: { type: 'string', 'x-sagat-classification': 'secret' },
      },
      required: ['publicValue', 'internalValue', 'personalValue', 'secretValue'],
    },
    nodes: [{ id: 'done', type: 'end' }],
  });
  const run = await admin.start(workflow.id, { publicValue: PUBLIC, internalValue: INTERNAL, personalValue: PII, secretValue: SECRET });

  await signIn(page, env.tokens.viewer);
  const viewerWire = await openValues(page, run);
  await expect(page.getByRole('tree', { name: 'Classified payload values' })).toContainText('○ public');
  await expect(page.getByText(new RegExp(PUBLIC))).toBeVisible();
  expect(viewerWire).toContain(PUBLIC);
  for (const hidden of [INTERNAL, PII, SECRET]) {
    expect(viewerWire).not.toContain(hidden);
    await expect(page.getByText(new RegExp(hidden))).toHaveCount(0);
  }

  await signIn(page, env.tokens.developer);
  const developerWire = await openValues(page, run);
  await expect(page.getByText(new RegExp(INTERNAL))).toBeVisible();
  expect(developerWire).toContain(PUBLIC);
  expect(developerWire).toContain(INTERNAL);
  expect(developerWire).not.toContain(PII);
  expect(developerWire).not.toContain(SECRET);
  await expect(page.getByRole('button', { name: 'Reveal personal data' })).toHaveCount(0);

  await signIn(page, env.tokens.operator);
  const operatorWire = await openValues(page, run);
  expect(operatorWire).not.toContain(PII);
  expect(operatorWire).not.toContain(SECRET);
  await expect(page.getByRole('tree', { name: 'Classified payload values' })).toContainText('[withheld: pii]');
  await expect(page.getByRole('tree', { name: 'Classified payload values' })).toContainText('△ pii');

  const namespaces = await admin.call<{ items: Array<{ id: string; slug: string }> }>('GET', '/namespaces', undefined, 200);
  const namespaceId = namespaces.body.items.find((namespace) => namespace.slug === env.namespace)?.id;
  expect(namespaceId, `namespace ${env.namespace} is listed`).toBeTruthy();
  await admin.call('PUT', `/namespaces/${namespaceId}/payload-reveal-grants/dev`, undefined, 200);

  await signIn(page, env.tokens.developer);
  await openValues(page, run);
  const revealWire = await revealValues(page, run);
  expect(revealWire).toContain(PII);
  expect(revealWire).not.toContain(SECRET);
  await expect(page.getByText(new RegExp(PII))).toBeVisible();
  await expect(page.getByText(new RegExp(SECRET))).toHaveCount(0);
  expect(await browserStores(page)).not.toContain(PII);

  await page.getByRole('button', { name: 'Close values' }).click();
  await expect(page.getByText(new RegExp(PII))).toHaveCount(0);
  expect(await browserStores(page)).not.toContain(PII);

  await openValues(page, run);
  await revealValues(page, run);
  await goTo(page, '/runs');
  await expect(page.getByText(new RegExp(PII))).toHaveCount(0);
  expect(await browserStores(page)).not.toContain(PII);

  const secondRun = await admin.start(workflow.id, { publicValue: PUBLIC, internalValue: INTERNAL, personalValue: PII, secretValue: SECRET });
  await openValues(page, run);
  await revealValues(page, run);
  await goTo(page, `/runs/${secondRun}`);
  await expect(page.getByRole('button', { name: 'View permitted run input values' })).toBeVisible();
  await expect(page.getByText(new RegExp(PII))).toHaveCount(0);
  expect(await browserStores(page)).not.toContain(PII);

  await openValues(page, run);
  await revealValues(page, run);
  await signIn(page, env.tokens.viewer);
  const newSession = await openValues(page, run);
  expect(newSession).not.toContain(PII);
  await expect(page.getByText(new RegExp(PII))).toHaveCount(0);
  expect(await browserStores(page)).not.toContain(PII);

  await signIn(page, env.tokens.developer);
  await openValues(page, run);
  await revealValues(page, run);
  await signIn(page, env.tokens.foreign);
  await expect(page.getByText(new RegExp(PII))).toHaveCount(0);
  expect(await browserStores(page)).not.toContain(PII);
  const captured = (await Promise.all(network)).filter((body) => body !== null);
  expect(captured.some((body) => body.includes('classified-v1'))).toBe(true);
  for (const body of captured) expect(body).not.toContain(SECRET);
  expect(problems).toEqual([]);
});
