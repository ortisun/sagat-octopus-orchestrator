import { useState } from 'react';
import type { FormEvent } from 'react';
import { endpoints } from '../api/endpoints';
import { isApiError } from '../api/errors';
import { startLogin } from '../auth/oidc';
import type { OidcSettings } from '../auth/oidc';
import { SessionStore } from '../auth/session';
import { Banner, Button, Heading, Stack, TextArea } from '../ui';
import { useApp, useSession } from './context';

const REASONS = {
  expired: 'Your session ended because the token expired.',
  rejected: 'The platform no longer accepts your token.',
  'signed-out': '',
} as const;

/** The way in: a bearer token pasted by the user, kept in memory only (D144). */
export function SignIn() {
  const { session, api, settings } = useApp();
  const { ended } = useSession();
  const [token, setToken] = useState('');
  const [error, setError] = useState<string | undefined>();
  const [busy, setBusy] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setError(undefined);
    setBusy(true);
    try {
      await session.signIn(token, (candidate) => endpoints(api.withToken(candidate)).authContext());
      setToken('');
    } catch (cause) {
      setError(isApiError(cause) && cause.status === 403 ? 'That token is valid but has no access here.' : cause instanceof Error ? cause.message : 'Sign-in failed.');
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="signin" id="main">
      <Stack gap={4}>
        <Heading level={1}>Sagat Studio</Heading>
        {ended !== undefined && REASONS[ended] !== '' ? <Banner tone="warn">{REASONS[ended]}</Banner> : null}
        <form onSubmit={(event) => void submit(event)}>
          <Stack gap={3}>
            <TextArea
              label="Access token"
              hint="A token issued for you by your identity provider. It stays in this page's memory and is gone when you close or reload it."
              error={error}
              value={token}
              onChange={setToken}
              rows={4}
              spellCheck={false}
              mono
              required
            />
            <div>
              <Button type="submit" variant="primary" disabled={busy || !SessionStore.looksLikeToken(token.trim())}>
                {busy ? 'Signing in…' : 'Sign in'}
              </Button>
            </div>
          </Stack>
        </form>
        {settings.oidc ? (
          <Stack as="section" gap={2} aria-labelledby="sso">
            <Heading level={2} id="sso">Or sign in with your organisation</Heading>
            <div>
              <Button
                onClick={() => {
                  setError(undefined);
                  void startLogin(settings.oidc as OidcSettings).catch((cause: unknown) => setError(cause instanceof Error ? cause.message : 'Sign-in failed.'));
                }}
              >
                Sign in with your organisation
              </Button>
            </div>
          </Stack>
        ) : null}
      </Stack>
    </main>
  );
}
