import { QueryClientProvider } from '@tanstack/react-query';
import { act, render } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { StrictMode } from 'react';
import { vi } from 'vitest';
import { Route, RouterProvider, Routes, createMemoryRouter } from 'react-router';
import './reactflow';
import { installStorage } from './storage';
import { AppContext, createApp } from '../src/app/context';
import type { App } from '../src/app/context';
import { WorkflowEditorPage } from '../src/features/workflows/WorkflowEditorPage';
import { DEFAULT_TIMINGS, TimingsContext } from '../src/features/workflows/timings';
import type { EditorTimings } from '../src/features/workflows/timings';
import { AnnouncerProvider } from '../src/ui/Announcer';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from './render';

export const WF_ID = '11111111-1111-4111-8111-111111111111';
export const NS_ID = '22222222-2222-4222-8222-222222222222';

export const ALL_PERMISSIONS = ['definition:read', 'definition:write', 'definition:publish', 'run:read', 'run:operate', 'saga:override', 'script:read'];

/** A small valid-looking workflow: a condition, a step, an end. */
export function sampleIr(): Record<string, unknown> {
  return {
    apiVersion: 'sagat-octopus-orchestrator.io/v1',
    kind: 'Workflow',
    metadata: { name: 'order-flow' },
    entrypoint: 'check',
    triggers: [{ kind: 'manual' }],
    nodes: [
      { id: 'check', type: 'condition', expression: 'input.amount > 10', ifTrue: 'pay', ifFalse: 'done' },
      { id: 'pay', type: 'activity', activity: 'payment.submit', timeoutMs: 30_000, next: 'done' },
      { id: 'done', type: 'end' },
    ],
  };
}

export function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

export function apiError(status: number, code: string, message: string, traceId = 'b'.repeat(32)): Response {
  return json({ code, message, traceId }, status);
}

export interface PlatformState {
  revision: number;
  ir: Record<string, unknown>;
  name: string;
  activeVersion: number | undefined;
  versions: {
    versionId: string;
    workflowId: string;
    versionNumber: number;
    digest: string;
    publishedBy?: string;
    publishedAt?: string;
    releaseNotes?: string;
    document: Record<string, unknown>;
  }[];
}

export interface Behaviour {
  /** What the platform answers (a body, or a `Response` for a failure); nothing means its usual answer. */
  validate?: (ir: Record<string, unknown>, state: PlatformState) => unknown;
  diff?: (url: string, state: PlatformState) => unknown;
  publish?: (body: { draftRevision: number; releaseNotes?: string }, state: PlatformState) => unknown;
  put?: (body: { draftRevision: number; ir: Record<string, unknown> }, state: PlatformState) => Response | undefined;
  get?: (state: PlatformState) => unknown;
  extra?: Parameters<typeof fakePlatform>[0];
}

export function emptyDiff(): Record<string, unknown> {
  return {
    from: { kind: 'version', number: 1 },
    to: { kind: 'draft', revision: 2 },
    identical: false,
    nodes: { added: [], removed: [], changed: [] },
    edges: { added: [], removed: [] },
    changes: [],
    breaking: [],
    attention: [],
  };
}

/** A platform that keeps one draft the way the real one does: compare-and-swap on the revision. */
export function createPlatform(state: Partial<PlatformState> = {}, behaviour: Behaviour = {}) {
  const current: PlatformState = { revision: 1, ir: sampleIr(), name: 'order-flow', activeVersion: undefined, versions: [], ...state };
  const draftBody = () => ({
    id: WF_ID,
    name: current.name,
    draftRevision: current.revision,
    ir: current.ir,
    ...(current.activeVersion === undefined
      ? {}
      : { activeVersion: { workflowId: WF_ID, versionId: 'v', versionNumber: current.activeVersion, digest: 'sha256:' + 'a'.repeat(64) } }),
  });
  const body = (init: RequestInit): never => bodyOf(init);
  const platform = fakePlatform({
    'GET /namespaces': () => ({ items: [{ id: NS_ID, workspaceId: NS_ID, slug: 'dev', environment: 'dev', createdAt: '2026-01-01T00:00:00Z' }] }),
    [`GET /workflows/${WF_ID}`]: () => behaviour.get?.(current) ?? draftBody(),
    [`PUT /workflows/${WF_ID}`]: (init) => {
      const sent = body(init) as { draftRevision: number; ir: Record<string, unknown> };
      const forced = behaviour.put?.(sent, current);
      if (forced) return forced;
      if (sent.draftRevision !== current.revision) return apiError(409, 'CONFLICT', 'The draft changed.');
      current.revision += 1;
      current.ir = sent.ir;
      return draftBody();
    },
    [`POST /workflows/${WF_ID}/validate`]: (init) =>
      behaviour.validate?.((body(init) as { ir: Record<string, unknown> }).ir, current) ?? { valid: true, problems: [], warnings: [] },
    [`GET /workflows/${WF_ID}/diff`]: (_init, url) => behaviour.diff?.(url, current) ?? emptyDiff(),
    [`POST /workflows/${WF_ID}/publish`]: (init) => {
      const sent = body(init) as { draftRevision: number; releaseNotes?: string };
      const forced = behaviour.publish?.(sent, current);
      if (forced) return forced;
      if (sent.draftRevision !== current.revision) return apiError(409, 'CONFLICT', 'The draft moved on.');
      const number = (current.activeVersion ?? 0) + 1;
      current.activeVersion = number;
      return { workflowId: WF_ID, versionId: `ver-${number}`, versionNumber: number, digest: 'sha256:' + 'b'.repeat(64), created: true, warnings: [] };
    },
    [`GET /workflows/${WF_ID}/versions`]: () => ({ items: [...current.versions].reverse(), nextCursor: null }),
    ...Object.fromEntries(
      [1, 2, 3].map((n) => [`GET /workflows/${WF_ID}/versions/${n}`, () => current.versions.find((v) => v.versionNumber === n) ?? apiError(404, 'NOT_FOUND', 'no such version')]),
    ),
    ...behaviour.extra,
  });
  return { ...platform, state: current };
}

export type Platform = ReturnType<typeof createPlatform>;

export interface MountOptions {
  permissions?: string[];
  state?: Partial<PlatformState>;
  behaviour?: Behaviour;
  /** The waits the editor really has, with no test shortening them (use with fake timers). */
  realTimings?: boolean;
  timings?: Partial<EditorTimings>;
  path?: string;
  /** Open in the list view instead of the canvas (the stored preference). */
  view?: 'canvas' | 'list';
  /** Keep the browser storage of an earlier mount instead of starting empty. */
  keepStorage?: boolean;
  /** Entries to put in the browser storage before the editor opens. */
  seed?: Record<string, string>;
  /** Sees every request before the platform does: to hold an answer back, or to fail one. */
  wrapFetch?: (inner: typeof fetch) => typeof fetch;
  /** Render under React's StrictMode, which runs every effect twice in development. */
  strict?: boolean;
  /** The test runs on fake timers that still advance with the clock. */
  fakeTimers?: boolean;
}

async function signedIn(platform: Platform, permissions: string[], wrap?: MountOptions['wrapFetch']): Promise<App> {
  const app = createApp({ fetch: wrap ? wrap(platform.fetcher) : platform.fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions } as never));
  app.namespace.set('dev');
  return app;
}

/** The editor page on its route, over a platform that behaves like the real one. */
function prepareStorage(options: MountOptions): void {
  if (!options.keepStorage || typeof globalThis.localStorage?.getItem !== 'function') installStorage();
  globalThis.localStorage.setItem('studio.scripts.plainEditor', '1');
  for (const [key, value] of Object.entries(options.seed ?? {})) localStorage.setItem(key, value);
}

export async function mountEditor(options: MountOptions = {}) {
  prepareStorage(options);
  if (options.view) localStorage.setItem('studio.workflows.view', options.view);
  const platform = createPlatform(options.state, options.behaviour);
  const app = await signedIn(platform, options.permissions ?? ALL_PERMISSIONS, options.wrapFetch);
  const timings = options.realTimings ? DEFAULT_TIMINGS : { validateMs: 15, autosaveMs: 40, ...options.timings };
  const user = userEvent.setup(options.fakeTimers ? { advanceTimers: (ms) => void vi.advanceTimersByTime(ms) } : {});
  const page = (
    <TimingsContext.Provider value={timings}>
      <Routes>
        <Route path="/workflows/:workflowId" element={<WorkflowEditorPage />} />
        <Route path="/workflows" element={<p>the list of workflows</p>} />
      </Routes>
    </TimingsContext.Provider>
  );
  const view = renderApp(options.strict ? <StrictMode>{page}</StrictMode> : page, { app, path: options.path ?? `/workflows/${WF_ID}` });
  return { ...view, app, platform, user };
}

/**
 * A data router builds a `Request` with the abort signal of its own controller; here that signal is
 * jsdom's and the `Request` is Node's, and Node refuses the pair. Routers do not need the link in a test.
 */
function tolerateJsdomSignals(): void {
  const Native = globalThis.Request;
  if ((Native as unknown as { tolerant?: true }).tolerant) return;
  class TolerantRequest extends Native {
    static tolerant = true as const;
    constructor(input: RequestInfo | URL, init?: RequestInit) {
      super(input, init ? { ...init, signal: undefined } : init);
    }
  }
  globalThis.Request = TolerantRequest;
}

/** The same page under a data router, which is what the Studio runs on (a leave guard needs it). */
export async function mountEditorInDataRouter(options: MountOptions = {}) {
  prepareStorage(options);
  tolerateJsdomSignals();
  if (options.view) localStorage.setItem('studio.workflows.view', options.view);
  const platform = createPlatform(options.state, options.behaviour);
  const app = await signedIn(platform, options.permissions ?? ALL_PERMISSIONS, options.wrapFetch);
  const timings = { ...DEFAULT_TIMINGS, validateMs: 15, autosaveMs: 40, ...options.timings };
  const router = createMemoryRouter(
    [
      { path: '/workflows/:workflowId', element: <WorkflowEditorPage /> },
      { path: '/workflows', element: <p>the list of workflows</p> },
    ],
    { initialEntries: [options.path ?? `/workflows/${WF_ID}`] },
  );
  const user = userEvent.setup(options.fakeTimers ? { advanceTimers: (ms) => void vi.advanceTimersByTime(ms) } : {});
  const view = render(
    <AppContext.Provider value={app}>
      <QueryClientProvider client={app.queries}>
        <AnnouncerProvider>
          <TimingsContext.Provider value={timings}>
            <RouterProvider router={router} />
          </TimingsContext.Provider>
        </AnnouncerProvider>
      </QueryClientProvider>
    </AppContext.Provider>,
  );
  return { ...view, app, platform, user, router };
}

/** The JSON a request carried. */
export function bodyOf<T = Record<string, unknown>>(init: RequestInit | undefined): T {
  return JSON.parse(typeof init?.body === 'string' ? init.body : '{}') as T;
}

/** The URL of a request as text, whatever form `fetch` was given it in. */
export function urlText(input: RequestInfo | URL): string {
  if (typeof input === 'string') return input;
  return input instanceof URL ? input.href : input.url;
}

/** The idempotency key a change request carried. */
export function keyOf(call: { init: RequestInit } | undefined): string | undefined {
  return new Headers(call?.init.headers).get('Idempotency-Key') ?? undefined;
}

export function putsOf(platform: Platform) {
  return platform.calls.filter((c) => c.method === 'PUT').map((c) => ({ body: bodyOf<{ draftRevision: number; ir: Record<string, unknown> }>(c.init), key: keyOf(c) }));
}

export function validationsOf(platform: Platform) {
  return platform.calls.filter((c) => c.method === 'POST' && c.url.includes('/validate'));
}

/** Lets timers and requests that are already due run. */
export function settle(ms = 120): Promise<void> {
  return act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

/** A request held back until the test lets it go. */
export function gate<T>() {
  let release: (value: T) => void = () => undefined;
  const opened = new Promise<T>((resolve) => {
    release = resolve;
  });
  return { opened, release };
}

export function urlOf(call: { url: string }): URL {
  return new URL(call.url, 'http://studio.test');
}
