import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import { createApp } from '../../app/context';
import { OperationsPage } from './OperationsPage';

const namespaces = [
  { id: 'ns-1', workspaceId: 'workspace-1', slug: 'payments', environment: 'prod', createdAt: '2026-10-01T00:00:00Z' },
  { id: 'ns-2', workspaceId: 'workspace-1', slug: 'other', environment: 'dev', createdAt: '2026-10-01T00:00:00Z' },
];

const alert = (id: string, overrides: Record<string, unknown> = {}) => ({
  id,
  namespaceId: 'ns-1',
  kind: 'OUTBOX_STUCK',
  subject: '',
  state: 'FIRING',
  severity: 'critical',
  summary: 'Outbox messages have waited longer than the alert threshold.',
  runbook: 'docs/runbooks/alerts.md#outbox-stuck',
  firstSeenAt: '2026-10-12T10:00:00Z',
  lastSeenAt: '2026-10-12T10:05:00Z',
  firedCount: 1,
  items: 4,
  oldestSeconds: 120,
  ...overrides,
});

const worker = (workerId: string, subject: string, name: string, overrides: Record<string, unknown> = {}) => ({
  workerId,
  subject,
  name,
  activities: ['payments.authorize'],
  languages: ['typescript'],
  pools: ['default'],
  maxInflight: 8,
  inFlight: 0,
  registeredAt: '2026-10-12T10:00:00Z',
  expiresAt: '2026-10-12T10:01:00Z',
  draining: false,
  ...overrides,
});

async function mount(options: { permissions?: string[]; drainFailure?: boolean } = {}) {
  let drains = [{ subject: 'retired-worker', reason: 'Host retired safely', drainedBy: 'ops-user', drainedAt: '2026-10-12T09:00:00Z' }];
  const platform = fakePlatform({
    'GET /namespaces': () => ({ items: namespaces, nextCursor: null }),
    'GET /namespaces/ns-1/alerts': (_init, url) => {
      const query = new URL(url, 'http://studio.test').searchParams;
      if (query.get('state') === 'RESOLVED') return { items: [alert('alert-resolved', { state: 'RESOLVED', severity: 'warning' })], nextCursor: null };
      if (query.get('cursor') === 'alert-next') return { items: [alert('alert-2', { kind: 'REVIEW_PENDING', severity: 'warning' })], nextCursor: null };
      return {
        items: [
          alert('alert-1', { runbook: 'https://runbooks.example.test/alerts#outbox-stuck' }),
          alert('alert-unsafe', { kind: 'DEAD_ITEMS', severity: 'warning', runbook: 'javascript:alert(document.cookie)' }),
        ],
        nextCursor: 'alert-next',
      };
    },
    'GET /namespaces/ns-1/workers': () => ({
      items: [worker('session-a', 'worker/alpha', 'alpha-a'), worker('session-b', 'worker/alpha', 'alpha-b', { inFlight: 2 })],
      drains,
    }),
    'GET /namespaces/ns-2/alerts': () => ({ items: [], nextCursor: null }),
    'GET /namespaces/ns-2/workers': () => ({ items: [], drains: [] }),
    'POST /namespaces/ns-1/workers/worker%2Falpha/drain': (init) => {
      if (options.drainFailure) {
        return new Response(JSON.stringify({ code: 'UNAVAILABLE', message: 'The operation store is unavailable.', retryable: true, traceId: 'd'.repeat(32) }), {
          status: 503,
          headers: { 'content-type': 'application/json' },
        });
      }
      const reason = (JSON.parse(init.body as string) as { reason: string }).reason;
      const record = { subject: 'worker/alpha', reason, drainedBy: 'alice', drainedAt: '2026-10-12T11:00:00Z' };
      drains = [...drains, record];
      return record;
    },
    'POST /namespaces/ns-1/workers/retired-worker/undrain': () => {
      drains = drains.filter((item) => item.subject !== 'retired-worker');
      return { wasDraining: true };
    },
  });
  const app = createApp({ fetch: platform.fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, traceId: 'session-a', permissions: options.permissions ?? ['ops:read', 'worker:manage'] } as never));
  app.namespace.set('payments');
  renderApp(<OperationsPage />, { app });
  return { app, platform, user: userEvent.setup() };
}

describe('OperationsPage', () => {
  it('gates reads and worker actions with their separate permissions', async () => {
    const denied = await mount({ permissions: [] });
    expect(await screen.findByRole('heading', { name: 'Operations' })).toBeInTheDocument();
    expect(screen.getByText(/ops:read/)).toBeInTheDocument();
    expect(denied.platform.calls).toHaveLength(0);

    denied.app.queries.clear();
    const readOnly = await mount({ permissions: ['ops:read'] });
    await screen.findByText('worker/alpha');
    expect(screen.getByText(/worker:manage/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /worker identity/ })).not.toBeInTheDocument();
    expect(readOnly.platform.calls.some((call) => call.url.includes('/alerts'))).toBe(true);
  });

  it('pages alerts, filters by state and never links an unsafe runbook URL', async () => {
    const { user, platform } = await mount();
    expect(await screen.findByText('Outbox stuck')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Open runbook for Outbox stuck' })).toHaveAttribute('href', 'https://runbooks.example.test/alerts#outbox-stuck');
    expect(screen.getByText('javascript:alert(document.cookie)').closest('a')).toBeNull();

    await user.click(screen.getByRole('button', { name: 'Load more alerts' }));
    expect(await screen.findByText('Review pending')).toBeInTheDocument();
    expect(platform.calls.some((call) => call.url.includes('cursor=alert-next'))).toBe(true);

    await user.selectOptions(screen.getByLabelText('Alert state'), 'RESOLVED');
    await waitFor(() => expect(screen.getAllByText('Resolved').some((item) => item.tagName === 'SPAN')).toBe(true));
    expect(screen.getByText('docs/runbooks/alerts.md#outbox-stuck').closest('a')).toBeNull();
    await waitFor(() => expect(platform.calls.some((call) => call.url.includes('state=RESOLVED'))).toBe(true));
  });

  it('requires an audited reason, explains identity scope and drains and resumes canonical subjects', async () => {
    const { user, platform } = await mount();
    await screen.findByText('worker/alpha');
    const persistent = screen.getByRole('group', { name: 'Worker identity retired-worker' });
    expect(within(persistent).getByText('No sessions are currently listed for this identity.')).toBeInTheDocument();
    expect(within(persistent).getByText('Host retired safely')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Drain worker identity worker/alpha' }));
    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveTextContent('every session of worker/alpha in payments');
    expect(dialog).toHaveTextContent('does not cancel attempts already in flight');
    await user.type(within(dialog).getByLabelText('Reason'), 'short');
    await user.click(within(dialog).getByRole('button', { name: 'Confirm drain' }));
    expect(within(dialog).getByRole('alert')).toHaveTextContent('at least 8 characters');
    await user.clear(within(dialog).getByLabelText('Reason'));
    await user.type(within(dialog).getByLabelText('Reason'), 'Rolling deployment of worker version 2');
    await user.click(within(dialog).getByRole('button', { name: 'Confirm drain' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());

    const drain = platform.calls.find((call) => call.method === 'POST' && call.url.endsWith('/worker%2Falpha/drain'));
    expect(JSON.parse(drain?.init.body as string) as unknown).toEqual({ reason: 'Rolling deployment of worker version 2' });

    await user.click(screen.getByRole('button', { name: 'Resume worker identity retired-worker' }));
    await user.type(screen.getByLabelText('Reason'), 'Host replacement is ready');
    await user.click(screen.getByRole('button', { name: 'Confirm resume' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(platform.calls.some((call) => call.method === 'POST' && call.url.endsWith('/retired-worker/undrain'))).toBe(true);
  });

  it('keeps an actionable platform failure in the confirmation dialog', async () => {
    const { user } = await mount({ drainFailure: true });
    await user.click(await screen.findByRole('button', { name: 'Drain worker identity worker/alpha' }));
    await user.type(screen.getByLabelText('Reason'), 'Investigating repeated worker failures');
    await user.click(screen.getByRole('button', { name: 'Confirm drain' }));
    const error = await screen.findByRole('alert');
    expect(error).toHaveTextContent('The operation store is unavailable.');
    expect(error).toHaveTextContent(`Trace ${'d'.repeat(32)}.`);
    expect(screen.getByRole('button', { name: 'Try again' })).toBeInTheDocument();
    expect(screen.getByLabelText('Reason')).toHaveValue('Investigating repeated worker failures');
  });

  it('aborts a mutation and discards its form when the namespace changes', async () => {
    let mutationStarted = false;
    let mutationAborted = false;
    const base = fakePlatform({
      'GET /namespaces': () => ({ items: namespaces, nextCursor: null }),
      'GET /namespaces/ns-1/alerts': () => ({ items: [], nextCursor: null }),
      'GET /namespaces/ns-1/workers': () => ({ items: [worker('session-a', 'worker/alpha', 'alpha-a')], drains: [] }),
      'GET /namespaces/ns-2/alerts': () => ({ items: [], nextCursor: null }),
      'GET /namespaces/ns-2/workers': () => ({ items: [], drains: [] }),
    });
    const fetcher = ((url: string, init: RequestInit = {}) => {
      if ((init.method ?? 'GET') === 'POST') {
        mutationStarted = true;
        return new Promise<Response>((_resolve, reject) => {
          init.signal?.addEventListener('abort', () => {
            mutationAborted = true;
            reject(new DOMException('aborted', 'AbortError'));
          }, { once: true });
        });
      }
      return base.fetcher(url, init);
    }) as typeof fetch;
    const app = createApp({ fetch: fetcher });
    await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, traceId: 'session-late', permissions: ['ops:read', 'worker:manage'] } as never));
    app.namespace.set('payments');
    renderApp(<OperationsPage />, { app });
    const user = userEvent.setup();
    await user.click(await screen.findByRole('button', { name: 'Drain worker identity worker/alpha' }));
    await user.type(screen.getByLabelText('Reason'), 'Move away from this worker version');
    await user.click(screen.getByRole('button', { name: 'Confirm drain' }));
    await waitFor(() => expect(mutationStarted).toBe(true));

    act(() => app.namespace.set('other'));
    expect(await screen.findByText('No worker identities are currently listed.')).toBeInTheDocument();
    await waitFor(() => expect(mutationAborted).toBe(true));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.queryByText('Worker identity drained.')).not.toBeInTheDocument();
  });
});
