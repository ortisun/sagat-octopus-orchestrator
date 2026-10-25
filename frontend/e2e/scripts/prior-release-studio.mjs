// Execute the archived release bundle served by the current real control plane.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { chromium } from '@playwright/test';

let browser;
let stage = 'configuration';
try {
  const { origin, token, runId, forbidden } = JSON.parse(readFileSync(0, 'utf8'));
  assert.equal(new URL(origin).hostname, '127.0.0.1');
  stage = 'launch';
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  page.setDefaultTimeout(15_000);
  const problems = [];
  const responses = [];
  const readPaths = new Set();
  page.on('pageerror', () => problems.push('pageerror'));
  page.on('console', (message) => {
    if (message.type() === 'error') problems.push('console.error');
  });
  page.on('response', (response) => {
    const url = new URL(response.url());
    if (url.origin !== origin || !url.pathname.startsWith('/v1/') || url.pathname.endsWith('/stream')) return;
    responses.push(response.text().then((body) => {
      if (forbidden.some((canary) => body.includes(canary))) problems.push('payload-in-response');
      if (response.status() === 200) readPaths.add(url.pathname);
    }).catch(() => undefined));
  });
  stage = 'sign-in';
  await page.goto(`${origin}/studio/`);
  await page.getByLabel('Access token').fill(token);
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await page.getByRole('navigation', { name: 'Main' }).waitFor({ state: 'visible' });
  stage = 'run';
  await page.evaluate((id) => {
    history.pushState({}, '', `/studio/runs/${id}`);
    dispatchEvent(new PopStateEvent('popstate'));
  }, runId);
  await page.locator('.rn-header h1').filter({ hasText: 'release-compat' }).waitFor({ state: 'visible' });
  stage = 'table';
  await page.getByRole('tab', { name: 'Table', exact: true }).click();
  await page.getByRole('row').filter({ hasText: 'work' }).last().waitFor({ state: 'visible' });
  stage = 'boundaries';
  const text = await page.locator('body').innerText();
  for (const canary of forbidden) assert(!text.includes(canary), 'old Studio displayed a payload canary');
  await Promise.all(responses);
  assert(readPaths.has(`/v1/runs/${runId}`), 'old bundle must read the real run');
  assert(readPaths.has(`/v1/runs/${runId}/graph`), 'old bundle must read the real graph');
  assert.equal(problems.length, 0, 'old Studio emitted a browser error');
  console.log('prior-release Studio: sign-in, run and table passed; payload canaries absent');
} catch {
  // A failed locator may contain typed text; report only a closed stage, never its raw error.
  console.error(`prior-release Studio failed at ${stage}`);
  process.exitCode = 1;
} finally {
  await browser?.close();
}
