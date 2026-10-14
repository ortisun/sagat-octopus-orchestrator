import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { env } from '../support/env';
import { Api, IR_BASE } from '../support/platform';
import { failOnBrowserErrors, goTo, signIn } from '../support/studio';

test.use({ viewport: { width: 1440, height: 900 } });

/** Text that would run if the page ever treated what it is shown as markup. */
const HOSTILE = '<img src=x onerror="window.__pwned=1"><script>window.__pwned=2</script>';

/** Fails the journey when a script of the page's data ran, or a dialog (alert, confirm, prompt) opened. */
function watchForInjection(page: Page): { dialogs: string[] } {
  const seen = { dialogs: [] as string[] };
  page.on('dialog', (dialog) => {
    seen.dialogs.push(dialog.message());
    void dialog.dismiss();
  });
  return seen;
}

async function nothingRan(page: Page): Promise<void> {
  expect(await page.evaluate(() => (window as unknown as { __pwned?: number }).__pwned)).toBeUndefined();
  expect(await page.locator('img[src="x"]').count()).toBe(0);
  expect(await page.locator('script:not([src])').count()).toBe(0);
}

test('SEC text that comes from outside is shown as text and never runs: a description, a correlation id, a release note, and the id and activity of a node', async ({ page }) => {
  test.setTimeout(180_000);
  const problems = failOnBrowserErrors(page);
  const injected = watchForInjection(page);
  const admin = new Api('admin');
  const created = await admin.call<{ id: string }>('POST', '/workflows', {
    name: 'sec-text',
    ir: {
      ...IR_BASE,
      metadata: { name: 'sec-text', description: HOSTILE },
      entrypoint: 'a',
      nodes: [{ id: 'a', type: 'activity', activity: 'sec.a', timeoutMs: 1000 }],
    },
  }, 201);
  await admin.call('POST', `/workflows/${created.body.id}/publish`, { draftRevision: 1, releaseNotes: HOSTILE }, 201);
  const run = await admin.start(created.body.id, {}, { correlationId: HOSTILE });

  // A draft is saved whatever it holds, so the id of a node and the name of its activity can be anything.
  const ids = await admin.call<{ id: string }>('POST', '/workflows', {
    name: 'sec-ids',
    ir: { ...IR_BASE, metadata: { name: 'sec-ids' }, entrypoint: HOSTILE, nodes: [{ id: HOSTILE, type: 'activity', activity: HOSTILE, timeoutMs: 1000 }] },
  }, 201);

  await signIn(page, env.tokens.developer);
  // The run: the correlation id is on the page, as the characters it is made of.
  await goTo(page, `/runs/${run}`);
  await expect(page.getByRole('heading', { level: 1 })).toContainText(run);
  await page.getByText('Run details', { exact: true }).click();
  await expect(page.getByText(HOSTILE, { exact: true }).first()).toBeVisible();
  await nothingRan(page);

  await page.getByRole('tab', { name: 'Table' }).click();
  await page.getByRole('tab', { name: 'Timeline' }).click();
  await expect(page.getByText(/journal events are read/)).toBeVisible({ timeout: 20_000 });
  await nothingRan(page);

  // The workflow: the description in the list and in the editor, the release note in the versions.
  await page.getByRole('link', { name: 'Workflows' }).click();
  await expect(page.getByRole('link', { name: 'sec-text', exact: true })).toBeVisible();
  await nothingRan(page);
  await page.getByRole('link', { name: 'sec-text', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'sec-text', level: 1 })).toBeVisible();
  await expect(page.getByRole('textbox', { name: 'Description' })).toHaveValue(HOSTILE);
  await page.getByRole('tab', { name: 'Versions' }).click();
  await expect(page.getByText(HOSTILE, { exact: true }).first()).toBeVisible();
  await nothingRan(page);

  // A workflow whose node is called by the hostile text: the canvas, the list, the inspector and the platform's own
  // problems about it (which quote the id) show it as text.
  await goTo(page, `/workflows/${ids.body.id}`);
  await expect(page.getByRole('heading', { name: 'sec-ids', level: 1 })).toBeVisible();
  await expect(page.getByTestId('validation-summary')).toHaveText(/problem/, { timeout: 20_000 });
  await nothingRan(page);
  await page.getByRole('button', { name: 'List' }).click();
  // The id is clipped (a name is not a paragraph), and what is shown of it is its characters.
  await expect(page.getByText('<img src=x onerror="window.__pwned=1">', { exact: false }).first()).toBeVisible();
  await nothingRan(page);
  await page.getByRole('button', { name: /^Edit /, includeHidden: false }).first().click();
  await expect(page.getByRole('complementary', { name: 'Inspector' })).toBeVisible();
  await nothingRan(page);

  expect(injected.dialogs).toEqual([]);
  expect(problems).toEqual([]);
});

test('SEC the token lives in the page\'s memory only, the page allows nothing but its own scripts, and a page load asks for the token again', async ({ page }) => {
  const reports: string[] = [];
  await page.addInitScript(() => {
    document.addEventListener('securitypolicyviolation', (event) => {
      (window as unknown as { __violations?: string[] }).__violations = [...((window as unknown as { __violations?: string[] }).__violations ?? []), event.violatedDirective];
    });
  });
  const response = await page.goto('./');
  const csp = response?.headers()['content-security-policy'] ?? '';
  for (const directive of ["default-src 'none'", "script-src 'self'", "base-uri 'none'", "frame-ancestors 'none'", "connect-src 'self'"]) expect(csp).toContain(directive);
  expect(csp).not.toContain("'unsafe-eval'");
  expect(csp).not.toMatch(/script-src[^;]*'unsafe-inline'/);
  expect(response?.headers()['x-content-type-options']).toBe('nosniff');
  expect(response?.headers()['referrer-policy']).toBe('no-referrer');

  await signIn(page, env.tokens.operator);
  const token = env.tokens.operator;
  // Nowhere the page can keep something across a load, and not in the address.
  const kept = await page.evaluate(async () => {
    const dump = (storage: Storage) => Object.keys(storage).map((key) => `${key}=${storage.getItem(key)}`).join('\n');
    const databases = 'databases' in indexedDB ? await indexedDB.databases() : [];
    const caches = await globalThis.caches.keys();
    return { local: dump(localStorage), session: dump(sessionStorage), cookie: document.cookie, url: location.href, databases: databases.map((d) => d.name), caches };
  });
  expect(kept.local).not.toContain(token);
  expect(kept.session).not.toContain(token);
  expect(kept.cookie).not.toContain(token);
  expect(kept.url).not.toContain(token);
  expect(kept.databases).toEqual([]);
  expect(kept.caches).toEqual([]);

  // The token is sent to the page's own origin and to nobody else.
  const hosts = new Set<string>();
  page.on('request', (request) => {
    if (request.headers().authorization) hosts.add(new URL(request.url()).origin);
  });
  await page.getByRole('link', { name: 'Runs' }).click();
  await expect(page.getByRole('heading', { name: 'Runs', level: 1 })).toBeVisible();
  expect([...hosts]).toEqual([env.origin]);

  // A script put into the page by something that got past the page's own code does not run: the policy forbids it.
  await page.evaluate(() => {
    const script = document.createElement('script');
    script.textContent = 'window.__inline = 1';
    document.body.append(script);
  });
  expect(await page.evaluate(() => (window as unknown as { __inline?: number }).__inline)).toBeUndefined();
  expect((await page.evaluate(() => (window as unknown as { __violations?: string[] }).__violations ?? [])).some((d) => d.startsWith('script-src'))).toBe(true);

  // A page load forgets the token: the sign-in is asked for again.
  await page.reload();
  await expect(page.getByLabel('Access token')).toBeVisible();
  expect(reports).toEqual([]);
});

test('SEC what belongs to another tenant is not found, from the API and from the Studio, and what is said about it holds nothing of it', async ({ page }) => {
  test.setTimeout(180_000);
  const admin = new Api('admin');
  const wf = await admin.call<{ id: string }>('POST', '/workflows', {
    name: 'acme-only',
    ir: { ...IR_BASE, metadata: { name: 'acme-only', description: 'visible to acme only' }, entrypoint: 'a', nodes: [{ id: 'a', type: 'activity', activity: 'acme.secret.step', timeoutMs: 1000 }] },
  }, 201);
  await admin.call('POST', `/workflows/${wf.body.id}/publish`, { draftRevision: 1 }, 201);
  const run = await admin.start(wf.body.id, { cardNumber: '4111111111111111' }, { correlationId: 'acme-correlation-7' });

  // The API: the other tenant's administrator gets nothing, and the same answer as for an id that does not exist.
  const bob = new Api('foreign');
  const real = await bob.call('GET', `/runs/${run}`);
  const missing = await bob.call('GET', '/runs/00000000-0000-7000-8000-000000000000');
  expect(real.status).toBe(missing.status);
  expect([403, 404]).toContain(real.status);
  expect(JSON.stringify(real.body)).not.toMatch(/acme|4111|correlation/);
  const listed = await bob.call<{ items?: Array<{ name: string }> }>('GET', '/workflows');
  expect(JSON.stringify(listed.body)).not.toContain('acme-only');

  // The Studio: signed in as that administrator, nothing of acme is on any page, and the errors say so in words.
  await signIn(page, env.tokens.foreign);
  await goTo(page, `/runs/${run}`);
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  // SC-007-004: what is said carries the code and the trace id to quote, and a way on; none of what it is about.
  await expect(page.getByText(/Code (NOT_FOUND|FORBIDDEN)\./)).toBeVisible();
  await expect(page.getByText(/Trace [0-9a-f]{32}\./)).toBeVisible();
  await goTo(page, `/workflows/${wf.body.id}`);
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  const text = await page.locator('body').innerText();
  for (const secret of ['acme-only', 'acme.secret.step', 'acme-correlation-7', '4111111111111111', 'visible to acme only']) expect(text).not.toContain(secret);
});
