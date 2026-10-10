import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { StrictMode } from 'react';
import { Route, Routes } from 'react-router';
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { fakePlatform, renderApp } from '../../../test/render';
import type { Routes as FakeRoutes } from '../../../test/render';
import { installRunsDom } from '../../../test/runsDom';
import { OPERATOR, VERSION_ID, WORKFLOW_ID, at, failure, runOf, signedInApp } from '../../../test/runs';
import type { Run } from '../../api/types';
import { RUN_STATUSES } from './display';
import { RunsPage } from './RunsPage';

let undo: () => void;
beforeAll(() => {
  undo = installRunsDom();
});
afterAll(() => undo());
afterEach(() => {
  vi.useRealTimers();
});

const OTHER_WORKFLOW = '019b5d6e-0000-7000-8000-0000000000ab';
const run = (n: number, over: Partial<Run> = {}): Run => runOf({ runId: `019b5d6e-0000-7000-8000-${String(n).padStart(12, '0')}`, createdAt: at(n), version: n, status: 'SUCCEEDED', finishedAt: at(n + 4), ...over });

/** A platform with these runs (served newest first, `size` at a time) and two workflows. */
function runsPlatform(initial: Run[], options: { size?: number; routes?: FakeRoutes } = {}) {
  const state = { runs: initial };
  const size = options.size ?? 50;
  const requests: URLSearchParams[] = [];
  const headers: Record<string, string>[] = [];
  const routes: FakeRoutes = {
    'GET /runs': (init, url) => {
      const query = new URL(url, 'http://studio.test').searchParams;
      requests.push(query);
      headers.push(init.headers as Record<string, string>);
      let list = [...state.runs].sort((a, b) => (a.createdAt < b.createdAt ? 1 : -1));
      const status = query.get('status');
      const workflow = query.get('workflowId');
      if (status) list = list.filter((r) => r.status === status);
      if (workflow) list = list.filter((r) => r.workflowId === workflow);
      const from = Number(query.get('cursor') ?? 0);
      return { items: list.slice(from, from + size), nextCursor: from + size < list.length ? String(from + size) : null };
    },
    'GET /workflows': () => ({
      items: [
        { id: WORKFLOW_ID, name: 'order-saga', draftRevision: 1, ir: {}, activeVersion: { workflowId: WORKFLOW_ID, versionId: VERSION_ID, versionNumber: 3, digest: 'sha256:a' } },
        { id: OTHER_WORKFLOW, name: 'refund', draftRevision: 1, ir: {}, activeVersion: null },
      ],
      nextCursor: null,
    }),
    [`GET /workflows/${WORKFLOW_ID}/versions`]: () => ({ items: [{ workflowId: WORKFLOW_ID, versionId: VERSION_ID, versionNumber: 3, digest: 'sha256:a' }], nextCursor: null }),
    [`GET /workflows/${OTHER_WORKFLOW}/versions`]: () => ({ items: [], nextCursor: null }),
    ...options.routes,
  };
  const platform = fakePlatform(routes);
  return { ...platform, state, requests, headers };
}

async function open(runs: Run[], options: { size?: number; routes?: FakeRoutes; permissions?: string[]; namespace?: string | null; strict?: boolean } = {}) {
  const platform = runsPlatform(runs, options);
  const app = await signedInApp(platform.fetcher, options.permissions ?? OPERATOR);
  if (options.namespace === null) app.namespace.set(undefined);
  const routes = (
    <Routes>
      <Route path="/runs" element={<RunsPage />} />
    </Routes>
  );
  const view = renderApp(options.strict ? <StrictMode>{routes}</StrictMode> : routes, { app, path: '/runs' });
  return { platform, ...view, user: userEvent.setup() };
}

const rows = () => within(screen.getByRole('table', { name: 'Runs, newest first' })).getAllByRole('row').slice(1);
const runIds = () => rows().map((row) => within(row).getAllByRole('link')[0]?.textContent);

describe('the runs page', () => {
  it('lists the runs of the namespace newest first, with workflow and version, status, times and who started them', async () => {
    const { platform } = await open([
      run(1),
      run(3, { status: 'RUNNING', finishedAt: null, startedBy: 'user:bob' }),
      run(2, { status: 'FAILED', workflowId: OTHER_WORKFLOW, startedBy: 'service:billing' }),
    ]);
    await screen.findByRole('table', { name: 'Runs, newest first' });
    expect(runIds()).toEqual([run(3).runId, run(2).runId, run(1).runId]);
    const [newest, middle, oldest] = rows() as [HTMLElement, HTMLElement, HTMLElement];
    expect(within(newest).getByRole('link')).toHaveAttribute('href', `/runs/${run(3).runId}`);
    expect(within(newest).getByText('Running')).toBeInTheDocument();
    expect(newest).toHaveTextContent(/running for/);
    expect(within(newest).getByText('user:bob')).toBeInTheDocument();
    expect(within(middle).getByText('Failed')).toBeInTheDocument();
    expect(within(middle).getByText('refund')).toBeInTheDocument();
    expect(within(middle).getByText('service:billing')).toBeInTheDocument();
    // Name and version of the workflow, from the workflows and their versions.
    expect(await within(oldest).findByText('order-saga')).toBeInTheDocument();
    expect(await within(oldest).findByText('v3')).toBeInTheDocument();
    expect(oldest).toHaveTextContent(/took 4\.0 s/);
    expect(within(oldest).getByText('Succeeded')).toBeInTheDocument();
    // The namespace the user works in is what the platform is asked about.
    expect(platform.headers[0]?.['X-Octopus-Namespace']).toBe('dev');
    expect(platform.requests[0]?.get('limit')).toBe('50');
  });

  it('marks the runs that need a person, and only those', async () => {
    await open([run(1, { status: 'MANUAL_REVIEW', finishedAt: null }), run(2, { status: 'PAUSED', finishedAt: null }), run(3)]);
    await screen.findByRole('table', { name: 'Runs, newest first' });
    const [, paused, review] = rows() as [HTMLElement, HTMLElement, HTMLElement];
    expect(within(review).getByText(/A person must decide/)).toBeInTheDocument();
    expect(within(review).getByText('Needs a person')).toBeInTheDocument();
    expect(within(paused).queryByText(/A person must decide/)).not.toBeInTheDocument();
    expect(screen.getAllByText(/A person must decide/)).toHaveLength(1);
  });

  it('can be filtered by status, with every status the platform has to choose from', async () => {
    const { platform, user } = await open([run(1), run(2, { status: 'FAILED' }), run(3, { status: 'FAILED' })]);
    await screen.findByRole('table', { name: 'Runs, newest first' });
    const select = screen.getByRole('combobox', { name: 'Status' });
    const options = within(select).getAllByRole('option').map((o) => o.getAttribute('value'));
    expect(options).toEqual(['', ...RUN_STATUSES]);
    expect(RUN_STATUSES).toHaveLength(13);
    await user.selectOptions(select, 'FAILED');
    await waitFor(() => expect(runIds()).toEqual([run(3).runId, run(2).runId]));
    expect(platform.requests.at(-1)?.get('status')).toBe('FAILED');
    await user.selectOptions(select, '');
    await waitFor(() => expect(rows()).toHaveLength(3));
    expect(platform.requests.at(-1)?.has('status')).toBe(false);
  });

  it('can be filtered by workflow, with the workflows of the namespace to choose from', async () => {
    const { platform, user } = await open([run(1), run(2, { workflowId: OTHER_WORKFLOW })]);
    await screen.findByRole('table', { name: 'Runs, newest first' });
    const select = await screen.findByRole('combobox', { name: 'Workflow' });
    await waitFor(() => expect(select).toBeEnabled());
    expect(within(select).getAllByRole('option').map((o) => o.textContent)).toEqual(['All workflows', 'order-saga', 'refund']);
    await user.selectOptions(select, 'refund');
    await waitFor(() => expect(runIds()).toEqual([run(2).runId]));
    expect(platform.requests.at(-1)?.get('workflowId')).toBe(OTHER_WORKFLOW);
  });

  it('loads older runs on request with the cursor the platform gave, until there are no more', async () => {
    const all = Array.from({ length: 120 }, (_, i) => run(i + 1));
    const { platform, user } = await open(all);
    await screen.findByRole('table', { name: 'Runs, newest first' });
    expect(screen.getByRole('table', { name: 'Runs, newest first' })).toHaveAttribute('aria-rowcount', '51');
    await user.click(screen.getByRole('button', { name: 'Load more' }));
    await waitFor(() => expect(screen.getByRole('table', { name: 'Runs, newest first' })).toHaveAttribute('aria-rowcount', '101'));
    expect(platform.requests.at(-1)?.get('cursor')).toBe('50');
    await user.click(screen.getByRole('button', { name: 'Load more' }));
    await waitFor(() => expect(screen.getByRole('table', { name: 'Runs, newest first' })).toHaveAttribute('aria-rowcount', '121'));
    expect(screen.queryByRole('button', { name: 'Load more' })).not.toBeInTheDocument();
    expect(screen.getByText('120 runs loaded.')).toBeInTheDocument();
    expect(await screen.findByText(/20 more runs loaded, 120 in all; that is all of them/)).toBeInTheDocument();
  });

  it('draws a window of thousands of runs, not all of them', async () => {
    const all = Array.from({ length: 3000 }, (_, i) => run(i + 1));
    await open(all, { size: 3000 });
    const table = await screen.findByRole('table', { name: 'Runs, newest first' });
    expect(table).toHaveAttribute('aria-rowcount', '3001');
    expect(rows().length).toBeLessThan(40);
    expect(runIds()[0]).toBe(run(3000).runId);
  });

  it('reads the list again when asked, and says so', async () => {
    const { platform, user } = await open([run(1)]);
    await screen.findByRole('table', { name: 'Runs, newest first' });
    platform.state.runs = [run(1), run(2)];
    await user.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(runIds()).toEqual([run(2).runId, run(1).runId]));
    expect(await screen.findByText(/2 runs\./)).toBeInTheDocument();
  });

  it('says there are no runs, and that none match the filters', async () => {
    const { user } = await open([]);
    expect(await screen.findByText('No run has been started in dev yet.')).toBeInTheDocument();
    await user.selectOptions(screen.getByRole('combobox', { name: 'Status' }), 'FAILED');
    expect(await screen.findByText('No run matches these filters.')).toBeInTheDocument();
  });

  it('shows the failure of the platform with its message and trace id, and a way to try again', async () => {
    let broken = true;
    const { user } = await open([run(1)], {
      routes: {
        'GET /runs': () => (broken ? failure(503, 'UNAVAILABLE', 'The platform is not available right now.') : { items: [run(1)], nextCursor: null }),
      },
    });
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('Loading the runs failed.');
    expect(alert).toHaveTextContent('The platform is not available right now.');
    expect(alert).toHaveTextContent('b'.repeat(32));
    broken = false;
    await user.click(within(alert).getByRole('button', { name: 'Try again' }));
    expect(await screen.findByRole('table', { name: 'Runs, newest first' })).toBeInTheDocument();
  });

  it('says a person who may not read runs is not allowed, and asks for no run', async () => {
    const { platform } = await open([run(1)], { permissions: ['definition:read'] });
    expect(await screen.findByText(/You are not allowed to see runs/)).toBeInTheDocument();
    expect(platform.calls.filter((c) => c.url.startsWith('/v1/runs'))).toHaveLength(0);
  });

  it('waits for a namespace instead of asking without one', async () => {
    const { platform } = await open([run(1)], { namespace: null });
    expect(await screen.findByText('Waiting for a namespace…')).toBeInTheDocument();
    expect(platform.calls.filter((c) => c.url.startsWith('/v1/runs'))).toHaveLength(0);
  });

  it('shows the ids when the workflows cannot be listed, instead of failing', async () => {
    await open([run(1)], { routes: { 'GET /workflows': () => failure(403, 'FORBIDDEN', 'no') } });
    await screen.findByRole('table', { name: 'Runs, newest first' });
    expect(within(rows()[0] as HTMLElement).getByText(`workflow ${WORKFLOW_ID.slice(0, 8)}`)).toBeInTheDocument();
    expect(await screen.findByText(/The workflows could not be listed/)).toBeInTheDocument();
  });
});

describe('the runs page: keeping up to date', () => {
  it('works when React runs every effect twice, as it does in development, and still asks every five seconds once', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { platform } = await open([run(1)], { strict: true });
    await screen.findByRole('table', { name: 'Runs, newest first' });
    expect(rows()).toHaveLength(1);
    const before = platform.requests.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5100);
    });
    await waitFor(() => expect(platform.requests.length).toBe(before + 1));
    expect(rows()).toHaveLength(1);
  });

  it('reads the newest runs again every five seconds while the tab is visible', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { platform } = await open([run(1, { status: 'RUNNING', finishedAt: null })]);
    await screen.findByRole('table', { name: 'Runs, newest first' });
    const reads = () => platform.requests.length;
    const before = reads();
    platform.state.runs = [run(1, { status: 'SUCCEEDED' }), run(2, { status: 'RUNNING', finishedAt: null })];
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5100);
    });
    await waitFor(() => expect(runIds()).toEqual([run(2).runId, run(1).runId]));
    expect(reads()).toBe(before + 1);
    expect(platform.requests.at(-1)?.has('cursor')).toBe(false);
    expect(within(rows()[1] as HTMLElement).getByText('Succeeded')).toBeInTheDocument();
  });

  it('asks nothing while the tab is hidden, and reads again as soon as it is visible', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { platform } = await open([run(1)]);
    await screen.findByRole('table', { name: 'Runs, newest first' });
    const before = platform.requests.length;
    const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(platform.requests.length).toBe(before);
    visibility.mockReturnValue('visible');
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
      await vi.advanceTimersByTimeAsync(10);
    });
    await waitFor(() => expect(platform.requests.length).toBe(before + 1));
    visibility.mockRestore();
  });

  it('stops asking when the page is left', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { platform, unmount } = await open([run(1)]);
    await screen.findByRole('table', { name: 'Runs, newest first' });
    unmount();
    const before = platform.requests.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(platform.requests.length).toBe(before);
  });
});
