import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import type { Routes as Scripted } from '../../../test/render';
import type { BatchJob } from '../../api/endpoints';
import { createApp } from '../../app/context';
import { BatchesPage } from './BatchesPage';

const NS = '0194f5e2-7b1c-7000-8000-0000000000a1';
const LIST = `GET /namespaces/${NS}/batch-operations`;
const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
const namespaces = () => ({ items: [{ id: NS, workspaceId: NS, slug: 'payments', environment: 'dev', createdAt: '2026-11-04T10:00:00Z' }], nextCursor: null });
const job = (n: number, changes: Partial<BatchJob> = {}): BatchJob => ({
  batchId: `0194f5e2-7b1c-7000-8000-00000000b${String(n).padStart(3, '0')}`,
  action: 'CANCEL',
  state: 'COMPLETED',
  createdBy: 'alice',
  createdAt: '2026-11-04T10:00:00Z',
  counts: { selected: 10, succeeded: 8, skipped: 2, failed: 0, cancelled: 0, deferred: 0 },
  ...changes,
});

async function mount(options: { permissions?: string[]; routes?: Scripted } = {}) {
  const { permissions = ['run:read'] } = options;
  const platform = fakePlatform({ 'GET /namespaces': namespaces, [LIST]: () => ({ items: [job(2, { action: 'PAUSE', state: 'FAILED' }), job(1)], nextCursor: null }), ...options.routes });
  const app = createApp({ fetch: platform.fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions } as never));
  app.namespace.set('payments');
  renderApp(<BatchesPage />, { app });
  return { platform, user: userEvent.setup() };
}

describe('The batch jobs of the namespace', () => {
  it('lists them with their state and how far each is, linked to their page', async () => {
    await mount();
    const table = await screen.findByRole('table', { name: /newest first/ });
    const rows = within(table).getAllByRole('row').slice(1);
    expect(rows).toHaveLength(2);
    expect(within(rows[0]!).getByText('PAUSE')).toBeVisible();
    expect(within(rows[0]!).getByText('Failed')).toBeVisible();
    expect(within(rows[1]!).getByText('Completed')).toBeVisible();
    expect(within(rows[1]!).getByText('10 of 10')).toBeVisible();
    expect(within(rows[1]!).getByRole('link')).toHaveAttribute('href', '/batches/0194f5e2-7b1c-7000-8000-00000000b001');
  });

  it('tells the jobs apart by the end of their identifiers: they are time-ordered, so the start is the same for every job made within a minute', async () => {
    await mount();
    const table = await screen.findByRole('table', { name: /newest first/ });
    const names = within(table).getAllByRole('link').map((link) => link.textContent);
    expect(names).toEqual(['0000b002', '0000b001']);
  });

  it('asks only for the jobs in the state a person filters by', async () => {
    const { user, platform } = await mount();
    await screen.findByRole('table');
    await user.selectOptions(screen.getByLabelText('State'), 'RUNNING');
    await waitFor(() => expect(platform.calls.some((call) => call.url.includes('/batch-operations?') && call.url.includes('state=RUNNING'))).toBe(true));
  });

  it('loads more when there is more', async () => {
    const { user, platform } = await mount({
      routes: { [LIST]: (_init, url) => (url.includes('cursor=next') ? { items: [job(3)], nextCursor: null } : { items: [job(2), job(1)], nextCursor: 'next' }) },
    });
    await screen.findByRole('table');
    await user.click(screen.getByRole('button', { name: 'Load more' }));
    await waitFor(() => expect(within(screen.getByRole('table')).getAllByRole('row')).toHaveLength(4));
    expect(platform.calls.filter((call) => call.url.includes('cursor=next'))).toHaveLength(1);
    expect(screen.queryByRole('button', { name: 'Load more' })).not.toBeInTheDocument();
  });

  it('says when there is none yet', async () => {
    await mount({ routes: { [LIST]: () => ({ items: [], nextCursor: null }) } });
    expect(await screen.findByText(/No batch job yet/)).toBeVisible();
  });

  it('says so when the deployment does not serve batch operations', async () => {
    await mount({ routes: { [LIST]: () => json(404, { code: 'NOT_FOUND', message: 'resource not found', retryable: false, details: { reason: 'ROUTE_NOT_FOUND' } }) } });
    expect(await screen.findByText(/not enabled on this deployment/)).toBeVisible();
  });

  it('is not shown to a person without run:read, and asks the platform nothing', async () => {
    const { platform } = await mount({ permissions: [] });
    expect(await screen.findByText(/run:read/)).toBeVisible();
    expect(platform.calls).toHaveLength(0);
  });
});
