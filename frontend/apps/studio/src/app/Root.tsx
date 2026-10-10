import { QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider, createBrowserRouter } from 'react-router';
import { AnnouncerProvider } from '../ui/Announcer';
import { AppContext, useSession } from './context';
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

const router = createBrowserRouter(routes, { basename: '/studio' });

function Gate() {
  const { info } = useSession();
  return info === undefined ? <SignIn /> : <RouterProvider router={router} />;
}
