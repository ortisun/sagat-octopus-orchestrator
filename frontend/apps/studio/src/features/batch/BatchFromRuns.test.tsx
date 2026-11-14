import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { Route, Routes } from 'react-router';
import { describe, expect, it, vi } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import type { Routes as Scripted } from '../../../test/render';
import { createApp } from '../../app/context';
import { BatchFromRuns } from './BatchFromRuns';
import type { RunFilters } from './model';

const NS = '0194f5e2-7b1c-7000-8000-0000000000a1';
const JOB = '0194f5e2-7b1c-7000-8000-0000000000b2';
const RUN = '0194f5e2-7b1c-7000-8000-0000000000c3';
const FILTERS: RunFilters = { status: 'MANUAL_REVIEW', workflowId: '', origin: '' };
const REASON = 'the provider is back, retry them';

const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
const job = { batchId: JOB, action: 'CANCEL', state: 'SELECTING', createdBy: 'alice', createdAt: '2026-11-04T10:00:00Z', counts: { selected: 0, succeeded: 0, skipped: 0, failed: 0, cancelled: 0, deferred: 0 } };
const namespaces = () => ({ items: [{ id: NS, workspaceId: NS, slug: 'payments', environment: 'dev', createdAt: '2026-11-04T10:00:00Z' }], nextCursor: null });

/** The button of the run list and, beside it, a way to change the filters the way the address does (the back button, a link). */
function Harness({ filters, changed }: { filters: RunFilters; changed?: RunFilters | undefined }) {
  const [current, setCurrent] = useState(filters);
  return (
    <>
      <BatchFromRuns filters={current} workflowName="checkout" />
      {changed ? <button onClick={() => setCurrent(changed)}>Change the filters</button> : null}
    </>
  );
}

async function mount(options: { permission?: boolean; matched?: number; routes?: Scripted; filters?: RunFilters; changed?: RunFilters } = {}) {
  const { permission = true, matched = 3, filters = FILTERS } = options;
  const platform = fakePlatform({
    'GET /namespaces': namespaces,
    [`POST /namespaces/${NS}/batch-operations/preview`]: () => ({ matched, capped: false, sample: matched > 0 ? [RUN] : [] }),
    [`POST /namespaces/${NS}/batch-operations`]: () => job,
    ...options.routes,
  });
  const app = createApp({ fetch: platform.fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions: permission ? ['run:read', 'run:batch'] : ['run:read'] } as never));
  app.namespace.set('payments');
  renderApp(
    <Routes>
      <Route path="/" element={<Harness filters={filters} changed={options.changed} />} />
      <Route path="/batches/:batchId" element={<h1>The job page</h1>} />
    </Routes>,
    { app },
  );
  return { app, platform, user: userEvent.setup() };
}

const posts = (platform: { calls: { method: string; url: string; init: RequestInit }[] }) => platform.calls.filter((call) => call.method === 'POST' && !call.url.endsWith('/preview'));
const bodyOf = (call: { init: RequestInit }) => JSON.parse(call.init.body as string) as Record<string, unknown>;
const keyOf = (call: { init: RequestInit }) => new Headers(call.init.headers).get('Idempotency-Key');

/** The announcer repeats what the dialog says; the dialog is what a person reads. */
const inDialog = () => within(screen.getByRole('dialog'));

async function open(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole('button', { name: 'Batch operation…' }));
  await screen.findByLabelText('Action');
}

describe('The batch operation of the run list', () => {
  it('is not offered without run:batch', async () => {
    await mount({ permission: false });
    expect(screen.queryByRole('button', { name: 'Batch operation…' })).not.toBeInTheDocument();
  });

  it('says what it will do to which runs, and asks nothing of the platform until told to', async () => {
    const { user, platform } = await mount();
    await open(user);
    expect(inDialog().getByText(/applies one action to the runs with status manual review/)).toBeVisible();
    expect(inDialog().getByText(/asks each run to stop/)).toBeVisible();
    expect(platform.calls.filter((call) => call.method === 'POST')).toHaveLength(0);
  });

  it('counts the runs, then starts the job with the selector of the filters and goes to its page', async () => {
    const { user, platform } = await mount();
    await open(user);
    await user.click(screen.getByRole('button', { name: 'Count the runs' }));
    expect(await inDialog().findByText(/3 runs match/)).toBeVisible();
    expect(screen.getByRole('link', { name: RUN.slice(-8) })).toHaveAttribute('href', `/runs/${RUN}`);
    await user.type(screen.getByLabelText('Reason'), REASON);
    await user.click(screen.getByRole('button', { name: 'Start the batch job' }));
    expect(await screen.findByRole('heading', { name: 'The job page' })).toBeVisible();
    const [created] = posts(platform);
    expect(bodyOf(created!)).toEqual({ selector: { filter: { status: ['MANUAL_REVIEW'] } }, action: 'CANCEL', reason: REASON });
  });

  it('will not start before the runs are counted, nor for a reason that is too short, nor when nothing matches', async () => {
    const { user, platform } = await mount();
    await open(user);
    await user.type(screen.getByLabelText('Reason'), REASON);
    await user.click(screen.getByRole('button', { name: 'Start the batch job' }));
    expect(inDialog().getByRole('alert')).toHaveTextContent('Count the runs first');
    await user.click(screen.getByRole('button', { name: 'Count the runs' }));
    await inDialog().findByText(/3 runs match/);
    await user.clear(screen.getByLabelText('Reason'));
    await user.type(screen.getByLabelText('Reason'), 'short');
    await user.click(screen.getByRole('button', { name: 'Start the batch job' }));
    expect(inDialog().getByRole('alert')).toHaveTextContent('at least 8');
    expect(posts(platform)).toHaveLength(0);
  });

  it('refuses to start a job that would take no run', async () => {
    const { user, platform } = await mount({ matched: 0 });
    await open(user);
    await user.click(screen.getByRole('button', { name: 'Count the runs' }));
    expect(await inDialog().findByText(/0 runs match/)).toBeVisible();
    await user.type(screen.getByLabelText('Reason'), REASON);
    await user.click(screen.getByRole('button', { name: 'Start the batch job' }));
    expect(inDialog().getByRole('alert')).toHaveTextContent('nothing to do');
    expect(posts(platform)).toHaveLength(0);
  });

  it('forgets the count when the limit changes: what was counted is what the job would take', async () => {
    const { user, platform } = await mount();
    await open(user);
    await user.click(screen.getByRole('button', { name: 'Count the runs' }));
    await inDialog().findByText(/3 runs match/);
    await user.type(screen.getByLabelText('Most runs'), '2');
    expect(inDialog().queryByText(/3 runs match/)).not.toBeInTheDocument();
    await user.type(screen.getByLabelText('Reason'), REASON);
    await user.click(screen.getByRole('button', { name: 'Start the batch job' }));
    expect(inDialog().getByRole('alert')).toHaveTextContent('Count the runs first');
    expect(posts(platform)).toHaveLength(0);
  });

  it('asks for a signal and its payload, and refuses a payload that is not a JSON object', async () => {
    const { user, platform } = await mount();
    await open(user);
    await user.selectOptions(screen.getByLabelText('Action'), 'SIGNAL');
    await user.click(screen.getByRole('button', { name: 'Count the runs' }));
    await inDialog().findByText(/3 runs match/);
    await user.type(screen.getByLabelText('Reason'), REASON);
    await user.type(screen.getByLabelText('Signal name'), 'order.approved');
    fireEvent.change(screen.getByLabelText('Payload (JSON object)'), { target: { value: '[1]' } });
    await user.click(screen.getByRole('button', { name: 'Start the batch job' }));
    expect(inDialog().getByRole('alert')).toHaveTextContent('JSON object');
    expect(posts(platform)).toHaveLength(0);
    fireEvent.change(screen.getByLabelText('Payload (JSON object)'), { target: { value: '{"approved":true}' } });
    await user.click(screen.getByRole('button', { name: 'Start the batch job' }));
    await screen.findByRole('heading', { name: 'The job page' });
    expect(bodyOf(posts(platform)[0]!)).toMatchObject({ action: 'SIGNAL', signal: { name: 'order.approved', payload: { approved: true } } });
  });

  it('names the step for the actions that need one', async () => {
    const { user } = await mount();
    await open(user);
    expect(screen.queryByLabelText('Step')).not.toBeInTheDocument();
    await user.selectOptions(screen.getByLabelText('Action'), 'RETRY_STEP');
    expect(screen.getByLabelText('Step')).toBeVisible();
    await user.selectOptions(screen.getByLabelText('Action'), 'PAUSE');
    expect(screen.queryByLabelText('Step')).not.toBeInTheDocument();
  });

  it('keeps the key of an identical retry and changes it when the request changes', async () => {
    let fail = true;
    const { user, platform } = await mount({ routes: { [`POST /namespaces/${NS}/batch-operations`]: () => (fail ? json(503, { code: 'UNAVAILABLE', message: 'Try again.', retryable: true }) : job) } });
    await open(user);
    await user.click(screen.getByRole('button', { name: 'Count the runs' }));
    await inDialog().findByText(/3 runs match/);
    await user.type(screen.getByLabelText('Reason'), REASON);
    await user.click(screen.getByRole('button', { name: 'Start the batch job' }));
    await inDialog().findByText(/Try again/);
    await user.click(screen.getByRole('button', { name: 'Start the batch job' }));
    await waitFor(() => expect(posts(platform)).toHaveLength(2));
    expect(keyOf(posts(platform)[0]!)).toBe(keyOf(posts(platform)[1]!));
    await user.type(screen.getByLabelText('Reason'), ' now');
    fail = false;
    await user.click(screen.getByRole('button', { name: 'Start the batch job' }));
    await waitFor(() => expect(posts(platform)).toHaveLength(3));
    expect(keyOf(posts(platform)[2]!)).not.toBe(keyOf(posts(platform)[0]!));
  });

  describe('with the business id prefix of the run list', () => {
    const WITH_PREFIX: RunFilters = { status: '', workflowId: '', origin: '', businessIdPrefix: 'order-' };
    const previews = (platform: { calls: { method: string; url: string; init: RequestInit }[] }) => platform.calls.filter((call) => call.method === 'POST' && call.url.endsWith('/preview'));

    it('says which runs it takes by the prefix, in words, and says nothing about it when there is none', async () => {
      const { user } = await mount({ filters: { ...WITH_PREFIX, status: 'FAILED' } });
      await open(user);
      expect(inDialog().getByText(/applies one action to the runs with status failed and business id starting with "order-", one run at a time/)).toBeVisible();
    });

    it('counts the runs with the prefix in the selector, then starts the job with the same selector', async () => {
      const { user, platform } = await mount({ filters: WITH_PREFIX });
      await open(user);
      await user.click(screen.getByRole('button', { name: 'Count the runs' }));
      expect(await inDialog().findByText(/3 runs match/)).toBeVisible();
      expect(previews(platform)).toHaveLength(1);
      expect(bodyOf(previews(platform)[0]!)).toEqual({ selector: { filter: { businessIdPrefix: 'order-' } } });
      await user.type(screen.getByLabelText('Reason'), REASON);
      await user.click(screen.getByRole('button', { name: 'Start the batch job' }));
      expect(await screen.findByRole('heading', { name: 'The job page' })).toBeVisible();
      expect(bodyOf(posts(platform)[0]!)).toEqual({ selector: { filter: { businessIdPrefix: 'order-' } }, action: 'CANCEL', reason: REASON });
    });

    it('sends it with the other filters, and nothing for a prefix that is empty', async () => {
      const { user, platform } = await mount({ filters: { status: 'FAILED', workflowId: '', origin: '', businessIdPrefix: '' } });
      await open(user);
      await user.click(screen.getByRole('button', { name: 'Count the runs' }));
      await inDialog().findByText(/3 runs match/);
      expect(bodyOf(previews(platform)[0]!)).toEqual({ selector: { filter: { status: ['FAILED'] } } });
      expect(inDialog().getByText(/applies one action to the runs with status failed, one run/)).toBeVisible();
    });

    it('says a prefix that matches nothing counts none, and starts nothing', async () => {
      const { user, platform } = await mount({ filters: WITH_PREFIX, matched: 0 });
      await open(user);
      await user.click(screen.getByRole('button', { name: 'Count the runs' }));
      expect(await inDialog().findByText(/0 runs match/)).toBeVisible();
      await user.type(screen.getByLabelText('Reason'), REASON);
      await user.click(screen.getByRole('button', { name: 'Start the batch job' }));
      expect(inDialog().getByRole('alert')).toHaveTextContent('nothing to do');
      expect(posts(platform)).toHaveLength(0);
    });

    it('forgets the count when the filters change under the dialog: what was counted is what the job would take', async () => {
      const { user, platform } = await mount({ filters: WITH_PREFIX, changed: { ...WITH_PREFIX, businessIdPrefix: 'refund-' } });
      await open(user);
      await user.click(screen.getByRole('button', { name: 'Count the runs' }));
      await inDialog().findByText(/3 runs match/);
      // the address changes by itself (the back button of the browser, say)
      fireEvent.click(screen.getByText('Change the filters'));
      expect(await inDialog().findByText(/business id starting with "refund-"/)).toBeVisible();
      expect(inDialog().queryByText(/3 runs match/)).not.toBeInTheDocument();
      await user.type(screen.getByLabelText('Reason'), REASON);
      await user.click(screen.getByRole('button', { name: 'Start the batch job' }));
      expect(inDialog().getByRole('alert')).toHaveTextContent('Count the runs first');
      expect(posts(platform)).toHaveLength(0);
      await user.click(screen.getByRole('button', { name: 'Count the runs' }));
      await inDialog().findByText(/3 runs match/);
      expect(bodyOf(previews(platform)[1]!)).toEqual({ selector: { filter: { businessIdPrefix: 'refund-' } } });
    });

    it('is customer text: shown as text, kept out of the address and of the logs', async () => {
      const logged = (['log', 'info', 'warn', 'error', 'debug'] as const).map((method) => vi.spyOn(console, method).mockImplementation(() => undefined));
      const risky = '<img/src=x/onerror=alert(1)>';
      const { user, platform } = await mount({ filters: { ...WITH_PREFIX, businessIdPrefix: risky } });
      await open(user);
      expect(screen.getByRole('dialog')).toHaveTextContent(`business id starting with "${risky}"`);
      expect(document.querySelector('img')).toBeNull();
      await user.click(screen.getByRole('button', { name: 'Count the runs' }));
      await inDialog().findByText(/3 runs match/);
      expect(platform.calls.every((call) => !call.url.includes('onerror'))).toBe(true);
      expect(logged.flatMap((spy) => spy.mock.calls).map((call) => JSON.stringify(call)).join('\n')).not.toContain('onerror');
      vi.restoreAllMocks();
    });

    it('says in words that the platform does not support a selector, whatever the reason it names', async () => {
      const { user } = await mount({
        filters: WITH_PREFIX,
        routes: { [`POST /namespaces/${NS}/batch-operations/preview`]: () => json(422, { code: 'VALIDATION_FAILED', message: 'refused', retryable: false, details: { reason: 'BATCH_SELECTOR_UNSUPPORTED' } }) },
      });
      await open(user);
      await user.click(screen.getByRole('button', { name: 'Count the runs' }));
      expect(await inDialog().findByText(/This selector is not supported/)).toBeVisible();
    });
  });

  it('says so when the deployment does not serve batch operations', async () => {
    const { user } = await mount({
      routes: { [`POST /namespaces/${NS}/batch-operations/preview`]: () => json(404, { code: 'NOT_FOUND', message: 'resource not found', retryable: false, details: { reason: 'ROUTE_NOT_FOUND' } }) },
    });
    await open(user);
    await user.click(screen.getByRole('button', { name: 'Count the runs' }));
    expect(await inDialog().findByText(/not enabled on this deployment/)).toBeVisible();
  });
});
