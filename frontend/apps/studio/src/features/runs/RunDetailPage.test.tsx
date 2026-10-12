import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Route, Routes } from 'react-router';
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { renderApp } from '../../../test/render';
import { openRunPage, openStream } from '../../../test/runPage';
import { installRunsDom } from '../../../test/runsDom';
import { ADMIN, OPERATOR, RUN_ID, TRACE, VIEWER, at, changesOf, failure, graphEvent, graphOf, journalEvent, node, runOf, runPlatform, sagaRun, signedInApp } from '../../../test/runs';
import type { Pipe, RunPlatformInit } from '../../../test/runs';
import { RunDetailPage } from './RunDetailPage';

let undo: () => void;
beforeAll(() => {
  undo = installRunsDom();
});
afterAll(() => undo());
afterEach(() => vi.unstubAllGlobals());

const open = openRunPage;
const stream = openStream;

const card = (id: string) => screen.getByTestId(`rf__node-${id}`);
/** A click on the canvas. (d3, which React Flow pans with, cannot take the mouse-down of a simulated pointer in jsdom.) */
const pick = (element: HTMLElement) => fireEvent.click(element);
const follow = () => screen.getByTestId('follow-state');

/** A journal of the saga run: `pay` was tried twice, `reserve` once (and its worker sent no trace). */
const JOURNAL = [
  journalEvent(1, 'run.started', null, { actor: 'user:alice' }),
  journalEvent(2, 'step.leased', 'reserve', { traceId: null }),
  journalEvent(3, 'step.completed', 'reserve', { traceId: null, payloadRef: 'f'.repeat(64) }),
  journalEvent(4, 'step.leased', 'pay', { attempt: 1 }),
  journalEvent(5, 'step.started', 'pay', { attempt: 1 }),
  journalEvent(7, 'step.retry_scheduled', 'pay', { attempt: 1 }),
  journalEvent(10, 'step.leased', 'pay', { attempt: 2 }),
  journalEvent(11, 'step.started', 'pay', { attempt: 2 }),
  journalEvent(14, 'step.failed', 'pay', { attempt: 2 }),
  journalEvent(15, 'step.needs_review', 'reserve~c', { attempt: 2 }),
  journalEvent(16, 'step.retry_requested', 'reserve~c', { attempt: null, actor: 'user:bob' }),
];

describe('the run page: header', () => {
  it('keeps technical metadata available in a disclosure without crowding the graph', async () => {
    const user = userEvent.setup();
    await open();
    const summary = await screen.findByText('Run details');
    const details = summary.closest('details');
    expect(details).not.toHaveAttribute('open');
    expect(screen.getByRole('button', { name: 'Copy run id' })).toBeVisible();
    expect(screen.getByRole('button', { name: 'Copy trace id' })).not.toBeVisible();
    await user.click(summary);
    expect(details).toHaveAttribute('open');
    expect(screen.getByRole('button', { name: 'Copy trace id' })).toBeVisible();
    expect(screen.getByText(/acme \/ dev/)).toBeVisible();
  });

  it('says what the run is, who started it and where it stands, with the meaning of the status in words', async () => {
    await open({ run: runOf({ status: 'MANUAL_REVIEW', deadline: '2099-01-01T00:00:00Z' }) });
    expect(await screen.findByRole('heading', { level: 1, name: new RegExp(RUN_ID) })).toBeInTheDocument();
    expect(await screen.findByRole('link', { name: 'order-saga' })).toHaveAttribute('href', '/workflows/019b5d6e-0000-7000-8000-0000000000aa');
    expect(await screen.findByText(/, v3/)).toBeInTheDocument();
    expect(screen.getAllByText('Needs a person').length).toBeGreaterThan(0);
    expect(screen.getByText('A decision is needed before it can go on.')).toBeInTheDocument();
    expect(screen.getByText('user:alice')).toBeInTheDocument();
    expect(screen.getByText('order-1234')).toBeInTheDocument();
    expect(screen.getByText(/acme \/ dev/)).toBeInTheDocument();
    expect(screen.getAllByText(TRACE).length).toBeGreaterThan(0);
    expect(screen.getByRole('button', { name: 'Copy run id' })).toBeInTheDocument();
    // The deadline is shown with its absolute time on hover.
    expect(document.querySelector('time[title="2099-01-01 00:00:00 UTC"]')).not.toBeNull();
  });

  it('says "no trace" and "none" for what the run does not have', async () => {
    await open({ run: runOf({ traceId: null, correlationId: null, deadline: null, startedBy: undefined }) });
    expect(await screen.findByText('no trace')).toBeInTheDocument();
    expect(screen.getAllByText('none').length).toBe(2);
    expect(screen.getByText('unknown')).toBeInTheDocument();
  });

  it('shows a finished run as finished, with how long it took, and does not follow it', async () => {
    const { nodes, edges } = sagaRun();
    const { platform } = await open({ run: runOf({ status: 'SUCCEEDED', finishedAt: at(75) }), graph: graphOf(nodes, edges, { status: 'SUCCEEDED' }) });
    expect((await screen.findByText('Finished')).nextElementSibling).toHaveTextContent(/took 1 min 15 s/);
    expect(follow()).toHaveTextContent('Ended');
    expect(platform.state.streams).toBe(0);
  });

  it('links a child run to the run that started it', async () => {
    await open({ run: runOf({ parent: { runId: '019b5d6e-0000-7000-8000-0000000000ee', node: 'charge', cancelPolicy: 'PROPAGATE' } }) });
    expect(await screen.findByRole('link', { name: /run 019b5d6e/ })).toHaveAttribute('href', '/runs/019b5d6e-0000-7000-8000-0000000000ee');
  });

  it('tells the person who may not read runs so, and asks the platform for nothing', async () => {
    const platform = runPlatform();
    const app = await signedInApp(platform.fetcher, ['definition:read']);
    renderApp(
      <Routes>
        <Route path="/runs/:runId" element={<RunDetailPage />} />
      </Routes>,
      { app, path: `/runs/${RUN_ID}` },
    );
    expect(await screen.findByText(/You are not allowed to see runs/)).toBeInTheDocument();
    expect(screen.getByText(/run:read/)).toBeInTheDocument();
    expect(platform.calls).toHaveLength(0);
  });

  it('shows a run that does not exist (or is not yours) with the message and the trace id', async () => {
    const platform = runPlatform({ routes: { [`GET /runs/${RUN_ID}`]: () => failure(404, 'NOT_FOUND', 'It does not exist, or it is not yours to see.') } });
    const app = await signedInApp(platform.fetcher, OPERATOR);
    renderApp(
      <Routes>
        <Route path="/runs/:runId" element={<RunDetailPage />} />
      </Routes>,
      { app, path: `/runs/${RUN_ID}` },
    );
    expect(await screen.findByRole('alert')).toHaveTextContent('It does not exist, or it is not yours to see.');
    expect(screen.getByText('b'.repeat(32))).toBeInTheDocument();
  });
});

describe('the run page: the graph', () => {
  it('draws every step with its state in words and shapes, and undo steps for what they are', async () => {
    await open();
    await waitFor(() => card('pay'));
    const pay = card('pay');
    expect(within(pay).getByText('Failed')).toBeInTheDocument();
    expect(within(pay).getByText('attempt 3')).toBeInTheDocument();
    expect(within(pay).getByText('failure HTTP_500')).toBeInTheDocument();
    expect(within(pay).getByText('Step')).toBeInTheDocument();
    // An undo the platform added is labelled as one, dashed, and says it waits for a person.
    const undoCard = card('reserve~c');
    expect(within(undoCard).getByText('Undo of reserve')).toBeInTheDocument();
    expect(undoCard.querySelector('.rn-card')).toHaveAttribute('data-variant', 'undo');
    expect(within(undoCard).getByText('Waiting for a person')).toBeInTheDocument();
    expect(within(card('pay~r')).getByText('Lookup for pay')).toBeInTheDocument();
    expect(card('pay~r').querySelector('.rn-card')).toHaveAttribute('data-variant', 'lookup');
    // A skipped branch, a saga, a timer with its due time as relative text and the absolute time in the tooltip.
    expect(within(card('shipping')).getByText('Skipped')).toBeInTheDocument();
    expect(within(card('order')).getByText(/saga going forward/)).toBeInTheDocument();
    const due = card('wait').querySelector('time');
    expect(due).toHaveAttribute('title', '2099-01-01 00:00:00 UTC');
    expect(due?.textContent).toMatch(/^in \d+ d$/);
    // The cards are named for assistive technology.
    expect(pay).toHaveAttribute('aria-label', 'pay, Step, Failed, attempt 3, failure HTTP_500');
  });

  it('links a child node to the child run', async () => {
    const { nodes, edges } = sagaRun();
    nodes.push(node('charge', 'WAITING', { type: 'child', waitingOn: 'child', childRunId: '019b5d6e-0000-7000-8000-0000000000ee' }));
    await open({ graph: graphOf(nodes, edges, { status: 'MANUAL_REVIEW' }) });
    const link = await screen.findByRole('link', { name: 'Open the child run of charge' });
    expect(link).toHaveAttribute('href', '/runs/019b5d6e-0000-7000-8000-0000000000ee');
    expect(within(card('charge')).getByText('Waiting for the child run')).toBeInTheDocument();
  });

  it('explains how the links are drawn, in words', async () => {
    await open();
    const legend = await screen.findByRole('list', { name: 'How links are drawn' });
    expect(legend).toHaveTextContent('Solid: the run went this way');
    expect(legend).toHaveTextContent('Dashed: not decided yet');
    expect(legend).toHaveTextContent('Dotted: omitted');
  });

  it('shows how far the graph was behind the journal when it was read', async () => {
    const { nodes, edges } = sagaRun();
    await open({ graph: graphOf(nodes, edges, { status: 'MANUAL_REVIEW', projectionLagMs: 2500 }) });
    expect(await screen.findByText(/2\.5 s behind the journal/)).toBeInTheDocument();
  });

  it('opens the details of a step with a click, and with the keyboard', async () => {
    const { user } = await open({ journal: JOURNAL });
    await waitFor(() => card('pay'));
    pick(within(card('pay')).getByText('pay'));
    expect(await screen.findByRole('complementary', { name: 'Details of pay' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Close details' }));
    expect(screen.queryByTestId('node-panel')).not.toBeInTheDocument();
    act(() => card('check').focus());
    await user.keyboard('{Enter}');
    expect(await screen.findByRole('complementary', { name: 'Details of check' })).toBeInTheDocument();
  });

  it('summarizes the selected step below the graph from the graph and journal facts', async () => {
    await open({ journal: JOURNAL });
    await waitFor(() => card('pay'));
    pick(within(card('pay')).getByText('pay'));
    const summary = await screen.findByRole('group', { name: 'Selected step' });
    expect(within(summary).getByText('pay')).toBeInTheDocument();
    expect(within(summary).getByText('Step')).toBeInTheDocument();
    expect(within(summary).getByText('Failed')).toBeInTheDocument();
    expect(within(summary).getByText('attempt 3')).toBeInTheDocument();
    expect(within(summary).getByText('HTTP_500')).toBeInTheDocument();
  });
});

describe('the run page: responsive master and detail', () => {
  it('keeps the run URL and shows the selected run in a master list on a wide viewport', async () => {
    vi.stubGlobal('matchMedia', (query: string) => ({
      matches: query.includes('min-width'),
      media: query,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
    }));
    await open({
      routes: {
        'GET /runs': () => ({ items: [runOf()], nextCursor: null }),
      },
    });
    const master = await screen.findByRole('complementary', { name: 'Runs list' });
    const selected = within(master).getByRole('link', { name: new RegExp(RUN_ID.slice(-8)) });
    expect(selected).toHaveAttribute('href', `/runs/${RUN_ID}`);
    expect(selected).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('heading', { level: 1, name: new RegExp(RUN_ID) })).toBeInTheDocument();
  });

  it('keeps the master filters, rows and scroll region while navigation resets only the run detail', async () => {
    const secondId = '019b5d6e-0000-7000-8000-000000000002';
    const first = runOf({ status: 'MANUAL_REVIEW' });
    const second = runOf({ runId: secondId, status: 'FAILED', finishedAt: null });
    const { nodes, edges } = sagaRun();
    vi.stubGlobal('matchMedia', (query: string) => ({
      matches: query.includes('min-width'),
      media: query,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
    }));
    const { user } = await open({
      routes: {
        'GET /runs': (_request, url) => {
          const status = new URL(url, 'http://studio.test').searchParams.get('status');
          const items = [second, first].filter((item) => status === null || item.status === status);
          return { items, nextCursor: null };
        },
        [`GET /runs/${secondId}`]: () => second,
        [`GET /runs/${secondId}/graph`]: () => graphOf(nodes, edges, { runId: secondId, status: 'FAILED' }),
      },
    });
    const master = await screen.findByRole('complementary', { name: 'Runs list' });
    const status = within(master).getByRole('combobox', { name: 'Status' });
    await user.selectOptions(status, 'FAILED');
    const next = await within(master).findByRole('link', { name: new RegExp(secondId.slice(-8)) });
    expect(within(next).getByText('—')).toBeInTheDocument();
    await user.click(next);
    expect(await screen.findByRole('heading', { level: 1, name: new RegExp(secondId) })).toBeInTheDocument();
    expect(screen.getByRole('complementary', { name: 'Runs list' })).toBe(master);
    expect(within(master).getByRole('combobox', { name: 'Status' })).toBe(status);
    expect(status).toHaveValue('FAILED');
    expect(within(master).getByRole('link', { name: new RegExp(secondId.slice(-8)) })).toHaveAttribute('aria-current', 'page');
  });
});

describe('the run page: live updates', () => {
  it('applies an update to the cards it names and leaves the rest alone, without reading the graph again', async () => {
    const { platform } = await open();
    await waitFor(() => card('pay'));
    const pipe = await stream(platform);
    const check = card('check');
    const before = check.innerHTML;
    pipe.send(changesOf(10, 12, [node('pay', 'RUNNING', { attempt: 4 })], { status: 'MANUAL_REVIEW', events: [graphEvent(11, 'step.retry_requested', 'pay'), graphEvent(12, 'step.leased', 'pay', 4)] }));
    await waitFor(() => expect(within(card('pay')).getByText('Running')).toBeInTheDocument());
    expect(within(card('pay')).getByText('attempt 4')).toBeInTheDocument();
    expect(within(card('pay')).queryByText(/failure/)).not.toBeInTheDocument();
    // Only the card the update touched is highlighted, and the others are the very same elements, untouched.
    expect(card('pay').querySelector('.rn-flash')).not.toBeNull();
    expect(card('check')).toBe(check);
    expect(card('check').innerHTML).toBe(before);
    expect(card('reserve~c').querySelector('.rn-flash')).toBeNull();
    expect(platform.state.graphReads).toBe(1);
    expect(follow()).toHaveTextContent('Live');
  });

  it('keeps every step where it was when updates arrive, even when the platform sends the whole graph again', async () => {
    const { platform } = await open();
    await waitFor(() => card('pay'));
    const ids = sagaRun().nodes.map((n) => n.id);
    const places = () => Object.fromEntries(ids.map((id) => [id, card(id).style.transform]));
    const before = places();
    expect(new Set(Object.values(before)).size).toBe(ids.length);
    const pipe = await stream(platform);
    pipe.send(changesOf(10, 11, [node('pay', 'RUNNING', { attempt: 4 })]));
    await waitFor(() => expect(within(card('pay')).getByText('Running')).toBeInTheDocument());
    expect(places()).toEqual(before);
    const { nodes, edges } = sagaRun();
    pipe.send(changesOf(0, 40, nodes.map((n) => (n.id === 'notify' ? node('notify', 'RUNNING', { attempt: 1 }) : n)), { reset: true, edges, status: 'RUNNING' }));
    await waitFor(() => expect(within(card('notify')).getByText('Running')).toBeInTheDocument());
    expect(places()).toEqual(before);
  });

  it('changes the status of the run when an update says so', async () => {
    const { platform } = await open();
    const pipe = await stream(platform);
    pipe.send(changesOf(10, 11, [node('pay', 'SUCCEEDED', { attempt: 4 })], { status: 'RUNNING' }));
    await waitFor(() => expect(screen.getByText('Steps are being worked on.')).toBeInTheDocument());
  });

  it('replaces the whole graph when the platform sends a reset', async () => {
    const { platform } = await open();
    await waitFor(() => card('pay'));
    const pipe = await stream(platform);
    pipe.send(changesOf(0, 80, [node('only', 'SUCCEEDED', { attempt: 1 })], { reset: true, edges: [], status: 'SUCCEEDED' }));
    await waitFor(() => expect(screen.getByTestId('rf__node-only')).toBeInTheDocument());
    expect(screen.queryByTestId('rf__node-pay')).not.toBeInTheDocument();
    expect(platform.state.graphReads).toBe(1);
  });

  it('reads the graph again when an update does not follow on from what it holds, and goes on from there', async () => {
    const { platform } = await open();
    await waitFor(() => card('notify'));
    const { nodes, edges } = sagaRun();
    nodes.push(node('late', 'READY'));
    platform.state.graph = graphOf(nodes, edges, { status: 'MANUAL_REVIEW', projectionSeq: 30 });
    const pipe = await stream(platform);
    pipe.send(changesOf(25, 31, [node('notify', 'READY')], { events: [graphEvent(31, 'step.released', 'notify')] }));
    await waitFor(() => expect(platform.state.graphReads).toBe(2));
    await waitFor(() => expect(screen.getByTestId('rf__node-late')).toBeInTheDocument());
    await waitFor(() => expect(within(card('notify')).getByText('Ready')).toBeInTheDocument());
  });

  it('says "Live", then "Reconnecting" when the connection drops, then "Ended" when the platform says the run is over', async () => {
    const { platform } = await open();
    const pipe = await stream(platform);
    pipe.send(changesOf(10, 11, []));
    await waitFor(() => expect(follow()).toHaveTextContent('Live'));
    pipe.close();
    await waitFor(() => expect(follow()).toHaveTextContent('Reconnecting'));
    const second = await waitFor(() => {
      expect(platform.pipes.length).toBe(2);
      return platform.pipes[1] as Pipe;
    }, { timeout: 3000 });
    second.send(changesOf(11, 12, [], { status: 'SUCCEEDED' }));
    second.end();
    await waitFor(() => expect(follow()).toHaveTextContent('Ended'));
  });

  it('tells assistive technology when the way updates arrive changes', async () => {
    const { platform } = await open();
    const pipe = await stream(platform);
    pipe.send(changesOf(10, 11, []));
    expect(await screen.findByText(/Live updates: Live\./)).toBeInTheDocument();
    pipe.close();
    expect(await screen.findByText(/Live updates: Reconnecting\./)).toBeInTheDocument();
  });

  it('says "Polling" and asks for the changes instead when the platform has no place for another stream', async () => {
    const { nodes, edges } = sagaRun();
    const platform = runPlatform({
      stream: () => failure(429, 'RATE_LIMITED', 'too many streams', { reason: 'STREAMS_PER_TENANT' }),
      routes: {
        [`GET /runs/${RUN_ID}/graph/changes`]: () => changesOf(10, 11, [node('notify', 'RUNNING', { attempt: 1 })], { status: 'MANUAL_REVIEW', events: [graphEvent(11, 'step.started', 'notify', 1)] }),
      },
      graph: graphOf(nodes, edges, { status: 'MANUAL_REVIEW' }),
    });
    const app = await signedInApp(platform.fetcher, OPERATOR);
    renderApp(
      <Routes>
        <Route path="/runs/:runId" element={<RunDetailPage />} />
      </Routes>,
      { app, path: `/runs/${RUN_ID}` },
    );
    await waitFor(() => expect(screen.getByTestId('follow-state')).toHaveTextContent('Polling'));
    expect(screen.getByText(/asks the platform for changes every few seconds/)).toBeInTheDocument();
    await waitFor(() => expect(within(card('notify')).getByText('Running')).toBeInTheDocument());
  });

  it('stops following when the page is left: the stream is closed and nothing more is read', async () => {
    const { platform, unmount } = await open();
    const pipe = await stream(platform);
    unmount();
    await waitFor(() => expect(() => pipe.send(changesOf(10, 11, []))).toThrow());
  });
});

describe('the run page: when it cannot be read', () => {
  it('says the graph could not be read, keeps the run on show, and reads it again on request', async () => {
    let broken = true;
    const { nodes, edges } = sagaRun();
    const { user } = await open({
      routes: {
        [`GET /runs/${RUN_ID}/graph`]: () => (broken ? failure(503, 'UNAVAILABLE', 'The platform is not available right now.') : graphOf(nodes, edges, { status: 'MANUAL_REVIEW' })),
      },
    });
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('Loading the graph failed.');
    expect(alert).toHaveTextContent('The platform is not available right now.');
    expect(screen.getByRole('heading', { level: 1, name: new RegExp(RUN_ID) })).toBeInTheDocument();
    broken = false;
    await user.click(within(alert).getByRole('button', { name: 'Try again' }));
    await waitFor(() => card('pay'));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('works when React runs every effect twice, as it does in development: one stream, the right graph', async () => {
    const { platform } = await openRunPage({}, OPERATOR, { strict: true });
    await waitFor(() => card('pay'));
    const pipe = await stream(platform);
    pipe.send(changesOf(10, 11, [node('pay', 'RUNNING', { attempt: 4 })]));
    await waitFor(() => expect(within(card('pay')).getByText('Running')).toBeInTheDocument());
    expect(platform.pipes).toHaveLength(1);
    expect(screen.getAllByTestId('follow-state')).toHaveLength(1);
  });
});

describe('the run page: the table', () => {
  async function table() {
    const view = await open({ journal: JOURNAL });
    await waitFor(() => card('pay'));
    await view.user.click(screen.getByRole('tab', { name: 'Table' }));
    const grid = await screen.findByRole('table', { name: 'Steps of the run' });
    return { ...view, grid };
  }
  const ids = (grid: HTMLElement) => within(grid).getAllByRole('row').slice(1).map((row) => within(row).getAllByRole('button')[0]?.textContent);

  it('lists the same steps as the graph with the same states, what needs a person first', async () => {
    const { grid } = await table();
    expect(ids(grid)).toEqual(['reserve~c', 'pay', 'order', 'wait', 'notify', 'pay~r', 'check', 'reserve', 'shipping']);
    for (const n of sagaRun().nodes) {
      const row = within(grid).getByRole('button', { name: n.id }).closest('tr') as HTMLElement;
      const label = { SUCCEEDED: 'Succeeded', FAILED: 'Failed', RUNNING: 'Running', WAITING: 'Waiting', NOT_STARTED: 'Not started', SKIPPED: 'Skipped' }[n.status as string];
      expect(within(row).getByText(label as string)).toBeInTheDocument();
      // And what the graph card says: the same state.
    }
    const pay = within(grid).getByRole('button', { name: 'pay' }).closest('tr') as HTMLElement;
    expect(within(pay).getByText('HTTP_500')).toBeInTheDocument();
    expect(within(pay).getByText('3')).toBeInTheDocument();
    const undoRow = within(grid).getByRole('button', { name: 'reserve~c' }).closest('tr') as HTMLElement;
    expect(within(undoRow).getByText('Waiting for a person')).toBeInTheDocument();
    expect(within(undoRow).getByText('Undo of reserve')).toBeInTheDocument();
    expect(grid).toHaveAttribute('aria-rowcount', '10');
  });

  it('filters to what needs attention, and says how many that is', async () => {
    const { grid, user } = await table();
    await user.click(screen.getByRole('checkbox', { name: /Only what needs attention \(2\)/ }));
    expect(ids(screen.getByRole('table', { name: 'Steps of the run' }))).toEqual(['reserve~c', 'pay']);
    expect(screen.getByText('2 of 9 steps')).toBeInTheDocument();
    expect(grid).toBeTruthy();
  });

  it('finds a step by a part of its id', async () => {
    const { user } = await table();
    await user.type(screen.getByRole('searchbox', { name: 'Find a step' }), 'PAY');
    expect(ids(screen.getByRole('table', { name: 'Steps of the run' }))).toEqual(['pay', 'pay~r']);
    await user.clear(screen.getByRole('searchbox', { name: 'Find a step' }));
    await user.type(screen.getByRole('searchbox', { name: 'Find a step' }), 'zzz');
    expect(screen.getByText('No step matches.')).toBeInTheDocument();
  });

  it('sorts by id, up and down, and says which way in the header', async () => {
    const { user, grid } = await table();
    const header = within(grid).getByRole('columnheader', { name: /Status/ });
    expect(header).toHaveAttribute('aria-sort', 'ascending');
    await user.click(within(grid).getByRole('button', { name: 'Step' }));
    expect(within(grid).getByRole('columnheader', { name: /Step/ })).toHaveAttribute('aria-sort', 'ascending');
    expect(ids(grid)).toEqual(['check', 'notify', 'order', 'pay', 'pay~r', 'reserve', 'reserve~c', 'shipping', 'wait']);
    await user.click(within(grid).getByRole('button', { name: /Step/ }));
    expect(within(grid).getByRole('columnheader', { name: /Step/ })).toHaveAttribute('aria-sort', 'descending');
    expect(ids(grid)[0]).toBe('wait');
  });

  it('follows the live updates like the graph does', async () => {
    const { platform, grid } = await table();
    const pipe = await stream(platform);
    pipe.send(changesOf(10, 11, [node('notify', 'FAILED', { attempt: 1, failureCode: 'BOOM' })]));
    await waitFor(() => expect(ids(grid).slice(0, 3)).toEqual(['reserve~c', 'notify', 'pay']));
    expect(within(within(grid).getByRole('button', { name: 'notify' }).closest('tr') as HTMLElement).getByText('BOOM')).toBeInTheDocument();
  });

  it('selects a step from the table and shows its details beside it', async () => {
    const { user, grid } = await table();
    await user.click(within(grid).getByRole('button', { name: 'pay' }));
    expect(await screen.findByRole('complementary', { name: 'Details of pay' })).toBeInTheDocument();
    expect(within(grid).getByRole('button', { name: 'pay' })).toHaveAttribute('aria-current', 'true');
  });

  it('offers a menu of actions per row that works with the keyboard alone, and closes on Escape', async () => {
    const { user } = await table();
    const trigger = screen.getByRole('button', { name: 'Actions for reserve~c' });
    trigger.focus();
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    await user.keyboard('{Enter}');
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
    const list = screen.getByRole('list', { name: 'Actions for reserve~c' });
    expect(within(list).getAllByRole('button').map((b) => b.textContent)).toEqual(['Retry step', 'Retry compensation', 'Skip compensation', 'Resolve step', 'Mark for manual review']);
    expect(within(list).getByRole('button', { name: 'Retry compensation' })).not.toHaveAttribute('aria-disabled');
    expect(within(list).getByRole('button', { name: 'Retry step' })).toHaveAttribute('aria-disabled', 'true');
    expect(within(list).getByText('This is a compensation: use Retry compensation.')).toBeInTheDocument();
    // Focus goes into the list; Escape closes it and brings focus back to the button.
    await user.tab();
    expect(within(list).getByRole('button', { name: 'Retry step' })).toHaveFocus();
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('list', { name: 'Actions for reserve~c' })).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
  });

  it('closes the actions of a row when focus leaves them', async () => {
    const { user } = await table();
    const trigger = screen.getByRole('button', { name: 'Actions for reserve~c' });
    await user.click(trigger);
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
    for (let i = 0; i < 6; i++) await user.tab();
    expect(screen.queryByRole('list', { name: 'Actions for reserve~c' })).not.toBeInTheDocument();
  });
});

describe('the run page: the timeline', () => {
  async function timeline(init: RunPlatformInit = { journal: JOURNAL }) {
    const view = await open(init);
    await view.user.click(screen.getByRole('tab', { name: 'Timeline' }));
    const grid = await screen.findByRole('table', { name: 'Journal events, oldest first' });
    return { ...view, grid };
  }
  /** The sequence numbers on show. The table is looked up each time: it is drawn afresh when a filter leaves nothing. */
  const seqs = () => within(screen.getByRole('table', { name: 'Journal events, oldest first' })).getAllByRole('row').slice(1).map((row) => within(row).getAllByRole('cell')[0]?.textContent);

  it('lists the journal with sequence, event in words, step, attempt, who, trace and payload digest', async () => {
    const { grid } = await timeline();
    await waitFor(() => expect(seqs()).toHaveLength(JOURNAL.length));
    expect(seqs()).toEqual(JOURNAL.map((e) => String(e.seq)));
    const row = (seq: number) => within(grid).getAllByRole('row')[seq] as HTMLElement;
    const started = within(grid).getByText('The run started').closest('tr') as HTMLElement;
    expect(within(started).getByText('user:alice')).toBeInTheDocument();
    expect(within(started).getByText(TRACE)).toBeInTheDocument();
    const retry = within(grid).getByText('The step failed and a retry was scheduled').closest('tr') as HTMLElement;
    expect(within(retry).getByText(/Problem/)).toBeInTheDocument();
    expect(within(retry).getByText('step.retry_scheduled')).toBeInTheDocument();
    expect(row(1)).toBeTruthy();
    // The payload is never shown, only its digest: shortened, redacted, with the whole of it to copy.
    const completed = within(grid).getByText('The step completed').closest('tr') as HTMLElement;
    expect(within(completed).getByText('payload (redacted) sha256:ffffffffffff…')).toBeInTheDocument();
    expect(within(completed).queryByText('f'.repeat(64))).not.toBeInTheDocument();
    // A worker that sent no trace.
    expect(within(completed).getByText('no trace')).toBeInTheDocument();
  });

  it('copies the whole digest, not the short form', async () => {
    const { grid, user } = await timeline();
    await waitFor(() => expect(seqs()).toHaveLength(JOURNAL.length));
    // (user-event gives the page a clipboard of its own to read back.)
    await user.click(within(grid).getByRole('button', { name: 'Copy payload digest' }));
    expect(await navigator.clipboard.readText()).toBe('f'.repeat(64));
    expect(await screen.findByText('Copied payload digest.')).toBeInTheDocument();
    await user.click(within(grid).getAllByRole('button', { name: 'Copy trace id' })[0] as HTMLElement);
    expect(await navigator.clipboard.readText()).toBe(TRACE);
  });

  it('marks decisions and problems with words, and filters to them', async () => {
    const { grid, user } = await timeline();
    await waitFor(() => expect(seqs()).toHaveLength(JOURNAL.length));
    await user.click(screen.getByRole('checkbox', { name: 'Only problems' }));
    expect(seqs()).toEqual(['7', '14', '15']);
    await user.click(screen.getByRole('checkbox', { name: 'Only decisions' }));
    expect(seqs()).toEqual(['7', '14', '15', '16']);
    await user.click(screen.getByRole('checkbox', { name: 'Only problems' }));
    expect(seqs()).toEqual(['16']);
    expect(within(grid).getByText(/Decision/)).toBeInTheDocument();
    expect(within(grid).getByText('user:bob')).toBeInTheDocument();
  });

  it('filters to one step, and clicking a step in the timeline opens its details', async () => {
    const { user } = await timeline();
    await waitFor(() => expect(seqs()).toHaveLength(JOURNAL.length));
    await user.type(screen.getByRole('searchbox', { name: 'Step' }), 'reserve');
    expect(seqs()).toEqual(['2', '3']);
    await user.click(within(screen.getByRole('table', { name: 'Journal events, oldest first' })).getAllByRole('button', { name: 'reserve' })[0] as HTMLElement);
    expect(await screen.findByRole('complementary', { name: 'Details of reserve' })).toBeInTheDocument();
  });

  it('says why an event happened, and which span of the trace an attempt is', async () => {
    const SPAN = 'e38d47a78a176150';
    const journal = [
      journalEvent(1, 'step.leased', 'pay', { attempt: 1, spanId: SPAN }),
      journalEvent(2, 'step.retry_scheduled', 'pay', { attempt: 1, spanId: SPAN, cause: { kind: 'retry', code: 'PAYMENT_DECLINED' } }),
      journalEvent(3, 'step.retry_requested', 'pay', { attempt: null, actor: 'user:bob', cause: { kind: 'decision', code: 'RETRY' } }),
      journalEvent(4, 'run.finished', null, { cause: { kind: 'outcome', code: 'SUCCEEDED' } }),
    ];
    const { grid, user } = await timeline({ journal });
    await waitFor(() => expect(seqs()).toHaveLength(4));
    expect(within(grid).getByRole('columnheader', { name: 'Cause' })).toBeInTheDocument();
    const retry = within(grid).getByText('step.retry_scheduled').closest('tr') as HTMLElement;
    expect(within(retry).getByText('PAYMENT_DECLINED')).toBeInTheDocument();
    expect(within(retry).getByText('retried')).toBeInTheDocument();
    const decision = within(grid).getByText('step.retry_requested').closest('tr') as HTMLElement;
    expect(within(decision).getByText('decided by a person')).toBeInTheDocument();
    expect(within(grid).getByText('how the run ended')).toBeInTheDocument();
    // The attempt's span is next to the trace, and copies whole.
    const leased = within(grid).getByText('step.leased').closest('tr') as HTMLElement;
    expect(within(leased).getByText(SPAN)).toBeInTheDocument();
    await user.click(within(leased).getByRole('button', { name: 'Copy span id' }));
    expect(await navigator.clipboard.readText()).toBe(SPAN);
    // An event with no cause says nothing about one.
    expect(within(leased).queryByText('retried')).not.toBeInTheDocument();
  });

  describe('how far the trace is behind the journal', () => {
    const telemetry = (state: string, lagMs: number) => ({ routes: { [`GET /runs/${RUN_ID}/telemetry`]: () => ({ runId: RUN_ID, traceId: TRACE, state, headSeq: 11, appliedSeq: state === 'CURRENT' ? 11 : 4, lagMs, budgetMs: 30_000 }) } });

    it('says nothing when the spans are current', async () => {
      await timeline({ journal: JOURNAL, ...telemetry('CURRENT', 0) });
      await waitFor(() => expect(seqs()).toHaveLength(JOURNAL.length));
      expect(screen.queryByText(/The trace of this run is behind/)).not.toBeInTheDocument();
      expect(screen.queryByText(/catching up/)).not.toBeInTheDocument();
    });

    it('says it is catching up, in seconds', async () => {
      await timeline({ journal: JOURNAL, ...telemetry('CATCHING_UP', 7_400) });
      expect(await screen.findByText(/The trace is catching up with the journal \(7 seconds behind\)/)).toBeInTheDocument();
    });

    it('warns that steps may be missing from the trace and that the journal is whole when it is behind', async () => {
      await timeline({ journal: JOURNAL, ...telemetry('BEHIND', 600_000) });
      expect(await screen.findByText('The trace of this run is behind its journal.')).toBeInTheDocument();
      expect(screen.getByText(/about 600 seconds late/)).toBeInTheDocument();
      expect(screen.getByText(/Everything in the journal below is complete/)).toBeInTheDocument();
    });

    it('says a run belongs to no trace when it does not', async () => {
      await timeline({ journal: JOURNAL, ...telemetry('NO_TRACE', 0) });
      expect(await screen.findByText(/belongs to no trace/)).toBeInTheDocument();
    });

    it('shows nothing, and no error, on a platform that does not know the endpoint', async () => {
      await timeline({ journal: JOURNAL, routes: { [`GET /runs/${RUN_ID}/telemetry`]: () => new Response('{"code":"NOT_FOUND"}', { status: 404 }) } });
      await waitFor(() => expect(seqs()).toHaveLength(JOURNAL.length));
      expect(screen.queryByText(/trace/i, { selector: '[role="alert"]' })).not.toBeInTheDocument();
    });
  });

  it('adds the events of live updates as they arrive, once each, and marks them as live', async () => {
    const { grid, platform } = await timeline();
    await waitFor(() => expect(seqs()).toHaveLength(JOURNAL.length));
    const pipe = await stream(platform);
    pipe.send(changesOf(10, 17, [node('notify', 'READY')], { events: [graphEvent(17, 'step.released', 'notify')] }));
    await waitFor(() => expect(seqs().at(-1)).toBe('17'));
    pipe.send(changesOf(17, 19, [node('notify', 'RUNNING', { attempt: 1 })], { events: [graphEvent(17, 'step.released', 'notify'), graphEvent(18, 'step.leased', 'notify', 1), graphEvent(19, 'step.started', 'notify', 1)] }));
    await waitFor(() => expect(seqs().at(-1)).toBe('19'));
    expect(seqs().slice(-3)).toEqual(['17', '18', '19']);
    expect(seqs()).toHaveLength(JOURNAL.length + 3);
    expect(within(grid).getAllByText('(live)')).toHaveLength(3);
  });

  it('reads the journal a page at a time and keeps reading as the list is scrolled', async () => {
    const journal = Array.from({ length: 450 }, (_, i) => journalEvent(i + 1, 'step.started', 'pay'));
    const { grid, platform } = await timeline({ journal, pageSize: 200 });
    await waitFor(() => expect(within(grid).getAllByRole('row').length).toBeGreaterThan(5));
    expect(grid).toHaveAttribute('aria-rowcount', '201');
    expect(screen.getByText(/200 journal events read so far/)).toBeInTheDocument();
    const reads = () => platform.calls.filter((c) => c.url.includes('/events')).length;
    expect(reads()).toBe(1);
    await userEvent.setup().click(screen.getByRole('button', { name: 'Load more events' }));
    await waitFor(() => expect(grid).toHaveAttribute('aria-rowcount', '401'));
    expect(platform.calls.filter((c) => c.url.includes('/events'))[1]?.url).toContain('cursor=200');
  });

  it('draws a window of a 10 000-event journal, not all of it, and moves the window with the scroll', async () => {
    const journal = Array.from({ length: 10_000 }, (_, i) => journalEvent(i + 1, i % 50 === 0 ? 'step.failed' : 'step.started', `s${i % 40}`));
    const { grid } = await timeline({ journal, pageSize: 10_000 });
    await waitFor(() => expect(within(grid).getAllByRole('row').length).toBeGreaterThan(5));
    expect(grid).toHaveAttribute('aria-rowcount', '10001');
    const drawn = () => within(grid).getAllByRole('row').length;
    expect(drawn()).toBeLessThan(40);
    expect(seqs()[0]).toBe('1');
    expect(screen.getByText(/All 10000 journal events are read/)).toBeInTheDocument();
    // Scroll to the end.
    const scroller = screen.getByRole('region', { name: 'Journal events' });
    Object.defineProperty(scroller, 'scrollTop', { configurable: true, value: 9_990 * 48 });
    fireEvent.scroll(scroller);
    await waitFor(() => expect(seqs().at(-1)).toBe('10000'));
    expect(drawn()).toBeLessThan(40);
    expect(seqs()).not.toContain('1');
  });

  it('says the journal is empty when it is', async () => {
    const view = await open({ journal: [] });
    await view.user.click(screen.getByRole('tab', { name: 'Timeline' }));
    expect(await screen.findByText('The journal has no events yet.')).toBeInTheDocument();
  });

  it('shows an error when the journal cannot be read, with a way to try again', async () => {
    let fail = true;
    const view = await open({
      journal: JOURNAL,
      routes: {
        [`GET /runs/${RUN_ID}/events`]: () => {
          if (fail) return failure(503, 'UNAVAILABLE', 'The platform is not available right now.');
          return { items: JOURNAL, nextCursor: null };
        },
      },
    });
    await view.user.click(screen.getByRole('tab', { name: 'Timeline' }));
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('Reading the journal failed.');
    fail = false;
    await view.user.click(within(alert).getByRole('button', { name: 'Try again' }));
    expect(await screen.findByRole('table', { name: 'Journal events, oldest first' })).toBeInTheDocument();
  });
});

describe('the run page: the details of a step', () => {
  async function select(id: string, permissions = OPERATOR) {
    const view = await open({ journal: JOURNAL }, permissions);
    await waitFor(() => card(id));
    pick(within(card(id)).getByText(id));
    const panel = await screen.findByRole('complementary', { name: `Details of ${id}` });
    return { ...view, panel };
  }

  it('tells the attempts of the step from the journal: when, how it ended, how long', async () => {
    const { panel } = await select('pay');
    const table = await within(panel).findByRole('table', { name: 'Attempts of this step, from the journal' });
    const rows = within(table).getAllByRole('row').slice(1);
    expect(rows).toHaveLength(2);
    expect(within(rows[0] as HTMLElement).getAllByRole('cell').map((c) => c.textContent?.replace(/\s+/g, ' ')).filter((_, i) => i === 0 || i === 1 || i === 4)).toEqual(['1', 'Failed, retry scheduled', '2.0 s']);
    expect(within(rows[1] as HTMLElement).getAllByRole('cell').map((c) => c.textContent).filter((_, i) => i === 0 || i === 1 || i === 4)).toEqual(['2', 'Failed', '3.0 s']);
    expect(within(panel).getByText('HTTP_500')).toBeInTheDocument();
  });

  it('shows the trace of the step, or says there is none', async () => {
    const { panel } = await select('pay');
    expect(await within(panel).findByText(TRACE)).toBeInTheDocument();
    expect(within(panel).getByRole('button', { name: 'Copy trace id' })).toBeInTheDocument();
  });

  it('says "no trace" for a step whose worker sent none', async () => {
    const { panel } = await select('reserve');
    expect(await within(panel).findByText(/no trace \(the worker did not send one/)).toBeInTheDocument();
  });

  it('says the journal keeps no data of a step, instead of showing any', async () => {
    const { panel } = await select('pay');
    expect(within(panel).getByText(/payload is redacted/)).toBeInTheDocument();
  });

  it('lists what leads into the step and out of it, so the structure is readable without the canvas', async () => {
    const { panel, user } = await select('pay');
    const comesAfter = within(panel).getByRole('heading', { name: 'Comes after' }).parentElement as HTMLElement;
    expect(within(comesAfter).getByRole('button', { name: 'reserve' })).toBeInTheDocument();
    const leadsTo = within(panel).getByRole('heading', { name: 'Leads to' }).parentElement as HTMLElement;
    expect(within(leadsTo).getByText(/On error, not decided yet|On error, taken/)).toBeInTheDocument();
    await user.click(within(leadsTo).getByRole('button', { name: 'reserve~c' }));
    expect(await screen.findByRole('complementary', { name: 'Details of reserve~c' })).toBeInTheDocument();
  });

  it('lists the decisions an operator made about the step, with their name', async () => {
    const { panel } = await select('reserve~c');
    expect(await within(panel).findByText(/An operator asked for another attempt/)).toBeInTheDocument();
    expect(within(panel).getByText(/by user:bob/)).toBeInTheDocument();
  });

  it('opens the timeline on that step', async () => {
    const { panel, user } = await select('pay');
    await user.click(within(panel).getByRole('button', { name: 'Show only this step in the timeline' }));
    const grid = await screen.findByRole('table', { name: 'Journal events, oldest first' });
    await waitFor(() => expect(within(grid).getAllByRole('row').length).toBe(1 + 6));
    expect(screen.getByRole('searchbox', { name: 'Step' })).toHaveValue('pay');
  });

  it('says so when the step is no longer in the graph', async () => {
    const { platform, panel } = await select('notify');
    const pipe = await stream(platform);
    pipe.send(changesOf(0, 90, [node('only', 'SUCCEEDED')], { reset: true, edges: [], status: 'SUCCEEDED' }));
    expect(await screen.findByText(/not in the graph any more/)).toBeInTheDocument();
    expect(panel).toBeTruthy();
  });
});

describe('the run page: big runs (D147, edge case: graph with 50k nodes)', () => {
  it('opens a run of 50 000 steps in the table, draws a window of it, and does not draw the graph unless asked', async () => {
    const statuses = ['SUCCEEDED', 'SUCCEEDED', 'NOT_STARTED', 'RUNNING', 'FAILED', 'WAITING'] as const;
    const nodes = Array.from({ length: 50_000 }, (_, i) => node(`step-${i}`, statuses[i % statuses.length] as (typeof statuses)[number], i === 7 ? { waitingOn: 'operator' } : {}));
    const started = performance.now();
    await open({ graph: graphOf(nodes, [], { status: 'RUNNING' }) });
    const grid = await screen.findByRole('table', { name: 'Steps of the run' }, { timeout: 10_000 });
    expect(screen.getByRole('tab', { name: 'Table' })).toHaveAttribute('aria-selected', 'true');
    expect(grid).toHaveAttribute('aria-rowcount', '50001');
    expect(within(grid).getAllByRole('row').length).toBeLessThan(40);
    // What needs a person comes first even among 50 000.
    expect(within(within(grid).getAllByRole('row')[1] as HTMLElement).getByRole('button', { name: 'step-7' })).toBeInTheDocument();
    expect(screen.getByText('50000 steps')).toBeInTheDocument();
    expect(document.querySelector('.react-flow')).toBeNull();
    expect(performance.now() - started).toBeLessThan(15_000);
  });

  it('draws the graph of a big run when the person insists, with simplified cards', async () => {
    const nodes = Array.from({ length: 10_050 }, (_, i) => node(`s${i}`, 'SUCCEEDED'));
    const edges = Array.from({ length: 10_049 }, (_, i) => ({ from: `s${i}`, to: `s${i + 1}`, kind: 'next' as const }));
    const { user } = await open({ graph: graphOf(nodes, edges, { status: 'RUNNING' }) });
    await screen.findByRole('table', { name: 'Steps of the run' }, { timeout: 10_000 });
    await user.click(screen.getByRole('tab', { name: 'Graph' }));
    expect(await screen.findByText(/This run has 10,050 steps/)).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Draw the graph anyway' }));
    await waitFor(() => expect(document.querySelector('.react-flow')).not.toBeNull(), { timeout: 20_000 });
    expect(screen.getByText(/Large graph: simplified cards/)).toBeInTheDocument();
    expect(screen.getByText(/overview map is left out of large graphs/)).toBeInTheDocument();
  }, 40_000);
});

describe('the run page: what the platform sends is only ever text', () => {
  const HOSTILE = '<img src=x onerror=alert(1)>';

  it('shows a hostile step id, failure code, actor and correlation id as text, in the graph, the table, the panel and the timeline', async () => {
    const nodes = [node(HOSTILE, 'FAILED', { attempt: 2, failureCode: '<script>alert(2)</script>' })];
    const { user } = await open({
      run: runOf({ correlationId: '<b>bold</b>', startedBy: '<i>eve</i>' }),
      graph: graphOf(nodes, [], { status: 'FAILED' }),
      journal: [journalEvent(1, 'step.failed', HOSTILE, { actor: '<u>worker</u>', traceId: '"><svg onload=alert(3)>' })],
    });
    await waitFor(() => card(HOSTILE));
    expect(within(card(HOSTILE)).getByText('failure <script>alert(2)</script>')).toBeInTheDocument();
    pick(within(card(HOSTILE)).getByText(HOSTILE));
    const panel = await screen.findByRole('complementary', { name: `Details of ${HOSTILE}` });
    expect(within(panel).getByRole('heading', { name: HOSTILE })).toBeInTheDocument();
    await user.click(screen.getByRole('tab', { name: 'Table' }));
    expect(within(await screen.findByRole('table', { name: 'Steps of the run' })).getByRole('button', { name: HOSTILE })).toBeInTheDocument();
    await user.click(screen.getByRole('tab', { name: 'Timeline' }));
    const grid = await screen.findByRole('table', { name: 'Journal events, oldest first' });
    expect(await within(grid).findByText('<u>worker</u>')).toBeInTheDocument();
    expect(within(grid).getByText('"><svg onload=alert(3)>')).toBeInTheDocument();
    expect(screen.getByText('<b>bold</b>')).toBeInTheDocument();
    expect(screen.getByText('<i>eve</i>')).toBeInTheDocument();
    // Nothing the platform sent became markup.
    for (const tag of ['img', 'script', 'svg', 'b', 'i', 'u']) {
      const found = Array.from(document.querySelectorAll(tag)).filter((el) => !el.closest('.react-flow__controls, .react-flow__minimap, .react-flow__edges, .react-flow__attribution'));
      expect(found).toHaveLength(0);
    }
  });

  it('clips an id of a thousand characters instead of laying it out', async () => {
    const long = 'x'.repeat(1000);
    await open({ graph: graphOf([node(long, 'READY')], [], { status: 'RUNNING' }) });
    await screen.findByTestId('follow-state');
    await userEvent.setup().click(screen.getByRole('tab', { name: 'Table' }));
    const grid = await screen.findByRole('table', { name: 'Steps of the run' });
    expect(within(grid).getByRole('button', { name: `${'x'.repeat(60)}…` })).toBeInTheDocument();
  });
});

describe('the run page: motion and the clock', () => {
  it('starts on the table for people who ask for less motion, and the tabs are the visible toggle', async () => {
    vi.stubGlobal('matchMedia', (query: string) => ({ matches: query.includes('prefers-reduced-motion'), media: query, addEventListener: () => undefined, removeEventListener: () => undefined }));
    await open();
    expect(await screen.findByRole('table', { name: 'Steps of the run' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Table' })).toHaveAttribute('aria-selected', 'true');
    expect(document.querySelector('.react-flow')).toBeNull();
    await userEvent.setup().click(screen.getByRole('tab', { name: 'Graph' }));
    await waitFor(() => expect(document.querySelector('.react-flow')).not.toBeNull());
  });

  it('counts the time of a running run up by itself, and not the time of a finished one', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const created = new Date(Date.now() - 65_000).toISOString();
      await open({ run: runOf({ status: 'MANUAL_REVIEW', createdAt: created }) });
      const dd = (await screen.findByText('Running for')).nextElementSibling as HTMLElement;
      expect(dd).toHaveTextContent(/1 min 0[5-9] s so far/);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10_000);
      });
      expect(dd).toHaveTextContent(/1 min 1[5-9] s so far/);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('the run page: who may do what', () => {
  it('offers a viewer every action disabled, each with the permission it lacks', async () => {
    const { user } = await open({}, VIEWER);
    await user.click(await screen.findByRole('button', { name: 'Run actions' }));
    const list = screen.getByRole('list', { name: 'Run actions' });
    for (const name of ['Pause run', 'Resume run', 'Cancel run', 'Run again as new']) expect(within(list).getByRole('button', { name })).toHaveAttribute('aria-disabled', 'true');
    expect(within(list).getAllByText('You need the run:operate permission to do this.')).toHaveLength(4);
  });

  it('keeps the run open for everybody (it is read-only without run:operate)', async () => {
    await open({}, VIEWER);
    await waitFor(() => card('pay'));
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeEnabled();
    expect(ADMIN).toContain('saga:override');
  });
});
