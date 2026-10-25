import { expect, test } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { signIn } from '../support/studio';

test('database catalog and OpenBao secrets survive a lost connection-save response', async ({ page }) => {
  const token = process.env['STUDIO_TOKEN_ADMIN'];
  if (!token) throw new Error('Run this journey through e2e_database_connections');
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => {
    // One failed resource is intentional: the route below drops the successful API response.
    if (['warning', 'error'].includes(message.type()) && !message.text().includes('net::ERR_FAILED')) errors.push(message.text());
  });
  const writes: string[] = [];
  page.on('request', request => {
    if (request.method() === 'PUT' && request.url().includes('/connections/')) writes.push(request.postData() ?? '');
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await signIn(page, token);
  await page.getByRole('link', { name: 'Connections', exact: true }).click();
  await page.getByRole('button', { name: 'New connection' }).click();
  await page.getByLabel('Name', { exact: true }).fill('orders-db');
  await page.getByLabel('Host', { exact: true }).fill('db.example.com');
  await page.getByLabel('Database', { exact: true }).fill('orders');
  await page.getByLabel('Credentials', { exact: true }).selectOption('new');
  await page.getByLabel('Username', { exact: true }).fill('orders-reader');
  await page.getByLabel('Password', { exact: true }).fill('browser-credential-canary-017');
  for (const width of [320,768,1024,1440]) {
    await page.setViewportSize({ width, height: 1000 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width + 1);
    const accessibility = await new AxeBuilder({ page }).withTags(['wcag2a','wcag2aa','wcag21aa']).analyze();
    expect(accessibility.violations.map(v => ({ id:v.id, targets:v.nodes.map(n => n.target) }))).toEqual([]);
    await page.screenshot({ path: test.info().outputPath(`connection-form-${width}.png`), fullPage: true });
  }
  // Traverse the remainder of the form with the keyboard, including its disclosure summary.
  for (let i=0; i<12 && !(await page.getByRole('button', { name:'Save connection',exact:true }).evaluate(e => e===document.activeElement)); i++) await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name:'Save connection',exact:true })).toBeFocused();
  // The API commits successfully, then the browser loses its acknowledgement.
  await page.route('**/v1/namespaces/*/connections/orders-db', async route => {
    const response=await route.fetch(); expect(response.status()).toBe(200);
    await route.abort('failed');
  }, { times:1 });
  await page.getByRole('button', { name:'Save connection',exact:true }).click();
  await expect(page.getByText(/Credentials saved as/)).toBeVisible();
  await expect(page.getByRole('alert')).toBeVisible();
  await expect(page.getByLabel('Password', { exact:true })).toHaveCount(0);
  await page.getByRole('button', { name:'Save connection',exact:true }).click();
  await expect(page.getByRole('button', { name:'Edit connection orders-db' })).toBeVisible();
  expect(writes).toHaveLength(2); expect(writes[0]).toBe(writes[1]);
  expect(writes.join('')).not.toContain('browser-credential-canary-017');
  await page.getByRole('link', { name:'Secrets',exact:true }).click();
  await expect(page.getByRole('button', { name:/Rotate secret database-/ })).toHaveCount(1);
  await expect(page.getByText('OpenBao', { exact:true })).toBeVisible();
  await page.getByRole('button', { name:/Rotate secret database-/ }).click();
  await expect(page.getByLabel('Password', { exact:true })).toHaveValue('');
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.getByRole('button', { name:/Rotate secret database-/ })).toBeFocused();
  await page.reload(); await signIn(page,token);
  await page.getByRole('link', { name:'Connections',exact:true }).click();
  await page.getByRole('button', { name:'Edit connection orders-db' }).click();
  await expect(page.getByLabel('Host', { exact:true })).toHaveValue('db.example.com');
  await expect(page.getByLabel('Credentials', { exact:true })).toHaveValue('existing');
  await expect(page.getByLabel('Password', { exact:true })).toHaveCount(0);
  await page.screenshot({ path:test.info().outputPath('connection-reloaded.png'),fullPage:true });
  expect(errors).toEqual([]);
});

async function rest(page: import('@playwright/test').Page, method: string, resource: string, body?: unknown, expected = 200) {
  const response = await page.request.fetch(`${process.env['STUDIO_ORIGIN']}/v1${resource}`, {
    method, headers: { Authorization: `Bearer ${process.env['STUDIO_TOKEN_ADMIN']}`, 'X-Octopus-Namespace': 'database', 'Idempotency-Key': crypto.randomUUID() },
    ...(body === undefined ? {} : { data: body }),
  });
  expect(response.status()).toBe(expected);
  return response.status() === 204 ? undefined : await response.json();
}
async function nodeResult(page: import('@playwright/test').Page, id: string) {
  await page.getByRole('tab', { name: 'Table', exact: true }).click();
  await page.getByRole('button', { name: id, exact: true }).click();
  await expect(page.getByTestId('node-panel')).toContainText('Succeeded');
  await page.getByRole('button', { name: 'Read database result', exact: true }).click();
  return page.getByRole('region', { name: 'Database result', exact: true });
}

test('managed probes and ad hoc work preserve identity and exact results across navigation', async ({ page }) => {
  const token = process.env['STUDIO_TOKEN_ADMIN']!;
  const errors: string[] = []; page.on('pageerror', e => errors.push(e.message));
  await page.setViewportSize({ width: 1440, height: 1000 }); await signIn(page, token);
  await page.getByRole('link', { name: 'Connections', exact: true }).click();
  await page.getByRole('button', { name: 'New connection' }).click();
  await page.getByLabel('Name', { exact: true }).fill('browser-target');
  await page.getByLabel('Host', { exact: true }).fill('127.0.0.1');
  await page.getByLabel('Port', { exact: true }).fill(process.env['STUDIO_TARGET_PORT']!);
  await page.getByLabel('Database', { exact: true }).fill('postgres');
  await page.getByLabel('TLS', { exact: true }).selectOption('development-plaintext');
  await page.getByLabel('Credentials', { exact: true }).selectOption('new');
  await page.getByLabel('Username', { exact: true }).fill('browser_reader');
  await page.getByLabel('Password', { exact: true }).fill('browser-target-password');
  await page.getByLabel('Status', { exact: true }).selectOption('ACTIVE');
  await page.getByRole('button', { name: 'Save connection', exact: true }).click();
  await page.getByRole('button', { name: 'Probe connection browser-target' }).click();
  await page.getByRole('button', { name: 'Start probe', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Latest probe' })).toContainText('SUCCEEDED');
  await page.getByRole('link', { name: 'Open probe run', exact: true }).click();
  await expect(await nodeResult(page, 'probe')).toContainText('1');

  await page.getByRole('link', { name: 'Ad hoc database', exact: true }).click();
  await page.getByLabel('Connection', { exact: true }).selectOption('browser-target');
  await page.getByLabel('Operation', { exact: true }).selectOption('execute');
  await page.getByLabel('SQL statement', { exact: true }).fill('INSERT INTO browser_effects(id) VALUES ($1)');
  await page.getByLabel('Typed parameters (JSON)', { exact: true }).fill('[{"literal":{"type":"integer","value":"9007199254740993"}}]');
  const requests: { key: string | undefined; body: string | null }[] = [];
  page.on('request', request => { if (request.method() === 'POST' && request.url().endsWith('/database-executions')) requests.push({ key: request.headers()['idempotency-key'], body: request.postData() }); });
  let accepted = '';
  await page.route('**/v1/namespaces/*/database-executions', async route => {
    const response = await route.fetch(); expect(response.status()).toBe(201); accepted = (await response.json()).runId;
    // Headers succeed, but the successful body's acknowledgement never arrives.
    await route.fulfill({ status: 201, contentType: 'application/json', body: '' });
  }, { times: 1 });
  await page.getByRole('button', { name: 'Run execution', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Retry same execution' })).toBeVisible();
  await page.getByRole('link', { name: 'Connections', exact: true }).click();
  await page.getByRole('link', { name: 'Ad hoc database', exact: true }).click();
  await expect(page.getByLabel('SQL statement', { exact: true })).toHaveValue('INSERT INTO browser_effects(id) VALUES ($1)');
  await expect(page.getByLabel('SQL statement', { exact: true })).toBeDisabled();
  await page.getByRole('button', { name: 'Retry same execution' }).click();
  await expect(page.getByRole('link', { name: 'Open run', exact: true })).toHaveAttribute('href', `/studio/runs/${accepted}`);
  expect(requests).toHaveLength(2); expect(requests[0]).toEqual(requests[1]);
  await page.getByRole('link', { name: 'Open run', exact: true }).click();
  await expect(await nodeResult(page, 'database_1')).toContainText('1 affected rows');
  const privateCalls: string[] = []; page.on('request', r => { if (/\/workflows\/[0-9a-f-]{36}/.test(r.url())) privateCalls.push(r.url()); });
  await page.reload(); await signIn(page, token);
  await page.getByRole('link', { name: 'Runs', exact: true }).click();
  await page.getByLabel('Origin', { exact: true }).selectOption('AD_HOC_STEP');
  await page.getByRole('link', { name: accepted, exact: true }).click();
  await expect(page.getByRole('heading', { level: 1 })).toContainText('Ad hoc database step');
  await nodeResult(page, 'database_1'); expect(privateCalls).toEqual([]);

  await page.getByRole('link', { name: 'Ad hoc database', exact: true }).click();
  await page.getByLabel('Execution mode').selectOption('workflow');
  await page.getByLabel('Connection', { exact: true }).selectOption('browser-target');
  await page.getByLabel('SQL statement', { exact: true }).fill('SELECT $1::bigint AS value');
  await page.getByLabel('Typed parameters (JSON)', { exact: true }).fill('[{"type":"integer","expression":"input.number"}]');
  await page.getByLabel('Run input (JSON)').fill('{"number":"9223372036854775807"}');
  await page.getByRole('button', { name: 'Add node', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Database', exact: true }).click();
  await page.getByLabel('Connection', { exact: true }).selectOption('browser-target');
  await page.getByLabel('SQL statement', { exact: true }).fill(`SELECT $1::bigint AS value, 12345678901234567890.123::numeric AS amount, '{"id":9007199254740993}'::json AS detail`);
  await page.getByLabel('Typed parameters (JSON)', { exact: true }).fill('[{"type":"integer","expression":"steps.database_1.output.rows[0][0].value"}]');
  await page.getByText('Result and timeout limits', { exact: true }).click();
  await page.getByLabel('Query timeout (ms)').fill('5000');
  for (const width of [320, 768, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width + 1);
    expect((await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze()).violations).toEqual([]);
    await page.screenshot({ path: test.info().outputPath(`ad-hoc-${width}.png`), fullPage: true });
  }
  await page.getByRole('button', { name: 'Run execution', exact: true }).click();
  await page.getByRole('link', { name: 'Open run', exact: true }).click();
  const rows = await nodeResult(page, 'database_2');
  await expect(rows).toContainText('9223372036854775807'); await expect(rows).toContainText('12345678901234567890.123'); await expect(rows).toContainText('{"id":9007199254740993}');
  expect((await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze()).violations).toEqual([]);
  await rows.screenshot({ path: test.info().outputPath('database-results-table.png') });
  await page.screenshot({ path: test.info().outputPath('database-results.png'), fullPage: true });
  await page.getByRole('button', { name: 'Close database result' }).click();
  await expect(page.getByText('12345678901234567890.123', { exact: true })).toHaveCount(0);
  expect(errors).toEqual([]);
});

test('published visual database node and role-scoped result access use the same managed contract', async ({ page }) => {
  await signIn(page, process.env['STUDIO_TOKEN_ADMIN']!);
  await page.getByRole('link', { name: 'Workflows', exact: true }).click();
  await page.getByRole('button', { name: 'New workflow', exact: true }).click();
  await page.getByRole('dialog').getByLabel('Name', { exact: true }).fill('visual-database');
  await page.getByRole('button', { name: 'Create workflow', exact: true }).click();
  await page.waitForURL(/\/workflows\/[0-9a-f-]{36}$/); const workflow = page.url().split('/').pop()!;
  await page.getByRole('button', { name: 'Add node', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Database', exact: true }).click();
  await page.getByLabel('Connection', { exact: true }).selectOption('browser-target');
  await page.getByLabel('SQL statement', { exact: true }).fill('SELECT $1::bigint AS value');
  await page.getByLabel('Typed parameters (JSON)', { exact: true }).fill('[{"literal":{"type":"integer","value":"9223372036854775807"}}]');
  await expect(page.getByRole('button', { name: 'Publish…' })).toBeEnabled();
  await page.getByRole('button', { name: 'Publish…' }).click();
  await page.getByRole('dialog', { name: 'Publish workflow' }).getByRole('button', { name: 'Publish', exact: true }).click();
  await expect(page.getByText('Published version 1.').first()).toBeVisible();
  const draft = await rest(page, 'GET', `/workflows/${workflow}`); expect(draft.ir.nodes[0]).toMatchObject({ type: 'database', operation: 'query', connectionRef: { name: 'browser-target' } });
  const run = await rest(page, 'POST', '/runs', { workflowId: workflow, input: {} }, 202);
  const { goTo } = await import('../support/studio'); await goTo(page, `/runs/${run.runId}`);
  await expect(await nodeResult(page, 'database_1')).toContainText('9223372036854775807');
  await page.getByRole('button', { name: 'Sign out', exact: true }).click();
  await signIn(page, process.env['STUDIO_TOKEN_VIEWER']!); await goTo(page, `/runs/${run.runId}`);
  await page.getByRole('tab', { name: 'Table', exact: true }).click(); await page.getByRole('button', { name: 'database_1', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Read database result' })).toHaveCount(0);
  await page.getByRole('link', { name: 'Ad hoc database', exact: true }).click();
  await expect(page.getByText(/Requires database:adhoc/)).toBeVisible();
  await page.getByRole('button', { name: 'Sign out', exact: true }).click();
  await signIn(page, process.env['STUDIO_TOKEN_DEVELOPER']!);
  const ns = process.env['STUDIO_NAMESPACE_ID']!;
  const connection = await rest(page, 'GET', `/namespaces/${ns}/connections/browser-target`);
  await rest(page, 'PUT', `/namespaces/${ns}/secrets/${connection.config.secretName}/access/browser-developer`, undefined, 204);
  await page.getByRole('link', { name: 'Ad hoc database', exact: true }).click();
  await page.getByLabel('Connection', { exact: true }).selectOption('browser-target');
  await expect(page.getByRole('option', { name: /Execute ·/ })).toBeDisabled();
  await page.getByRole('button', { name: 'Run execution', exact: true }).click();
  await page.getByRole('link', { name: 'Open run', exact: true }).click();
  await page.getByRole('tab', { name: 'Table', exact: true }).click(); await page.getByRole('button', { name: 'database_1', exact: true }).click();
  await expect(page.getByTestId('node-panel')).toContainText('Succeeded');
  await expect(page.getByRole('button', { name: 'Read database result' })).toHaveCount(0);
});


test('paused connections refuse probes and unsupported SQL has a durable safe failure', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', e => errors.push(e.message));
  await signIn(page, process.env['STUDIO_TOKEN_ADMIN']!);
  await page.getByRole('link', { name: 'Connections', exact: true }).click();
  await page.getByRole('button', { name: 'Probe connection orders-db' }).click();
  await expect(page.getByRole('button', { name: 'Start probe', exact: true })).toBeDisabled();
  await expect(page.getByText('Activate the connection before starting a probe.')).toBeVisible();
  await page.keyboard.press('Escape');
  await page.getByRole('link', { name: 'Ad hoc database', exact: true }).click();
  await page.getByLabel('Connection', { exact: true }).selectOption('browser-target');
  await page.getByLabel('SQL statement', { exact: true }).fill('DROP TABLE browser_effects');
  await page.getByRole('button', { name: 'Run execution', exact: true }).click();
  await page.getByRole('link', { name: 'Open run', exact: true }).click();
  await page.getByRole('tab', { name: 'Table', exact: true }).click();
  await page.getByRole('button', { name: 'database_1', exact: true }).click();
  await expect(page.getByTestId('node-panel')).toContainText('DATABASE_STATEMENT_DENIED');
  await expect(page.getByRole('button', { name: 'Read database result' })).toHaveCount(0);
  await page.screenshot({ path: test.info().outputPath('database-safe-failure.png'), fullPage: true });
  expect(errors).toEqual([]);
});
