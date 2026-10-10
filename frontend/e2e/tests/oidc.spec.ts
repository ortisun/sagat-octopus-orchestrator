import { createHash } from 'node:crypto';
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { env } from '../support/env';
import { failOnBrowserErrors } from '../support/studio';

test.use({ viewport: { width: 1440, height: 900 } });

const issuer = `${env.origin}/idp`;
const callback = `${env.origin}/studio/auth/callback`;

interface Behaviour {
  /** What the provider sends back to the page after the user "signs in". */
  answer?: (state: string) => Record<string, string>;
}

/**
 * A stand-in identity provider, played inside the browser by routing: its discovery document, its authorization
 * endpoint (which answers with a redirect to the page, as a provider does once the user has signed in) and its token
 * endpoint, which checks the PKCE verifier against the challenge it was given and gives a token of the platform's own
 * test issuer. The platform, which is real, validates that token as it does any other. The settings of the page are
 * the ones a deployment would give it: the route of `config.json` is the only thing that says there is a provider.
 */
async function standInProvider(page: Page, behaviour: Behaviour = {}): Promise<{ seen: string[]; exchanges: number }> {
  const log = { seen: [] as string[], exchanges: 0 };
  let challenge = '';
  let spent = false;
  await page.route(`${env.origin}/studio/config.json`, (route) => route.fulfill({ json: { oidc: { issuer, clientId: 'studio-journeys', scope: 'openid profile' } } }));
  await page.route(`${issuer}/**`, async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    log.seen.push(`${request.method()} ${url.pathname}`);
    if (url.pathname === '/idp/.well-known/openid-configuration') {
      await route.fulfill({ json: { issuer, authorization_endpoint: `${issuer}/authorize`, token_endpoint: `${issuer}/token` } });
    } else if (url.pathname === '/idp/authorize') {
      const query = url.searchParams;
      expect(query.get('response_type')).toBe('code');
      expect(query.get('client_id')).toBe('studio-journeys');
      expect(query.get('code_challenge_method')).toBe('S256');
      expect(query.get('redirect_uri')).toBe(callback);
      expect(query.get('state')?.length ?? 0).toBeGreaterThanOrEqual(16);
      challenge = query.get('code_challenge') ?? '';
      expect(challenge.length).toBeGreaterThanOrEqual(43);
      expect(url.search).not.toContain(env.tokens.operator);
      const back = new URLSearchParams((behaviour.answer ?? ((state) => ({ code: 'code-1', state })))(query.get('state') ?? ''));
      await route.fulfill({ status: 302, headers: { location: `${callback}?${back.toString()}` }, body: '' });
    } else if (url.pathname === '/idp/token') {
      log.exchanges += 1;
      const form = new URLSearchParams(request.postData() ?? '');
      const verifier = form.get('code_verifier') ?? '';
      const matches = createHash('sha256').update(verifier).digest('base64url') === challenge;
      if (form.get('grant_type') !== 'authorization_code' || form.get('code') !== 'code-1' || form.get('redirect_uri') !== callback || !matches || spent) {
        await route.fulfill({ status: 400, json: { error: 'invalid_grant' } });
        return;
      }
      spent = true;
      await route.fulfill({ json: { access_token: env.tokens.operator, token_type: 'Bearer', expires_in: 3000 } });
    } else {
      await route.fulfill({ status: 404, body: '' });
    }
  });
  return log;
}

test('OIDC signing in with an identity provider: the authorization code flow with PKCE, a public client, and the token never kept', async ({ page }) => {
  const problems = failOnBrowserErrors(page);
  const provider = await standInProvider(page);
  await page.goto('./');
  await page.getByRole('button', { name: 'Sign in with your organisation' }).click();

  // Back from the provider: signed in as the person the token names, with the address bar clean.
  await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible({ timeout: 20_000 });
  await expect(page.getByText('olivia')).toBeVisible();
  expect(page.url()).not.toMatch(/code=|state=/);
  expect(provider.seen).toEqual(['GET /idp/.well-known/openid-configuration', 'GET /idp/authorize', 'GET /idp/.well-known/openid-configuration', 'POST /idp/token']);
  expect(provider.exchanges).toBe(1);

  // What was kept for the redirect is gone, and the token is nowhere the page keeps things.
  const kept = await page.evaluate(() => ({ session: JSON.stringify({ ...sessionStorage }), local: JSON.stringify({ ...localStorage }) }));
  expect(kept.session).not.toContain('verifier');
  expect(kept.session + kept.local).not.toContain(env.tokens.operator);
  expect(problems.filter((p) => !p.includes('Failed to load resource'))).toEqual([]);
});

test('OIDC a callback that did not start in this page, and a provider that refuses, are told as what they are and sign nobody in', async ({ page }) => {
  // A callback with the wrong state: somebody else's redirect, or an attack. Nothing is exchanged.
  const wrongState = await standInProvider(page, { answer: () => ({ code: 'code-1', state: 'not-the-state-of-this-page' }) });
  await page.goto('./');
  await page.getByRole('button', { name: 'Sign in with your organisation' }).click();
  await expect(page.getByText('That sign-in did not start in this page. Start again.')).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole('navigation', { name: 'Main' })).toBeHidden();
  expect(wrongState.exchanges).toBe(0);

  // The provider refuses (the user said no): its reason is shown, short, as text.
  await page.unrouteAll();
  const refused = await standInProvider(page, { answer: (state) => ({ error: 'access_denied', error_description: 'The user said no <b>sorry</b>', state }) });
  await page.goto('./');
  await page.getByRole('button', { name: 'Sign in with your organisation' }).click();
  await expect(page.getByText(/The identity provider refused the sign-in: access_denied \(The user said no <b>sorry<\/b>\)\./)).toBeVisible({ timeout: 20_000 });
  expect(refused.exchanges).toBe(0);
  expect(await page.locator('main b').count()).toBe(0);
});
