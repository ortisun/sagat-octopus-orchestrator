import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import { createApp } from '../../app/context';
import type { components } from '../../api/schema';
import { OperationsPage } from './OperationsPage';
import { keyLabelWords, poolWords, scopeWords } from './flowControlKeys';

// Feature 022 (T022-056, T022-085; FR-022-022, FR-022-048, FR-022-057): the keys of a namespace, oldest wait first.

type ConcurrencyKey = components['schemas']['ConcurrencyKey'];
type RateLimitKey = components['schemas']['RateLimitKey'];

const namespaces = [{ id: 'ns-1', workspaceId: 'workspace-1', slug: 'payments', environment: 'prod', createdAt: '2026-10-01T00:00:00Z' }];

const concurrencyKey = (over: Partial<ConcurrencyKey> = {}): ConcurrencyKey => ({
  pool: 'ORDINARY',
  scope: 'RUN',
  group: 'orders-by-account',
  keyLabel: '3f9a1c02be7d',
  holders: 1,
  waiters: 4,
  oldestWaitSince: '2026-11-24T10:00:05.123Z',
  oldestHolderSince: '2026-11-24T09:59:00.000Z',
  ...over,
});

const rateKey = (over: Partial<RateLimitKey> = {}): RateLimitKey => ({
  pool: 'ORDINARY',
  scope: 'STEP',
  group: '=provider-sync/fetch',
  keyLabel: '0c1d2e3f4a5b',
  grantsInWindow: 10,
  unleasedGrants: 2,
  waiters: 7,
  oldestWaitSince: '2026-11-24T10:01:00.000Z',
  nextSlotAt: null,
  ...over,
});

async function mount(routes: Record<string, (init: RequestInit, url: string) => unknown> = {}, permissions = ['ops:read']) {
  const platform = fakePlatform({
    'GET /namespaces': () => ({ items: namespaces, nextCursor: null }),
    'GET /namespaces/ns-1/alerts': () => ({ items: [], nextCursor: null }),
    'GET /namespaces/ns-1/workers': () => ({ items: [], drains: [] }),
    'GET /namespaces/ns-1/concurrency-keys': (_init, url) => {
      const query = new URL(url, 'http://studio.test').searchParams;
      if (query.get('cursor') === 'sealed-next') return { queuedRuns: 9, items: [concurrencyKey({ group: '=order-flow', keyLabel: null, holders: 2, waiters: 0, oldestWaitSince: null, oldestHolderSince: '2026-11-24T09:58:00Z' })], nextCursor: null };
      if (query.get('group') === 'orders-by-account') return { queuedRuns: 9, items: [concurrencyKey()], nextCursor: null };
      if (query.get('group') === 'Not A Group') return new Response(JSON.stringify({ code: 'VALIDATION_FAILED', message: 'The group is not a group name.', traceId: 'c'.repeat(32) }), { status: 422, headers: { 'content-type': 'application/json' } });
      return { queuedRuns: 9, items: [concurrencyKey(), concurrencyKey({ scope: 'STEP', group: 'crm', keyLabel: 'aa00bb11cc22', holders: 2, waiters: 1, oldestWaitSince: '2026-11-24T10:02:00Z' })], nextCursor: 'sealed-next' };
    },
    'GET /namespaces/ns-1/rate-limit-keys': () => ({ queuedRuns: 9, items: [rateKey(), rateKey({ scope: 'RUN', group: 'provider-starts', keyLabel: null, unleasedGrants: 0, waiters: 3, nextSlotAt: '2026-11-24T10:05:00Z', pool: 'DRAFT_TEST' })], nextCursor: null }),
    ...routes,
  });
  const app = createApp({ fetch: platform.fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions } as never));
  app.namespace.set('payments');
  renderApp(<OperationsPage />, { app });
  return { platform, user: userEvent.setup() };
}

const cells = (row: HTMLElement) => [...row.querySelectorAll('th, td')].map((cell) => cell.textContent);

describe('the words of a key', () => {
  it('names a hidden label, scopes and pools in words; an unknown one is unsupported', () => {
    expect(keyLabelWords(null)).toBe('Hidden');
    expect(keyLabelWords('3f9a1c02be7d')).toBe('3f9a1c02be7d');
    expect(keyLabelWords(undefined)).toBe('Unsupported (no label sent)');
    expect([scopeWords('RUN'), scopeWords('STEP'), scopeWords('BODY')]).toEqual(['Run starts', 'Step attempts', 'Unsupported (BODY)']);
    expect([poolWords('ORDINARY'), poolWords('DRAFT_TEST'), poolWords('BATCH')]).toEqual(['Ordinary', 'Draft test', 'Unsupported (BATCH)']);
  });
});

describe('the flow-control keys of a namespace', () => {
  it('lists the concurrency keys as a table, oldest wait first as the platform sent them, and pages on', async () => {
    const { platform, user } = await mount();
    const table = await screen.findByRole('table', { name: /Concurrency keys of payments/ });
    expect(within(table).getAllByRole('columnheader').map((h) => h.textContent)).toEqual(['Group', 'Applies to', 'Pool', 'Key label', 'Holders', 'Waiters', 'Oldest wait since', 'Oldest holder since']);
    const rows = within(table).getAllByRole('row').slice(1);
    expect(rows.map((row) => within(row).getByRole('rowheader').textContent)).toEqual(['orders-by-account', 'crm']);
    expect(cells(rows[0] as HTMLElement).slice(1, 6)).toEqual(['Run starts', 'Ordinary', '3f9a1c02be7d', '1', '4']);
    expect(screen.getByRole('region', { name: 'Concurrency keys' })).toHaveTextContent('9 runs of this namespace are queued');

    await user.click(screen.getByRole('button', { name: 'Load more concurrency keys' }));
    await waitFor(() => expect(within(table).getAllByRole('row')).toHaveLength(4));
    expect(platform.calls.some((call) => call.url.includes('/concurrency-keys') && call.url.includes('cursor=sealed-next'))).toBe(true);
    const last = within(table).getAllByRole('row')[3] as HTMLElement;
    expect(within(last).getByRole('rowheader')).toHaveTextContent('=order-flow');
    expect(cells(last)[3]).toBe('Hidden');
    expect(cells(last)[6]).toBe('Nobody waits');
    expect(screen.queryByRole('button', { name: 'Load more concurrency keys' })).not.toBeInTheDocument();
  });

  it('shows the rate-limit keys with their next slot, whole instants', async () => {
    await mount();
    const table = await screen.findByRole('table', { name: /Rate-limit keys of payments/ });
    expect(within(table).getAllByRole('columnheader').map((h) => h.textContent)).toEqual(['Group', 'Applies to', 'Pool', 'Key label', 'Grants in window', 'Unleased grants', 'Waiters', 'Oldest wait since', 'Next slot frees']);
    const second = within(table).getAllByRole('row')[2] as HTMLElement;
    expect(cells(second).slice(1, 4)).toEqual(['Run starts', 'Draft test', 'Hidden']);
    expect(cells(second)[8]).not.toMatch(/to the minute/);
    const shown = within(table).getAllByRole('row')[1] as HTMLElement;
    expect(cells(shown)[8]).toBe('Not by time: a granted step frees it when a worker leases it or it is given back');
    expect(screen.getByRole('region', { name: 'Rate-limit keys' })).not.toHaveTextContent(/to the minute/);
  });

  it('shows one row per group, and no key, to a caller the platform lists groups to (T022-111, D022-086)', async () => {
    const { platform, user } = await mount({
      'GET /namespaces/ns-1/concurrency-keys': (_init, url) => {
        const query = new URL(url, 'http://studio.test').searchParams;
        if (query.get('cursor') === 'sealed-groups') {
          return { view: 'GROUPS', queuedRuns: 3, items: [], groups: [{ pool: 'ORDINARY', scope: 'STEP', group: 'regions', holders: 1, waiters: 0, oldestWaitSince: null, oldestHolderSince: '2026-11-24T09:58:00Z' }], nextCursor: null };
        }
        return { view: 'GROUPS', queuedRuns: 3, items: [], groups: [{ pool: 'ORDINARY', scope: 'RUN', group: 'accounts', holders: 2, waiters: 3, oldestWaitSince: '2026-11-24T10:00:05.123Z', oldestHolderSince: '2026-11-24T09:59:00Z' }], nextCursor: 'sealed-groups' };
      },
      'GET /namespaces/ns-1/rate-limit-keys': () => ({ view: 'GROUPS', queuedRuns: 3, items: [], groups: [{ pool: 'DRAFT_TEST', scope: 'STEP', group: '=provider-sync/fetch', grantsInWindow: 10, unleasedGrants: 2, waiters: 7, oldestWaitSince: '2026-11-24T10:01:00Z' }], nextCursor: null }),
    });
    const table = await screen.findByRole('table', { name: /Concurrency keys of payments, one row per group/ });
    expect(within(table).getAllByRole('columnheader').map((h) => h.textContent)).toEqual(['Group', 'Applies to', 'Pool', 'Holders', 'Waiters', 'Oldest wait since', 'Oldest holder since']);
    const row = within(table).getAllByRole('row')[1] as HTMLElement;
    expect(cells(row).slice(0, 5)).toEqual(['accounts', 'Run starts', 'Ordinary', '2', '3']);
    expect(cells(row)[5]).not.toMatch(/to the minute/);
    const region = screen.getByRole('region', { name: 'Concurrency keys' });
    expect(region).toHaveTextContent(/one row per group, summed over its keys, and no key/);
    // decision D022-131: a group is read whole on one page; the note never says a group may take more than one page
    expect(region).toHaveTextContent(/each group read whole on one page/);
    expect(region).not.toHaveTextContent(/more than one page/);
    expect(region).not.toHaveTextContent(/Key label|Hidden/);
    await user.click(within(region).getByRole('button', { name: 'Load more concurrency keys' }));
    await waitFor(() => expect(within(table).getAllByRole('row')).toHaveLength(3));
    expect(platform.calls.some((call) => call.url.includes('cursor=sealed-groups'))).toBe(true);
    const rate = await screen.findByRole('table', { name: /Rate-limit keys of payments, one row per group/ });
    expect(within(rate).getAllByRole('columnheader').map((h) => h.textContent)).toEqual(['Group', 'Applies to', 'Pool', 'Grants in window', 'Unleased grants', 'Waiters', 'Oldest wait since']);
    expect(cells(within(rate).getAllByRole('row')[1] as HTMLElement).slice(3, 6)).toEqual(['10', '2', '7']);
  });

  it('never marks a group row as read across pages: each group is read whole (D022-131)', async () => {
    const { user } = await mount({
      'GET /namespaces/ns-1/concurrency-keys': (_init, url) => {
        const query = new URL(url, 'http://studio.test').searchParams;
        if (query.get('cursor') === 'sealed-groups') {
          return { view: 'GROUPS', queuedRuns: 3, items: [], groups: [{ pool: 'ORDINARY', scope: 'STEP', group: 'regions', holders: 7, waiters: 0, oldestWaitSince: null, oldestHolderSince: '2026-11-24T09:58:00Z', spansPages: true }], nextCursor: null };
        }
        return { view: 'GROUPS', queuedRuns: 3, items: [], groups: [{ pool: 'ORDINARY', scope: 'RUN', group: 'accounts', holders: 2, waiters: 3, oldestWaitSince: '2026-11-24T10:00:05.123Z', oldestHolderSince: '2026-11-24T09:59:00Z', spansPages: false }], nextCursor: 'sealed-groups' };
      },
      'GET /namespaces/ns-1/rate-limit-keys': () => ({ view: 'GROUPS', queuedRuns: 3, items: [], groups: [{ pool: 'ORDINARY', scope: 'STEP', group: '=provider-sync/fetch', grantsInWindow: 12, unleasedGrants: 0, waiters: 1, oldestWaitSince: '2026-11-24T10:01:00Z', spansPages: true }], nextCursor: null }),
    });
    const table = await screen.findByRole('table', { name: /Concurrency keys of payments, one row per group/ });
    const first = within(within(table).getAllByRole('row')[1] as HTMLElement).getByRole('rowheader');
    expect(first).toHaveTextContent(/^accounts$/);
    await user.click(screen.getByRole('button', { name: 'Load more concurrency keys' }));
    await waitFor(() => expect(within(table).getAllByRole('row')).toHaveLength(3));
    // `spansPages` is deprecated and always false (D022-131); a `true` sent by an older platform is not shown either
    const second = within(within(table).getAllByRole('row')[2] as HTMLElement).getByRole('rowheader');
    expect(second).toHaveTextContent(/^regions$/);
    expect(second).toHaveAccessibleName('regions');
    const rate = await screen.findByRole('table', { name: /Rate-limit keys of payments, one row per group/ });
    const rateRow = within(within(rate).getAllByRole('row')[1] as HTMLElement).getByRole('rowheader');
    expect(rateRow).toHaveAccessibleName('=provider-sync/fetch');
    expect(screen.queryByText(/read across pages/i)).toBeNull();
  });

  it('filters by group with the keyboard and shows the platform refusal of a group that is no group', async () => {
    const { platform, user } = await mount();
    await screen.findByRole('table', { name: /Concurrency keys of payments/ });
    const region = screen.getByRole('region', { name: 'Concurrency keys' });
    const filter = within(region).getByRole('textbox', { name: 'Group' });
    await user.click(filter);
    await user.keyboard('orders-by-account{Enter}');
    await waitFor(() => expect(platform.calls.some((call) => call.url.includes('/concurrency-keys') && call.url.includes('group=orders-by-account'))).toBe(true));
    await waitFor(() => expect(within(within(region).getByRole('table')).getAllByRole('row')).toHaveLength(2));
    expect(within(region).getByRole('table')).toHaveAccessibleName(/group orders-by-account/);

    await user.clear(filter);
    await user.keyboard('Not A Group{Enter}');
    expect(await within(region).findByText(/The group is not a group name/)).toBeInTheDocument();

    await user.clear(filter);
    await user.keyboard('{Enter}');
    await waitFor(() => expect(within(within(region).getByRole('table')).getAllByRole('row')).toHaveLength(3));
  });

  it('says when a namespace has no live key', async () => {
    await mount({
      'GET /namespaces/ns-1/concurrency-keys': () => ({ queuedRuns: 0, items: [], nextCursor: null }),
      'GET /namespaces/ns-1/rate-limit-keys': () => ({ queuedRuns: 0, items: [], nextCursor: null }),
    });
    expect(await screen.findByText('No concurrency key has a holder or a waiter in this namespace.')).toBeInTheDocument();
    expect(await screen.findByText('No rate-limit key has a counted grant or a waiter in this namespace.')).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Concurrency keys' })).toHaveTextContent('No run of this namespace is queued.');
  });

  it('shows the listings unavailable, not as an error, on a platform that has no such route (FR-007-010)', async () => {
    const notFound = () => new Response(JSON.stringify({ code: 'NOT_FOUND', message: 'no route', traceId: 'd'.repeat(32) }), { status: 404, headers: { 'content-type': 'application/json' } });
    await mount({ 'GET /namespaces/ns-1/concurrency-keys': notFound, 'GET /namespaces/ns-1/rate-limit-keys': notFound });
    const region = await screen.findByRole('region', { name: 'Concurrency keys' });
    expect(await within(region).findByText('Unavailable: this platform does not list concurrency keys.')).toBeInTheDocument();
    expect(await screen.findByText('Unavailable: this platform does not list rate-limit keys.')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(within(region).queryByRole('search')).not.toBeInTheDocument();
  });

  it('reads nothing without ops:read', async () => {
    const { platform } = await mount({}, []);
    expect(await screen.findByRole('heading', { name: 'Operations' })).toBeInTheDocument();
    expect(platform.calls.some((call) => call.url.includes('-keys'))).toBe(false);
  });

  it('the table scrolls in a region the keyboard reaches', async () => {
    await mount();
    const table = await screen.findByRole('table', { name: /Concurrency keys of payments/ });
    const scroller = table.closest('[role="region"][tabindex="0"]');
    expect(scroller).not.toBeNull();
    expect(scroller).toHaveAccessibleName('Concurrency keys, scrollable');
  });
});
