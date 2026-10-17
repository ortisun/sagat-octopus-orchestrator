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
    const databases = typeof indexedDB.databases === 'function' ? (await indexedDB.databases()).map((database) => database.name ?? '') : [];
    return JSON.stringify({ local, session, databases });
  });
}

test('viewer, developer, operator and revealer see only the classified projection and revealed values remain ephemeral', async ({ page }) => {
  test.setTimeout(180_000);
  const problems = failOnBrowserErrors(page);
  const admin = new Api('admin');
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
  const reveal = page.getByRole('button', { name: 'Reveal personal data' });
  await expect(reveal).toBeVisible();
  await reveal.focus();
  await reveal.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Reveal personal data?' });
  await expect(dialog).toBeVisible();
  const revealResponse = page.waitForResponse((candidate) => candidate.url().endsWith(`/v1/runs/${run}/payload-disclosures`) && candidate.request().method() === 'POST');
  const confirm = dialog.getByRole('button', { name: 'Reveal personal data' });
  await confirm.focus();
  await confirm.press('Enter');
  const revealWire = await (await revealResponse).text();
  expect(revealWire).toContain(PII);
  expect(revealWire).not.toContain(SECRET);
  await expect(page.getByText(new RegExp(PII))).toBeVisible();
  await expect(page.getByText(new RegExp(SECRET))).toHaveCount(0);
  expect(await browserStores(page)).not.toContain(PII);

  await page.getByRole('button', { name: 'Close values' }).click();
  await expect(page.getByText(new RegExp(PII))).toHaveCount(0);
  expect(await browserStores(page)).not.toContain(PII);
  expect(problems).toEqual([]);
});
