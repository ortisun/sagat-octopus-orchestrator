import { QueryClient } from '@tanstack/react-query';
import { createContext, useContext, useSyncExternalStore } from 'react';
import { ApiClient } from '../api/client';
import { endpoints } from '../api/endpoints';
import type { Endpoints } from '../api/endpoints';
import { SessionStore } from '../auth/session';
import type { SessionSnapshot } from '../auth/session';
import { NamespaceStore } from './namespace';

/** The deployment's public settings (`/studio/config.json`). */
export interface StudioSettings {
  oidc: { issuer: string; clientId: string; scope: string } | null;
}

export interface App {
  session: SessionStore;
  namespace: NamespaceStore;
  api: ApiClient;
  ep: Endpoints;
  queries: QueryClient;
  settings: StudioSettings;
}

export interface AppOptions {
  fetch?: typeof globalThis.fetch;
  settings?: StudioSettings;
  baseUrl?: string;
}

export function createApp(options: AppOptions = {}): App {
  const session = new SessionStore();
  const namespace = new NamespaceStore();
  const queries = new QueryClient({
    defaultOptions: {
      queries: {
        // The platform is the source of truth and changes under us; a stale answer is refreshed
        // when the window is focused again, and a failed one is not retried blindly.
        staleTime: 5_000,
        retry: false,
        refetchOnWindowFocus: true,
      },
    },
  });
  const api = new ApiClient({
    baseUrl: options.baseUrl ?? '/v1',
    token: session.token,
    namespace: namespace.current,
    onUnauthenticated: () => session.signOut('rejected'),
    ...(options.fetch ? { fetch: options.fetch } : {}),
  });
  // Nothing of a session outlives it: queries cached for one user are not shown to the next.
  session.subscribe(() => {
    if (session.snapshot().info === undefined) queries.clear();
  });
  return { session, namespace, api, ep: endpoints(api), queries, settings: options.settings ?? { oidc: null } };
}

export const AppContext = createContext<App | null>(null);

export function useApp(): App {
  const app = useContext(AppContext);
  if (app === null) throw new Error('the Studio is not mounted');
  return app;
}

export function useSession(): SessionSnapshot {
  const { session } = useApp();
  return useSyncExternalStore(session.subscribe, session.snapshot);
}

export function useCan(permission: string): boolean {
  const { session } = useApp();
  useSession();
  return session.can(permission);
}

export function useNamespace(): string | undefined {
  const { namespace } = useApp();
  return useSyncExternalStore(namespace.subscribe, namespace.current);
}
