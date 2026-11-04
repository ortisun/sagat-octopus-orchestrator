import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Route, Routes } from 'react-router';
import { describe, expect, it } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import type { Routes as Scripted } from '../../../test/render';
import type { BatchJob } from '../../api/endpoints';
import { createApp } from '../../app/context';
import { BatchPage } from './BatchPage';

const NS = '0194f5e2-7b1c-7000-8000-0000000000a1';
const JOB = '0194f5e2-7b1c-7000-8000-0000000000b2';
const BASE = `/namespaces/${NS}/batch-operations/${JOB}`;
const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
const namespaces = () => ({ items: [{ id: NS, workspaceId: NS, slug: 'payments', environment: 'dev', createdAt: '2026-11-04T10:00:00Z' }], nextCursor: null });

const job = (changes: Partial<BatchJob> = {}): BatchJob => ({
  batchId: JOB,
  action: 'RETRY_STEP',
  state: 'RUNNING',
  createdBy: 'alice',
  createdAt: '2026-11-04T10:00:00Z',
  maxItems: 10000,
  itemsPerSecond: 20,
  counts: { selected: 40, succeeded: 20, skipped: 4, failed: 1, cancelled: 0, deferred: 3 },
  ...changes,
});
const item = (ordinal: number, state: string, extra: Record<string, unknown> = {}) => ({ ordinal, runId: `0194f5e2-7b1c-7000-8000-00000000${String(ordinal).padStart(4, '0')}`, state, attempts: 1, updatedAt: '2026-11-04T10:00:01Z', ...extra });

async function mount(options: { permissions?: string[]; routes?: Scripted } = {}) {
  const { permissions = ['run:read', 'run:batch'] } = options;
  const platform = fakePlatform({
    'GET /namespaces': namespaces,
    [`GET ${BASE}`]: () => job(),
    [`GET ${BASE}/items`]: () => ({ items: [item(1, 'SUCCEEDED'), item(2, 'SKIPPED', { reason: 'NOT_WAITING' }), item(3, 'FAILED', { errorCode: 'CONFLICT' })], nextCursor: null }),
    ...options.routes,
  });
  const app = createApp({ fetch: platform.fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions } as never));
  app.namespace.set('payments');
  renderApp(
    <Routes>
      <Route path="/batches/:batchId" element={<BatchPage />} />
    </Routes>,
    { app, path: `/batches/${JOB}` },
  );
  return { app, platform, user: userEvent.setup() };
}

describe('A batch job', () => {
  it('shows how far it is, in words and in numbers, and what each run became', async () => {
    await mount();
    expect(await screen.findByRole('heading', { name: /Batch job: Retry a step/ })).toBeVisible();
    expect(screen.getByText(/25 of 40 runs have an outcome \(62%\)/)).toBeVisible();
    expect(screen.getByRole('progressbar', { name: 'Runs with an outcome' })).toHaveAttribute('value', '25');
    const counts = within(screen.getByText('Succeeded', { selector: 'dt' }).closest('dl') as HTMLElement);
    expect(counts.getByText('To be tried again').nextSibling).toHaveTextContent('3');
    const table = await screen.findByRole('table', { name: /what became of each/ });
    const rows = within(table).getAllByRole('row').slice(1);
    expect(rows).toHaveLength(3);
    expect(within(rows[1]!).getByText(/No step of the run was waiting for this/)).toBeVisible();
    expect(within(rows[2]!).getByText('CONFLICT')).toBeVisible();
    expect(within(rows[0]!).getByRole('link')).toHaveAttribute('href', '/runs/0194f5e2-7b1c-7000-8000-000000000001');
  });

  it('does not show the payload it carries, nor anything that could confirm a guess of it, only that it has one', async () => {
    await mount({ routes: { [`GET ${BASE}`]: () => job({ action: 'SIGNAL', hasPayload: true }) } });
    expect(await screen.findByText(/carries a payload, which is not shown/)).toBeVisible();
    expect(screen.queryByText(/digest/i)).not.toBeInTheDocument();
  });

  it('asks the platform for the outcome a person filters by', async () => {
    const { user, platform } = await mount();
    await screen.findByRole('table', { name: /what became of each/ });
    await user.selectOptions(screen.getByLabelText('Outcome'), 'SKIPPED');
    await waitFor(() => expect(platform.calls.some((call) => call.url.includes('/items') && call.url.includes('state=SKIPPED'))).toBe(true));
  });

  it('says why a job that failed stopped', async () => {
    await mount({ routes: { [`GET ${BASE}`]: () => job({ state: 'FAILED', failureReason: 'AUTHORITY_REVOKED' }) } });
    expect(await screen.findByText(/lost a permission it needs/)).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Cancel the job' })).not.toBeInTheDocument();
  });

  it('is not cancellable once it has ended, nor by a person who may not', async () => {
    await mount({ routes: { [`GET ${BASE}`]: () => job({ state: 'COMPLETED' }) } });
    await screen.findByRole('heading', { name: /Batch job/ });
    expect(screen.queryByRole('button', { name: 'Cancel the job' })).not.toBeInTheDocument();
  });

  it('offers no cancellation without run:batch', async () => {
    await mount({ permissions: ['run:read'] });
    await screen.findByRole('heading', { name: /Batch job/ });
    expect(screen.queryByRole('button', { name: 'Cancel the job' })).not.toBeInTheDocument();
  });

  it('is cancelled only after a reason, and says what stays applied', async () => {
    const { user, platform } = await mount({ routes: { [`POST ${BASE}/cancel`]: () => job({ state: 'CANCELLING' }) } });
    await user.click(await screen.findByRole('button', { name: 'Cancel the job' }));
    const dialog = within(screen.getByRole('dialog'));
    expect(dialog.getByText(/What was already applied stays applied/)).toBeVisible();
    await user.type(dialog.getByLabelText('Reason'), 'short');
    await user.click(dialog.getByRole('button', { name: 'Confirm cancellation' }));
    expect(dialog.getByRole('alert')).toHaveTextContent('at least 8');
    expect(platform.calls.filter((call) => call.method === 'POST')).toHaveLength(0);
    await user.clear(dialog.getByLabelText('Reason'));
    await user.type(dialog.getByLabelText('Reason'), 'the provider is down');
    await user.click(dialog.getByRole('button', { name: 'Confirm cancellation' }));
    await waitFor(() => expect(platform.calls.filter((call) => call.method === 'POST')).toHaveLength(1));
    const [call] = platform.calls.filter((c) => c.method === 'POST');
    expect(JSON.parse(call!.init.body as string)).toEqual({ reason: 'the provider is down' });
    await waitFor(() => expect(screen.queryByRole('dialog', { hidden: false })).not.toBeInTheDocument());
  });

  it('says so when the deployment does not serve batch operations', async () => {
    await mount({ routes: { [`GET ${BASE}`]: () => json(404, { code: 'NOT_FOUND', message: 'resource not found', retryable: false, details: { reason: 'ROUTE_NOT_FOUND' } }) } });
    expect(await screen.findByText(/not enabled on this deployment/)).toBeVisible();
  });

  it('is not shown to a person without run:read', async () => {
    const { platform } = await mount({ permissions: [] });
    expect(await screen.findByText(/run:read/)).toBeVisible();
    expect(platform.calls.filter((call) => call.url.includes('batch-operations'))).toHaveLength(0);
  });
});
