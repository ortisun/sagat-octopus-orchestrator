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
