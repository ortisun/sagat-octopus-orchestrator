import { useState } from 'react';
import type { FormEvent } from 'react';
import { endpoints } from '../api/endpoints';
import { isApiError } from '../api/errors';
import { SessionStore } from '../auth/session';
import { Banner } from '../ui/Banner';
import { Button } from '../ui/Button';
import { Field } from '../ui/Field';
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
      <h1>Sagat Studio</h1>
      {ended !== undefined && REASONS[ended] !== '' ? <Banner tone="warn">{REASONS[ended]}</Banner> : null}
      <form onSubmit={(event) => void submit(event)}>
        <Field label="Access token" hint="A token issued for you by your identity provider. It stays in this page's memory and is gone when you close or reload it." {...(error ? { error } : {})}>
          <textarea
            value={token}
            onChange={(event) => setToken(event.target.value)}
            rows={4}
            spellCheck={false}
            autoComplete="off"
            autoCapitalize="off"
            required
            aria-required="true"
          />
        </Field>
        <Button type="submit" variant="primary" disabled={busy || !SessionStore.looksLikeToken(token.trim())}>
          {busy ? 'Signing in…' : 'Sign in'}
        </Button>
      </form>
      {settings.oidc ? <p>Single sign-on is configured for this deployment.</p> : null}
    </main>
  );
}
