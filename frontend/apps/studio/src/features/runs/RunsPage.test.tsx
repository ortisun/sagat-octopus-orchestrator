import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { StrictMode } from 'react';
import type { ReactNode } from 'react';
import { Route, Routes, useLocation, useNavigate } from 'react-router';
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { fakePlatform, renderApp } from '../../../test/render';
import type { Routes as FakeRoutes } from '../../../test/render';
import { installRunsDom } from '../../../test/runsDom';
import { OPERATOR, VERSION_ID, WORKFLOW_ID, at, failure, runOf, signedInApp } from '../../../test/runs';
import type { Run } from '../../api/types';
import { RUN_STATUSES } from './display';
import { RunsPage, parseRunId, readFilters } from './RunsPage';

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

/** Says where the page is, so a test can read the address the filters were written to. */
function Where() {
  const { pathname, search } = useLocation();
  return <output data-testid="where">{`${pathname}${search}`}</output>;
}

/** The back button of the browser, for a test that needs the address to change by itself. */
function Back() {
  const navigate = useNavigate();
  return <button onClick={() => void navigate(-1)}>Go back</button>;
}

async function open(runs: Run[], options: { size?: number; routes?: FakeRoutes; permissions?: string[]; namespace?: string | null; strict?: boolean; path?: string; beside?: ReactNode } = {}) {
  const platform = runsPlatform(runs, options);
  const app = await signedInApp(platform.fetcher, options.permissions ?? OPERATOR);
  if (options.namespace === null) app.namespace.set(undefined);
  const routes = (
    <Routes>
      <Route
        path="/runs"
        element={
          <>
            <RunsPage />
            <Where />
            {options.beside}
          </>
        }
      />
      <Route path="/runs/:runId" element={<Where />} />
    </Routes>
  );
  const view = renderApp(options.strict ? <StrictMode>{routes}</StrictMode> : routes, { app, path: options.path ?? '/runs' });
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
    expect(within(rows()[0] as HTMLElement).getByText(`workflow ${WORKFLOW_ID.slice(-8)}`)).toBeInTheDocument();
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

describe('the runs page: filters in the address', () => {
  const where = () => screen.getByTestId('where').textContent;

  it('takes only filters the page could have written: a status it knows, a workflow id that is one', () => {
    expect(readFilters(new URLSearchParams(`status=FAILED&workflow=${OTHER_WORKFLOW}&business=order-`))).toEqual({ status: 'FAILED', workflowId: OTHER_WORKFLOW, business: 'order-' });
    expect(readFilters(new URLSearchParams('status=DROP%20TABLE&workflow=../../etc&business=has%20space'))).toEqual({ status: '', workflowId: '', business: '' });
    expect(readFilters(new URLSearchParams(''))).toEqual({ status: '', workflowId: '', business: '' });
  });

  it('opens with the filters of the link it was reached by, and asks the platform for them', async () => {
    const { platform } = await open([run(1), run(2, { status: 'FAILED' }), run(3, { status: 'FAILED', workflowId: OTHER_WORKFLOW })], { path: `/runs?status=FAILED&workflow=${OTHER_WORKFLOW}` });
    await screen.findByRole('table', { name: 'Runs, newest first' });
    expect(runIds()).toEqual([run(3).runId]);
    expect(platform.requests.at(-1)?.get('status')).toBe('FAILED');
    expect(platform.requests.at(-1)?.get('workflowId')).toBe(OTHER_WORKFLOW);
    expect(screen.getByRole('combobox', { name: 'Status' })).toHaveValue('FAILED');
  });

  it('writes a filter to the address when it is chosen, so the back button and a shared link have it, and clears them together', async () => {
    const { user } = await open([run(1), run(2, { status: 'FAILED' })]);
    await screen.findByRole('table', { name: 'Runs, newest first' });
    expect(screen.queryByRole('button', { name: 'Clear filters' })).not.toBeInTheDocument();
    await user.selectOptions(screen.getByRole('combobox', { name: 'Status' }), 'FAILED');
    expect(where()).toBe('/runs?status=FAILED');
    await user.click(screen.getByRole('button', { name: 'Clear filters' }));
    expect(where()).toBe('/runs');
    await waitFor(() => expect(rows()).toHaveLength(2));
  });

  it('clears the origin alone and together with address filters', async () => {
    const { platform, user } = await open([run(1), run(2, { status: 'FAILED' })]);
    await screen.findByRole('table', { name: 'Runs, newest first' });
    const origin = screen.getByRole('combobox', { name: 'Origin' });
    await user.selectOptions(origin, 'CONNECTION_PROBE');
    await waitFor(() => expect(platform.requests.at(-1)?.get('origin')).toBe('CONNECTION_PROBE'));
    await user.click(screen.getByRole('button', { name: 'Clear filters' }));
    expect(origin).toHaveValue('');
    await waitFor(() => expect(platform.requests.at(-1)?.has('origin')).toBe(false));
    await user.selectOptions(origin, 'CONNECTION_PROBE');
    await user.selectOptions(screen.getByRole('combobox', { name: 'Status' }), 'FAILED');
    await user.selectOptions(screen.getByRole('combobox', { name: 'Workflow' }), WORKFLOW_ID);
    await user.click(screen.getByRole('button', { name: 'Clear filters' }));
    expect(where()).toBe('/runs');
    expect(origin).toHaveValue('');
    await waitFor(() => {
      const query = platform.requests.at(-1);
      expect(query?.has('origin')).toBe(false);
      expect(query?.has('status')).toBe(false);
      expect(query?.has('workflowId')).toBe(false);
    });
  });

  it('keeps the filters on the link to a run, so the list beside it stays the one that was chosen', async () => {
    await open([run(2, { status: 'FAILED' })], { path: '/runs?status=FAILED' });
    await screen.findByRole('table', { name: 'Runs, newest first' });
    expect(within(rows()[0] as HTMLElement).getByRole('link')).toHaveAttribute('href', `/runs/${run(2).runId}?status=FAILED`);
  });

  it('opens a run from its id however it was pasted, and says so when the text is not an id', async () => {
    expect(parseRunId(`  ${run(5).runId.toUpperCase()}\n`)).toBe(run(5).runId);
    expect(parseRunId(`https://studio.example/studio/runs/${run(5).runId}`)).toBe(run(5).runId);
    expect(parseRunId('not-a-run')).toBeUndefined();
    const { user } = await open([run(1)]);
    await screen.findByRole('table', { name: 'Runs, newest first' });
    const field = screen.getByRole('searchbox', { name: 'Open a run by id' });
    await user.type(field, 'not-a-run{Enter}');
    expect(await screen.findByRole('alert')).toHaveTextContent(/not a run id/);
    await user.clear(field);
    await user.type(field, `${run(1).runId}{Enter}`);
    await waitFor(() => expect(where()).toBe(`/runs/${run(1).runId}`));
  });
});

describe('the runs page: business identifiers', () => {
  const where = () => screen.getByTestId('where').textContent;
  const field = () => screen.getByRole('searchbox', { name: 'Business id starts with' });
  const asked = (platform: { requests: URLSearchParams[] }) => platform.requests.filter((query) => query.has('businessIdPrefix')).map((query) => query.get('businessIdPrefix'));
  const rest = (ms: number) =>
    act(async () => {
      await vi.advanceTimersByTimeAsync(ms);
    });

  it('shows the identifier of a run under its id, and nothing under a run without one', async () => {
    await open([run(1, { businessId: 'order-1001' }), run(2)]);
    await screen.findByRole('table', { name: 'Runs, newest first' });
    const [without, withId] = rows() as [HTMLElement, HTMLElement];
    expect(within(withId).getByText('order-1001')).toBeInTheDocument();
    expect(withId).toHaveTextContent('Business id order-1001');
    expect(without).not.toHaveTextContent('Business id');
  });

  it('asks for the start of an identifier once typing rests, not at every key, and keeps it in the address', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { platform } = await open([run(1), run(2)]);
    await screen.findByRole('table', { name: 'Runs, newest first' });
    fireEvent.change(field(), { target: { value: 'o' } });
    await rest(100);
    fireEvent.change(field(), { target: { value: 'order-' } });
    await rest(100);
    expect(asked(platform)).toEqual([]);
    expect(where()).toBe('/runs');
    await rest(250);
    await waitFor(() => expect(asked(platform)).toEqual(['order-']));
    expect(where()).toBe('/runs?business=order-');
  });

  it('opens with the identifier of the link, shows it in the field and asks the platform for it', async () => {
    const { platform } = await open([run(1)], { path: '/runs?business=order-' });
    await screen.findByRole('table', { name: 'Runs, newest first' });
    expect(field()).toHaveValue('order-');
    expect(platform.requests.at(-1)?.get('businessIdPrefix')).toBe('order-');
    expect(screen.getByRole('button', { name: 'Clear filters' })).toBeInTheDocument();
  });

  it('takes no identifier from a link that could not have written it', async () => {
    const { platform } = await open([run(1)], { path: '/runs?business=has%20space' });
    await screen.findByRole('table', { name: 'Runs, newest first' });
    expect(field()).toHaveValue('');
    expect(asked(platform)).toEqual([]);
    expect(screen.queryByRole('button', { name: 'Clear filters' })).not.toBeInTheDocument();
  });

  it('carries the prefix of the filter into the batch operation, which counts the runs with it and says so', async () => {
    const NS = '019b5d6e-0000-7000-8000-0000000000a1';
    const { platform, user } = await open([run(1, { status: 'FAILED' })], {
      path: '/runs?business=order-&status=FAILED',
      permissions: [...OPERATOR, 'run:batch'],
      routes: {
        'GET /namespaces': () => ({ items: [{ id: NS, workspaceId: NS, slug: 'dev', environment: 'dev', createdAt: '2026-11-04T10:00:00Z' }], nextCursor: null }),
        [`POST /namespaces/${NS}/batch-operations/preview`]: () => ({ matched: 2, capped: false, sample: [] }),
      },
    });
    await screen.findByRole('table', { name: 'Runs, newest first' });
    await user.click(await screen.findByRole('button', { name: 'Batch operation…' }));
    const dialog = within(await screen.findByRole('dialog'));
    expect(await dialog.findByText(/applies one action to the runs with status failed and business id starting with "order-"/)).toBeVisible();
    await user.click(dialog.getByRole('button', { name: 'Count the runs' }));
    expect(await dialog.findByText(/2 runs match/)).toBeVisible();
    const preview = platform.calls.find((call) => call.method === 'POST' && call.url.endsWith('/preview'));
    expect(JSON.parse(preview!.init.body as string)).toEqual({ selector: { filter: { status: ['FAILED'], businessIdPrefix: 'order-' } } });
  });

  it('clears the identifier with the other filters', async () => {
    const { platform, user } = await open([run(1, { status: 'FAILED' })], { path: '/runs?business=order-&status=FAILED' });
    await screen.findByRole('table', { name: 'Runs, newest first' });
    await user.click(screen.getByRole('button', { name: 'Clear filters' }));
    expect(where()).toBe('/runs');
    expect(field()).toHaveValue('');
    await waitFor(() => expect(platform.requests.at(-1)?.has('businessIdPrefix')).toBe(false));
  });

  it('follows the address when it changes by itself, and does not write the text it had back into it', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { platform, user } = await open([run(1)], { beside: <Back /> });
    await screen.findByRole('table', { name: 'Runs, newest first' });
    fireEvent.change(field(), { target: { value: 'order-' } });
    await rest(400);
    await waitFor(() => expect(where()).toBe('/runs?business=order-'));
    const before = platform.requests.length;
    await user.click(screen.getByRole('button', { name: 'Go back' }));
    expect(where()).toBe('/runs');
    expect(field()).toHaveValue('');
    await rest(1000);
    expect(where()).toBe('/runs');
    expect(field()).toHaveValue('');
    expect(platform.requests.slice(before).some((query) => query.has('businessIdPrefix'))).toBe(false);
  });
});

describe('the runs page: narrow windows', () => {
  it('shows the runs as cards where a table of six columns would not fit, and as a table where it does', async () => {
    vi.stubGlobal('matchMedia', (query: string) => ({ matches: query.includes('max-width'), media: query, addEventListener: () => undefined, removeEventListener: () => undefined }));
    try {
      await open([run(1, { status: 'FAILED' })]);
      const cards = await screen.findByRole('list', { name: 'Runs, newest first' });
      expect(screen.queryByRole('table')).not.toBeInTheDocument();
      expect(within(cards).getByText('Failed')).toBeInTheDocument();
      expect(within(cards).getByRole('link')).toHaveAttribute('href', `/runs/${run(1).runId}`);
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

describe('the runs page: delayed starts', () => {
  it('says under a waiting run when its entry is released, and says nothing under the others', async () => {
    await open([
      run(1, { status: 'WAITING', finishedAt: null, startDelayedUntil: '2026-10-12T09:00:00Z' }),
      run(2, { status: 'WAITING', finishedAt: null }),
    ]);
    await screen.findByRole('table', { name: 'Runs, newest first' });
    const [plain, delayed] = rows() as [HTMLElement, HTMLElement];
    expect(delayed).toHaveTextContent(/Starts .*2026/);
    expect(plain).not.toHaveTextContent(/Starts /);
  });
});
