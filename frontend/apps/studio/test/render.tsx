import { QueryClientProvider } from '@tanstack/react-query';
import { render } from '@testing-library/react';
import type { ReactElement } from 'react';
import { MemoryRouter } from 'react-router';
import { AppContext, createApp } from '../src/app/context';
import type { App, AppOptions } from '../src/app/context';
import { AnnouncerProvider } from '../src/ui/Announcer';

/** A JWT-shaped string (the platform verifies tokens; the Studio only checks the shape). */
export const TOKEN = `${btoa('{"alg":"RS256"}')}.${btoa(JSON.stringify({ exp: 4_102_444_800 }))}.signature`
  .replace(/=/g, '')
  .replace(/\+/g, '-')
  .replace(/\//g, '_');

export type Routes = Record<string, (init: RequestInit, url: string) => unknown>;

/**
 * A Studio over a scripted platform: `routes` maps "METHOD /path" to a body (or a Response). Every
 * request is recorded in `calls`.
 */
export function fakePlatform(routes: Routes) {
  const calls: { method: string; url: string; init: RequestInit }[] = [];
  const fetcher = ((url: string, init: RequestInit = {}) => {
    const method = init.method ?? 'GET';
    calls.push({ method, url, init });
    const key = `${method} ${url.replace(/^\/v1/, '').split('?')[0]}`;
    const handler = routes[key] ?? routes[`${method} *`];
    if (!handler) return Promise.resolve(new Response(JSON.stringify({ code: 'NOT_FOUND', message: `no route ${key}` }), { status: 404, headers: { 'content-type': 'application/json' } }));
    const out = handler(init, url);
    if (out instanceof Response) return Promise.resolve(out);
    return Promise.resolve(new Response(JSON.stringify(out), { status: 200, headers: { 'content-type': 'application/json' } }));
  }) as typeof fetch;
  return { fetcher, calls };
}

export function renderApp(ui: ReactElement, options: { app?: App; appOptions?: AppOptions; path?: string } = {}) {
  const app = options.app ?? createApp(options.appOptions);
  const result = render(
    <AppContext.Provider value={app}>
      <QueryClientProvider client={app.queries}>
        <AnnouncerProvider>
          <MemoryRouter initialEntries={[options.path ?? '/']}>{ui}</MemoryRouter>
        </AnnouncerProvider>
      </QueryClientProvider>
    </AppContext.Provider>,
  );
  return { app, ...result };
}

export const AUTH_CONTEXT = { tenant: 'acme', subject: 'alice', role: 'admin', permissions: ['definition:read', 'definition:write', 'definition:publish', 'run:read', 'run:operate', 'saga:override'], traceId: 'a'.repeat(32) };
