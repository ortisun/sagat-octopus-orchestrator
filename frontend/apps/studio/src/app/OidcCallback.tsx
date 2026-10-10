import { useEffect, useRef, useState } from 'react';
import { endpoints } from '../api/endpoints';
import { finishLogin } from '../auth/oidc';
import { Banner } from '../ui/Banner';
import { Button } from '../ui/Button';
import { useApp } from './context';

/** The page the identity provider sends the browser back to: it finishes the sign-in or says why not. */
export function OidcCallback({ search = window.location.search }: { search?: string }) {
  const { session, api, settings } = useApp();
  const [error, setError] = useState<string | undefined>();
  const started = useRef(false);

  useEffect(() => {
    // Once: a code is good for one exchange, and a second run would find the attempt gone.
    if (started.current || settings.oidc === null) return;
    started.current = true;
    finishLogin(search, settings.oidc)
      .then((token) => session.signIn(token, (candidate) => endpoints(api.withToken(candidate)).authContext()))
      .catch((cause: unknown) => setError(cause instanceof Error ? cause.message : 'Sign-in failed.'));
  }, [api, search, session, settings.oidc]);

  return (
    <main className="signin" id="main">
      <h1>Sagat Studio</h1>
      {error === undefined ? (
        <p role="status">Signing you in…</p>
      ) : (
        <>
          <Banner tone="error">{error}</Banner>
          <Button variant="primary" onClick={() => window.location.assign('/studio/')}>
            Back to sign-in
          </Button>
        </>
      )}
    </main>
  );
}
