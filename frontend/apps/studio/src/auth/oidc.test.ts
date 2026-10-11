import { beforeEach, describe, expect, it, vi } from 'vitest';
import { challengeOf, finishLogin, newVerifier, startLogin } from './oidc';
import type { OidcSettings } from './oidc';

const ISSUER = 'https://login.example.test/realms/acme';
const settings: OidcSettings = { issuer: ISSUER, clientId: 'studio', scope: 'openid profile' };

const discovery = {
  issuer: ISSUER,
  authorization_endpoint: `${ISSUER}/protocol/openid-connect/auth`,
  token_endpoint: `${ISSUER}/protocol/openid-connect/token`,
};

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

class MemoryStorage {
  readonly data = new Map<string, string>();
  getItem(key: string) {
    return this.data.get(key) ?? null;
  }
  setItem(key: string, value: string) {
    this.data.set(key, value);
  }
  removeItem(key: string) {
    this.data.delete(key);
  }
}

function env(routes: Record<string, () => Response> = {}) {
  const storage = new MemoryStorage();
  const calls: { url: string; init: RequestInit | undefined }[] = [];
  const fetcher = vi.fn((url: string, init?: RequestInit) => {
    calls.push({ url, init });
    const handler = routes[url] ?? (url.endsWith('/.well-known/openid-configuration') ? () => json(discovery) : undefined);
    return Promise.resolve(handler ? handler() : json({ error: 'nope' }, 404));
  }) as unknown as typeof fetch;
  const assign = vi.fn();
  const replace = vi.fn();
  return { storage, fetcher, calls, assign, replace, origin: 'https://studio.example.test' };
}

beforeEach(() => vi.restoreAllMocks());

describe('PKCE (RFC 7636)', () => {
  it('derives the challenge the RFC says it derives', async () => {
    expect(await challengeOf('dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk')).toBe('E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM');
  });

  it('makes verifiers that are long enough, unpredictable and made of the allowed characters', () => {
    const a = newVerifier();
    const b = newVerifier();
    expect(a).toMatch(/^[A-Za-z0-9_-]{43,128}$/);
    expect(a).not.toBe(b);
  });
});

describe('startLogin', () => {
  it('sends the browser to the issuer with a challenge and a state, and keeps the verifier for the way back', async () => {
    const e = env();
    await startLogin(settings, { fetch: e.fetcher, storage: e.storage, origin: e.origin, assign: e.assign });
    expect(e.assign).toHaveBeenCalledOnce();
    const url = new URL(e.assign.mock.calls[0]![0] as string);
    expect(`${url.origin}${url.pathname}`).toBe(discovery.authorization_endpoint);
    expect(Object.fromEntries(url.searchParams)).toMatchObject({
      response_type: 'code',
      client_id: 'studio',
      redirect_uri: 'https://studio.example.test/studio/auth/callback',
      scope: 'openid profile',
      code_challenge_method: 'S256',
    });
    const pending = JSON.parse(e.storage.getItem('studio.oidc.pending') ?? '{}') as { state: string; verifier: string };
    expect(url.searchParams.get('state')).toBe(pending.state);
    expect(url.searchParams.get('code_challenge')).toBe(await challengeOf(pending.verifier));
    // The verifier is what the issuer is shown a hash of: it is not in the URL.
    expect(url.toString()).not.toContain(pending.verifier);
  });

  it('does not trust a discovery document that is about another issuer or points the browser elsewhere', async () => {
    for (const bad of [
      { ...discovery, issuer: 'https://evil.example.test' },
      { ...discovery, token_endpoint: 'https://evil.example.test/token' },
      { ...discovery, authorization_endpoint: 'http://login.example.test/auth' },
      { ...discovery, authorization_endpoint: 'javascript:alert(1)' },
      { issuer: ISSUER },
    ]) {
      const e = env({ [`${ISSUER}/.well-known/openid-configuration`]: () => json(bad) });
      await expect(startLogin(settings, { fetch: e.fetcher, storage: e.storage, origin: e.origin, assign: e.assign })).rejects.toThrow(/identity provider/);
      expect(e.assign).not.toHaveBeenCalled();
      expect(e.storage.data.size).toBe(0);
    }
  });
});

describe('finishLogin', () => {
  async function started() {
    const e = env({ [discovery.token_endpoint]: () => json({ access_token: 'aaa.bbb.ccc', token_type: 'Bearer', expires_in: 300 }) });
    await startLogin(settings, { fetch: e.fetcher, storage: e.storage, origin: e.origin, assign: e.assign });
    const pending = JSON.parse(e.storage.getItem('studio.oidc.pending') ?? '{}') as { state: string; verifier: string };
    return { e, pending };
  }

  it('exchanges the code with the verifier and gives back the access token, cleaning the address bar', async () => {
    const { e, pending } = await started();
    const token = await finishLogin(`?code=the-code&state=${pending.state}`, settings, { fetch: e.fetcher, storage: e.storage, origin: e.origin, replace: e.replace });
    expect(token).toBe('aaa.bbb.ccc');
    const exchange = e.calls.find((c) => c.url === discovery.token_endpoint)!;
    expect(exchange.init?.method).toBe('POST');
    expect(exchange.init).toMatchObject({ credentials: 'omit', redirect: 'error', cache: 'no-store' });
    const body = new URLSearchParams(exchange.init?.body as string);
    expect(Object.fromEntries(body)).toMatchObject({ grant_type: 'authorization_code', code: 'the-code', client_id: 'studio', code_verifier: pending.verifier, redirect_uri: 'https://studio.example.test/studio/auth/callback' });
    expect(e.replace).toHaveBeenCalledWith('/studio/');
    expect(e.storage.getItem('studio.oidc.pending')).toBeNull();
  });

  it('refuses a state it did not issue, and a code used twice', async () => {
    const { e, pending } = await started();
    await expect(finishLogin('?code=x&state=forged', settings, { fetch: e.fetcher, storage: e.storage, origin: e.origin, replace: e.replace })).rejects.toThrow(/did not start/);
    // The attempt is over whatever happened: the genuine callback now has nothing to match.
    await expect(finishLogin(`?code=x&state=${pending.state}`, settings, { fetch: e.fetcher, storage: e.storage, origin: e.origin, replace: e.replace })).rejects.toThrow(/did not start/);
    expect(e.calls.some((c) => c.url === discovery.token_endpoint)).toBe(false);
  });

  it('says what the issuer said when it refused, and what is wrong when its answer is odd', async () => {
    const { e, pending } = await started();
    await expect(finishLogin(`?error=access_denied&error_description=No+access&state=${pending.state}`, settings, { fetch: e.fetcher, storage: e.storage, origin: e.origin, replace: e.replace })).rejects.toThrow(/access_denied/);

    for (const answer of [{ access_token: 'a.b.c', token_type: 'mac' }, { token_type: 'Bearer' }, { access_token: 'not a token', token_type: 'Bearer' }]) {
      const started2 = env({ [discovery.token_endpoint]: () => json(answer) });
      await startLogin(settings, { fetch: started2.fetcher, storage: started2.storage, origin: started2.origin, assign: started2.assign });
      const p = JSON.parse(started2.storage.getItem('studio.oidc.pending') ?? '{}') as { state: string };
      await expect(finishLogin(`?code=c&state=${p.state}`, settings, { fetch: started2.fetcher, storage: started2.storage, origin: started2.origin, replace: started2.replace })).rejects.toThrow(/token/);
    }
    const failing = env({ [discovery.token_endpoint]: () => json({ error: 'invalid_grant', error_description: 'secret detail' }, 400) });
    await startLogin(settings, { fetch: failing.fetcher, storage: failing.storage, origin: failing.origin, assign: failing.assign });
    const p = JSON.parse(failing.storage.getItem('studio.oidc.pending') ?? '{}') as { state: string };
    await expect(finishLogin(`?code=c&state=${p.state}`, settings, { fetch: failing.fetcher, storage: failing.storage, origin: failing.origin, replace: failing.replace })).rejects.toThrow(/invalid_grant/);
  });
});
