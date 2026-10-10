import { QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider, createBrowserRouter } from 'react-router';
import { AnnouncerProvider } from '../ui/Announcer';
import { useMemo } from 'react';
import { CALLBACK_PATH } from '../auth/oidc';
import { AppContext, useApp, useSession } from './context';
import { OidcCallback } from './OidcCallback';
import type { App } from './context';
import { routes } from './routes';
import { SignIn } from './SignIn';

/** Everything the Studio is: providers, the sign-in gate and the routes. */
export function Root({ app }: { app: App }) {
  return (
    <AppContext.Provider value={app}>
      <QueryClientProvider client={app.queries}>
        <AnnouncerProvider>
          <Gate />
        </AnnouncerProvider>
      </QueryClientProvider>
    </AppContext.Provider>
  );
}

function Gate() {
  const { info } = useSession();
  const { settings } = useApp();
  if (info === undefined) {
    return settings.oidc !== null && window.location.pathname === CALLBACK_PATH ? <OidcCallback /> : <SignIn />;
  }
  return <Authenticated />;
}

/** The router starts when the user is in, so it reads the address the sign-in left, not the callback's. */
function Authenticated() {
  const router = useMemo(() => createBrowserRouter(routes, { basename: '/studio' }), []);
  return <RouterProvider router={router} />;
}
