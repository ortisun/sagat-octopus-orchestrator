import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactElement } from 'react';
import { Link, Route, Routes } from 'react-router';
import type { Schedule, ScheduleActionRecord, ScheduleBackfill, ScheduleDescription } from '../src/api/endpoints';
import { createApp } from '../src/app/context';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from './render';
import type { Routes as Scripted } from './render';

export const NS = '0194f5e2-7b1c-7000-8000-0000000000a1';
export const ID = '0194f5e2-7b1c-7000-8000-0000000000c3';
export const WORKFLOW = '0194f5e2-7b1c-7000-8000-000000000001';
export const BASE = `/namespaces/${NS}/schedules`;

export const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
export const failure = (status: number, code: string, reason?: string) =>
  json(status, { code, message: reason ?? code, retryable: false, ...(reason ? { details: { reason } } : {}) });
export const namespaces = () => ({ items: [{ id: NS, workspaceId: NS, slug: 'payments', environment: 'dev', createdAt: '2026-11-04T10:00:00Z' }], nextCursor: null });
export const workflows = () => ({
  items: [
    { id: WORKFLOW, name: 'nightly-report', draftRevision: 1, activeVersion: { versionId: 'v1', versionNumber: 1 }, createdAt: '2026-11-01T00:00:00Z' },
    { id: '0194f5e2-7b1c-7000-8000-000000000002', name: 'unpublished', draftRevision: 1, activeVersion: null, createdAt: '2026-11-01T00:00:00Z' },
  ],
  nextCursor: null,
});

export function schedule(changes: Partial<Schedule> = {}): Schedule {
  return {
    scheduleId: ID,
    name: 'nightly',
    origin: 'API',
    state: 'ACTIVE',
    revision: 3,
    spec: { intervals: [{ everyMs: 7_200_000, offsetMs: 0 }], timezone: 'UTC', jitterMs: 0 },
    policies: { overlap: 'SKIP', catchupWindowMs: 31_622_400_000, catchupMode: 'ONE', pauseOnFailure: false, bufferLimit: 100 },
    action: { workflowId: WORKFLOW, versionPolicy: 'ACTIVE', inputDigest: 'ab'.repeat(32) },
    authorizedBy: 'alice',
    nextActionAt: '2026-11-05T02:00:00Z',
    createdAt: '2026-11-01T00:00:00Z',
    ...changes,
  };
}

export function described(changes: Partial<ScheduleDescription> = {}): ScheduleDescription {
  return {
    ...schedule(),
    nextActions: [
      { nominalTime: '2026-11-05T02:00:00Z', actualTime: '2026-11-05T02:00:00Z' },
      { nominalTime: '2026-11-05T04:00:00Z', actualTime: '2026-11-05T04:00:07Z' },
    ],
    recentActions: [],
    openRuns: [],
    bufferedCount: 0,
    counters: {},
    ...changes,
  };
}

export const decision = (changes: Partial<ScheduleActionRecord> = {}): ScheduleActionRecord => ({
  nominalTime: '2026-11-04T02:00:00Z',
  actualTime: '2026-11-04T02:00:00Z',
  kind: 'SCHEDULED',
  origin: 'SPEC',
  revision: 3,
  outcome: 'STARTED',
  decidedAt: '2026-11-04T02:00:01Z',
  runId: '0194f5e2-7b1c-7000-8000-0000000000d4',
  ...changes,
});

export const backfill = (changes: Partial<ScheduleBackfill> = {}): ScheduleBackfill => ({
  backfillId: '0194f5e2-7b1c-7000-8000-0000000000e5',
  start: '2026-11-01T00:00:00Z',
  end: '2026-11-02T00:00:00Z',
  state: 'RUNNING',
  overlap: 'BUFFER_ALL',
  counts: { STARTED: 3, ALREADY_ACTED: 1 },
  ...changes,
});

export const ALL = ['namespace:read', 'schedule:read', 'schedule:write', 'schedule:operate', 'run:start', 'definition:read'];

/** The Studio over a scripted platform, showing `element` at `path`; `landing` stands in for the pages a flow navigates to. */
export async function mountSchedules(element: ReactElement, options: { path: string; route: string; permissions?: string[]; routes?: Scripted } ) {
  const { permissions = ALL } = options;
  const platform = fakePlatform({
    'GET /namespaces': namespaces,
    'GET /workflows': workflows,
    [`GET ${BASE}`]: () => ({ items: [schedule()], nextCursor: null }),
    [`GET ${BASE}/${ID}`]: () => described(),
    [`GET ${BASE}/${ID}/actions`]: () => ({ items: [decision()], nextCursor: null }),
    [`GET ${BASE}/${ID}/backfills`]: () => ({ items: [] }),
    ...options.routes,
  });
  const app = createApp({ fetch: platform.fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions } as never));
  app.namespace.set('payments');
  renderApp(
    <Routes>
      <Route path={options.route} element={element} />
      <Route path="/schedules" element={<h1>Schedule list</h1>} />
      <Route
        path="/schedules/:scheduleId"
        element={
          <>
            <h1>Schedule landing</h1>
            <Link to={`/schedules/${ID}/edit`}>Edit again</Link>
          </>
        }
      />
    </Routes>,
    { app, path: options.path },
  );
  return { app, platform, user: userEvent.setup() };
}

export const writes = (platform: { calls: { method: string; url: string; init: RequestInit }[] }) => platform.calls.filter((call) => call.method !== 'GET');
export const bodyOf = (call: { init: RequestInit }) => JSON.parse(call.init.body as string) as Record<string, unknown>;
export const keyOf = (call: { init: RequestInit }) => new Headers(call.init.headers).get('Idempotency-Key');
export const heading = (name: RegExp | string) => screen.findByRole('heading', { name });
