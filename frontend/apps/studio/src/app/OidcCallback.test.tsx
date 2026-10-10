import { screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../test/render';
import { OidcCallback } from './OidcCallback';

const ISSUER = 'https://login.example.test/realms/acme';

const sessionStorageStub = (value: string | null) => {
  const data = new Map<string, string>(value === null ? [] : [['studio.oidc.pending', value]]);
  vi.stubGlobal('sessionStorage', { getItem: (k: string) => data.get(k) ?? null, setItem: (k: string, v: string) => void data.set(k, v), removeItem: (k: string) => void data.delete(k) });
};

function platform() {
  return fakePlatform({
    'GET /auth/context': () => AUTH_CONTEXT,
  });
}

describe('OidcCallback', () => {
  it('signs the user in with the token the code is exchanged for', async () => {
    sessionStorageStub(JSON.stringify({ state: 's1', verifier: 'v'.repeat(43) }));
    const p = platform();
    const idp = vi.fn((url: string | URL | Request, init?: RequestInit) => {
      const target = String(url);
      if (target.endsWith('/.well-known/openid-configuration')) return Promise.resolve(Response.json({ issuer: ISSUER, authorization_endpoint: `${ISSUER}/auth`, token_endpoint: `${ISSUER}/token` }));
      if (target === `${ISSUER}/token`) return Promise.resolve(Response.json({ access_token: TOKEN, token_type: 'Bearer' }));
      return p.fetcher(String(url), init);
    });
    vi.stubGlobal('fetch', idp);
    const { app } = renderApp(<OidcCallback search="?code=c&state=s1" />, { appOptions: { fetch: p.fetcher, settings: { oidc: { issuer: ISSUER, clientId: 'studio', scope: 'openid' } } } });
    await waitFor(() => expect(app.session.token()).toBe(TOKEN));
    expect(app.session.snapshot().info?.subject).toBe('alice');
    vi.unstubAllGlobals();
  });

  it('says why when the sign-in cannot be finished, and offers a way back', async () => {
    sessionStorageStub(null);
    const p = platform();
    renderApp(<OidcCallback search="?code=c&state=forged" />, { appOptions: { fetch: p.fetcher, settings: { oidc: { issuer: ISSUER, clientId: 'studio', scope: 'openid' } } } });
    expect(await screen.findByRole('alert')).toHaveTextContent(/did not start in this page/);
    expect(screen.getByRole('button', { name: 'Back to sign-in' })).toBeInTheDocument();
    vi.unstubAllGlobals();
  });
});
