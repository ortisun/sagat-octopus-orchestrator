import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { useLocation } from 'react-router';
import { describe, expect, it, vi } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import type { Routes } from '../../../test/render';
import { createApp } from '../../app/context';
import { ResetRun, ResetsMade, entryLabel, pointLabel } from './ResetRun';
import type { ReapplyEntry } from '../../api/types';

const REASON = 'The second step used a wrong price list.';

const POINTS = {
  items: [
    { sequence: 1, kind: 'RUN_STARTED', node: null, occurredAt: '2026-11-06T10:00:00Z' },
    { sequence: 5, kind: 'STEP_COMPLETED', node: 'charge', occurredAt: '2026-11-06T10:00:03Z' },
    { sequence: 9, kind: 'STEP_COMPLETED', node: 'ship', occurredAt: '2026-11-06T10:00:05Z' },
  ],
  nextCursor: null,
};

function planAt(sequence: number) {
  if (sequence === 9) return { sequence, restored: [{ node: 'charge', kind: 'STEP' }, { node: 'ship', kind: 'STEP' }], reexecuted: [], reapply: [], requiresAcknowledgement: false, prefixEvents: 9 };
  return {
    sequence,
    restored: [{ node: 'charge', kind: 'STEP' }],
    reexecuted: [{ node: 'ship', completedInSource: true, markers: ['irreversible', 'highRisk'] }],
    reapply: [],
    requiresAcknowledgement: true,
    prefixEvents: sequence,
  };
}

const RESET = { resetId: 'reset-1', sourceRunId: 'run-1', sequence: 5, state: 'CREATED', failureReason: null, newRunId: 'run-2', reapplyReport: [], createdAt: '2026-11-06T10:05:00Z' };
/** A reset asked for on a run that has not ended: the source is being cancelled and there is no new run yet. */
const PENDING = { ...RESET, state: 'PENDING_SOURCE', newRunId: null };
const FAILED = { ...RESET, state: 'FAILED', newRunId: null, failureReason: 'BOUNDARY_NO_LONGER_ELIGIBLE' };

const CANCEL = 'Cancel and reset to a point…';
const CONFIRM_CANCEL = 'Confirm: cancel the run and reset it when it ends';

function json(body: unknown, status: number) {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function Where() {
  return <p data-testid="where">{useLocation().pathname}</p>;
}

/** The run as the page holds it: a button stands for the run ending while the dialog is open. */
function Run({ status, businessId, withList }: { status: string; businessId: string | null; withList: boolean }) {
  const [current, setCurrent] = useState(status);
  return (
    <>
      <button type="button" onClick={() => setCurrent('CANCELLED')}>
        the run ended
      </button>
      <ResetRun runId="run-1" status={current} businessId={businessId} />
      {withList ? <ResetsMade runId="run-1" /> : null}
    </>
  );
}

async function mount({ permissions = ['run:operate', 'run:start', 'run:read'], status = 'SUCCEEDED', businessId, routes = {}, holdPreview = false, withList = false }: { permissions?: string[]; status?: string; businessId?: string; routes?: Routes; holdPreview?: boolean; withList?: boolean } = {}) {
  let release = () => {};
  const held = holdPreview ? new Promise<void>((resolve) => { release = resolve; }) : undefined;
  const platform = fakePlatform({
    'GET /runs/run-1/reset-points': () => POINTS,
    'POST /runs/run-1/reset-previews': (init) => planAt((JSON.parse(init.body as string) as { sequence: number }).sequence),
    'POST /runs/run-1/resets': () => json(RESET, 202),
    'GET /runs/run-1/resets': () => ({ items: [RESET] }),
    ...routes,
  });
  // a preview that is slow: the plan is not there yet while the person has already chosen a point
  const fetcher = held === undefined ? platform.fetcher : (((url: string, init?: RequestInit) => (String(url).includes('reset-previews') ? held.then(() => platform.fetcher(url, init)) : platform.fetcher(url, init))) as typeof fetch);
  const app = createApp({ fetch: fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions } as never));
  app.namespace.set('payments');
  const view = renderApp(
    <>
      <Run status={status} businessId={businessId ?? null} withList={withList} />
      <Where />
    </>,
    { app },
  );
  return { platform, release, user: userEvent.setup(), ...view };
}

async function open(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole('button', { name: 'Reset to a point…' }));
  await screen.findByRole('radio', { name: /The start of the run/ });
}

/** Opens the dialog of a run that has not ended. */
async function openCancel(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole('button', { name: CANCEL }));
  await screen.findByRole('radio', { name: /The start of the run/ });
}

/** The requests that changed something, in order. */
function posts(platform: Awaited<ReturnType<typeof mount>>['platform'], suffix: string) {
  return platform.calls.filter((call) => call.method === 'POST' && call.url.split('?')[0]!.endsWith(suffix));
}

describe('Reset to a point', () => {
  it('is offered for a run that ended only to someone who may operate and start runs', async () => {
    for (const status of ['SUCCEEDED', 'FAILED', 'CANCELLED', 'TIMED_OUT', 'COMPENSATED', 'PARTIAL', 'COMPENSATION_FAILED']) {
      for (const permissions of [['run:operate'], ['run:start'], ['run:read']]) {
        const { unmount } = await mount({ permissions, status });
        expect(screen.queryByRole('button', { name: 'Reset to a point…' })).not.toBeInTheDocument();
        expect(screen.queryByRole('button', { name: CANCEL })).not.toBeInTheDocument();
        unmount();
      }
      const { unmount } = await mount({ status });
      expect(screen.getByRole('button', { name: 'Reset to a point…' })).toBeVisible();
      // a run that ended is never asked to be cancelled
      expect(screen.queryByRole('button', { name: CANCEL })).not.toBeInTheDocument();
      unmount();
    }
  });

  it('is offered for a run that has not ended, as cancelling it and resetting it when it ends, to the same people', async () => {
    for (const status of ['RUNNING', 'WAITING', 'PAUSED', 'CANCELLING', 'COMPENSATING', 'MANUAL_REVIEW']) {
      for (const permissions of [['run:operate'], ['run:start'], ['run:read'], []]) {
        const { unmount } = await mount({ permissions, status });
        expect(screen.queryByRole('button', { name: CANCEL })).not.toBeInTheDocument();
        expect(screen.queryByRole('button', { name: 'Reset to a point…' })).not.toBeInTheDocument();
        unmount();
      }
      const { unmount } = await mount({ status });
      expect(screen.getByRole('button', { name: CANCEL })).toBeVisible();
      // the plain offer is for a run that ended
      expect(screen.queryByRole('button', { name: 'Reset to a point…' })).not.toBeInTheDocument();
      unmount();
    }
  });

  it('names each point by where it is in the run and shows what a reset to it keeps and runs again', async () => {
    const { user } = await mount();
    await open(user);
    expect(screen.getByRole('radio', { name: /After step charge/ })).toBeVisible();
    expect(screen.getByRole('radio', { name: /After step ship/ })).toBeVisible();
    await user.click(screen.getByRole('radio', { name: /After step charge/ }));
    const plan = await screen.findByRole('region', { name: 'What the reset does' });
    expect(plan).toHaveTextContent('Kept as it was: charge');
    expect(plan).toHaveTextContent('ship (it completed in this run: running it again repeats what it did); what it does cannot be undone; it is marked high risk');
    const box = screen.getByRole('checkbox', { name: /will run again/ });
    expect(box).not.toBeChecked();
    // the explanation of the cancelling form is not pointed at when there is none
    expect(box).not.toHaveAttribute('aria-describedby');
  });

  it('forgets what was acknowledged for a point when another is chosen', async () => {
    const { user, platform } = await mount();
    await open(user);
    await user.click(screen.getByRole('radio', { name: /After step charge/ }));
    await user.click(await screen.findByRole('checkbox', { name: /will run again/ }));
    expect(screen.getByRole('checkbox', { name: /will run again/ })).toBeChecked();
    await user.click(screen.getByRole('radio', { name: /The start of the run/ }));
    await waitFor(() => expect(posts(platform, '/reset-previews')).toHaveLength(2));
    expect(await screen.findByRole('checkbox', { name: /will run again/ })).not.toBeChecked();
  });

  it('asks nothing to be acknowledged when nothing runs again', async () => {
    const { user } = await mount();
    await open(user);
    await user.click(screen.getByRole('radio', { name: /After step ship/ }));
    const plan = await screen.findByRole('region', { name: 'What the reset does' });
    expect(plan).toHaveTextContent('Runs again: nothing');
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument();
  });

  it('sends nothing until a point, the acknowledgement and a reason are given', async () => {
    const { user, platform } = await mount();
    await open(user);
    await user.click(screen.getByRole('button', { name: 'Confirm: reset the run' }));
    expect(await screen.findByText('Choose where to reset the run to.')).toBeVisible();
    await user.click(screen.getByRole('radio', { name: /After step charge/ }));
    await screen.findByRole('region', { name: 'What the reset does' });
    await user.click(screen.getByRole('button', { name: 'Confirm: reset the run' }));
    expect(await screen.findByText(/Write at least 8 characters/)).toBeVisible();
    await user.type(screen.getByLabelText('Reason'), REASON);
    await user.click(screen.getByRole('button', { name: 'Confirm: reset the run' }));
    expect(await screen.findByText('Confirm that you know these steps will run again.')).toBeVisible();
    expect(posts(platform, '/resets')).toHaveLength(0);
  });

  it('does not send a reset before what it would do has been read', async () => {
    const { user, platform, release } = await mount({ holdPreview: true });
    await open(user);
    await user.click(screen.getByRole('radio', { name: /After step charge/ }));
    expect(await screen.findByText('Reading what the reset would do…')).toBeVisible();
    await user.type(screen.getByLabelText('Reason'), REASON);
    await user.click(screen.getByRole('button', { name: 'Confirm: reset the run' }));
    expect(posts(platform, '/resets')).toHaveLength(0);
    // once it is read, the person sees what runs again and must say they know
    release();
    await user.click(await screen.findByRole('checkbox', { name: /will run again/ }));
    await user.click(screen.getByRole('button', { name: 'Confirm: reset the run' }));
    await waitFor(() => expect(posts(platform, '/resets')).toHaveLength(1));
  });

  it('makes the reset once confirmed, with a key, and takes the person to the new run', async () => {
    const { user, platform } = await mount();
    await open(user);
    await user.click(screen.getByRole('radio', { name: /After step charge/ }));
    await screen.findByRole('region', { name: 'What the reset does' });
    await user.click(screen.getByRole('checkbox', { name: /will run again/ }));
    await user.type(screen.getByLabelText('Reason'), `  ${REASON}  `);
    await user.click(screen.getByRole('button', { name: 'Confirm: reset the run' }));
    await waitFor(() => expect(screen.getByTestId('where')).toHaveTextContent('/runs/run-2'));
    const sent = posts(platform, '/resets');
    expect(sent).toHaveLength(1);
    expect(JSON.parse(sent[0]!.init.body as string)).toEqual({ sequence: 5, reason: REASON, acknowledgeReexecution: true });
    expect(new Headers(sent[0]!.init.headers).get('Idempotency-Key')).toBeTruthy();
    expect(screen.queryByRole('radio')).not.toBeInTheDocument();
  });

  it('does not claim an acknowledgement nothing asked for', async () => {
    const { user, platform } = await mount();
    await open(user);
    await user.click(screen.getByRole('radio', { name: /After step ship/ }));
    await screen.findByRole('region', { name: 'What the reset does' });
    await user.type(screen.getByLabelText('Reason'), REASON);
    await user.click(screen.getByRole('button', { name: 'Confirm: reset the run' }));
    await waitFor(() => expect(posts(platform, '/resets')).toHaveLength(1));
    expect(JSON.parse(posts(platform, '/resets')[0]!.init.body as string)).toEqual({ sequence: 9, reason: REASON, acknowledgeReexecution: false });
  });

  it('offers the choice about the business identifier only to a run that has one', async () => {
    const without = await mount();
    await open(without.user);
    expect(screen.queryByText(/Business identifier/)).not.toBeInTheDocument();
    without.unmount();

    const { user, platform } = await mount({ businessId: 'order-1001' });
    await open(user);
    expect(screen.getByText('Business identifier order-1001')).toBeVisible();
    await user.click(screen.getByRole('radio', { name: /After step ship/ }));
    await screen.findByRole('region', { name: 'What the reset does' });
    await user.click(screen.getByRole('radio', { name: /without it/ }));
    await user.type(screen.getByLabelText('Reason'), REASON);
    await user.click(screen.getByRole('button', { name: 'Confirm: reset the run' }));
    await waitFor(() => expect(posts(platform, '/resets')).toHaveLength(1));
    expect(JSON.parse(posts(platform, '/resets')[0]!.init.body as string)).toEqual({ sequence: 9, reason: REASON, acknowledgeReexecution: false, businessId: 'DROP' });
  });

  it('keeps the key of an identical request sent again and changes it when the request changes', async () => {
    const { user, platform } = await mount({ routes: { 'POST /runs/run-1/resets': () => json({ code: 'UNAVAILABLE', message: 'Try again.', retryable: true }, 503) } });
    await open(user);
    await user.click(screen.getByRole('radio', { name: /After step ship/ }));
    await screen.findByRole('region', { name: 'What the reset does' });
    await user.type(screen.getByLabelText('Reason'), REASON);
    await user.click(screen.getByRole('button', { name: 'Confirm: reset the run' }));
    await screen.findByRole('alert');
    await user.click(screen.getByRole('button', { name: 'Confirm: reset the run' }));
    await waitFor(() => expect(posts(platform, '/resets')).toHaveLength(2));
    const key = (index: number) => new Headers(posts(platform, '/resets')[index]!.init.headers).get('Idempotency-Key');
    expect(key(0)).toBeTruthy();
    expect(key(1)).toBe(key(0));
    await user.type(screen.getByLabelText('Reason'), ' Again.');
    await user.click(screen.getByRole('button', { name: 'Confirm: reset the run' }));
    await waitFor(() => expect(posts(platform, '/resets')).toHaveLength(3));
    expect(key(2)).not.toBe(key(0));
  });

  it('says in words that another open run holds the identifier, and stays open', async () => {
    const { user } = await mount({
      businessId: 'order-1001',
      routes: { 'POST /runs/run-1/resets': () => json({ code: 'CONFLICT', message: 'The business identifier is in use.', retryable: false, details: { reason: 'BUSINESS_ID_IN_USE', runId: 'run-9' } }, 409) },
    });
    await open(user);
    await user.click(screen.getByRole('radio', { name: /After step ship/ }));
    await screen.findByRole('region', { name: 'What the reset does' });
    await user.type(screen.getByLabelText('Reason'), REASON);
    await user.click(screen.getByRole('button', { name: 'Confirm: reset the run' }));
    expect(await screen.findByText(/Another open run holds this business identifier/)).toBeVisible();
    expect(screen.getByRole('radio', { name: /After step ship/ })).toBeVisible();
    expect(screen.getByTestId('where')).toHaveTextContent('/');
  });

  it('says why a run cannot be reset when the platform refuses to list its points', async () => {
    const { user } = await mount({
      routes: { 'GET /runs/run-1/reset-points': () => json({ code: 'VALIDATION_FAILED', message: 'Child runs cannot be reset in this version.', retryable: false, details: { reason: 'RESET_NOT_SUPPORTED_FOR_SOURCE' } }, 422) },
    });
    await user.click(screen.getByRole('button', { name: 'Reset to a point…' }));
    expect(await screen.findByText(/cannot be reset in this version/)).toBeVisible();
    expect(screen.queryByRole('radio')).not.toBeInTheDocument();
  });

  it('says so when there is no point to reset to', async () => {
    const { user } = await mount({ routes: { 'GET /runs/run-1/reset-points': () => ({ items: [], nextCursor: null }) } });
    await user.click(screen.getByRole('button', { name: 'Reset to a point…' }));
    expect(await screen.findByText('This run cannot be reset to any point.')).toBeVisible();
  });

  it('words a point without leaking the number of a row of the journal', () => {
    expect(pointLabel({ kind: 'RUN_STARTED', node: null })).toBe('The start of the run');
    expect(pointLabel({ kind: 'STEP_COMPLETED', node: 'charge' })).toBe('After step charge');
    expect(pointLabel({ kind: 'CHILD_FINISHED', node: 'kid' })).toBe('After child run kid');
    expect(pointLabel({ kind: 'COLLECTION_FINISHED', node: 'items' })).toBe('After collection items');
    expect(pointLabel({ kind: 'LOOP_FINISHED', node: 'retry' })).toBe('After loop retry');
  });
});

/** A platform that has the reset made by the answer given: the list of the register is empty until then. */
function register(answer: unknown) {
  let made = false;
  return {
    'POST /runs/run-1/resets': () => {
      made = true;
      return json(answer, 202);
    },
    'GET /runs/run-1/resets': () => ({ items: made ? [answer] : [] }),
  };
}

describe('Cancel the run and reset it when it ends', () => {
  /** Opens the dialog of a run that has not ended and gets it ready to send: a point, its plan and a reason. */
  async function ready(user: ReturnType<typeof userEvent.setup>, point: RegExp = /After step ship/) {
    await openCancel(user);
    await user.click(screen.getByRole('radio', { name: point }));
    await screen.findByRole('region', { name: 'What the reset does' });
    await user.type(screen.getByLabelText('Reason'), REASON);
  }
  /** As above, and the acknowledgement of the steps that run again given. */
  async function readyAcknowledged(user: ReturnType<typeof userEvent.setup>) {
    await ready(user);
    await user.click(screen.getByRole('checkbox', { name: /before this run ends/ }));
  }
  const confirm = () => screen.getByRole('button', { name: CONFIRM_CANCEL });

  it('says plainly that the run is cancelled and that the new run is made when it has ended', async () => {
    const { user } = await mount({ status: 'RUNNING' });
    await openCancel(user);
    const dialog = screen.getByRole('dialog');
    expect(within(dialog).getByRole('heading', { name: 'Cancel the run and reset it when it ends' })).toBeVisible();
    expect(dialog).toHaveTextContent('This run has not ended. Confirming cancels it, in the ordinary way.');
    expect(dialog).toHaveTextContent('The new run is made once it has ended; if the reset can no longer be made by then, it fails and the list of resets made from this run says why.');
    expect(within(dialog).getByRole('button', { name: CONFIRM_CANCEL })).toBeVisible();
    // the run stays as it is only in the plain reset
    expect(dialog).not.toHaveTextContent('This run stays as it is.');
    expect(within(dialog).queryByRole('button', { name: 'Confirm: reset the run' })).not.toBeInTheDocument();
    // "Cancel" next to a button that cancels the run would be read as cancelling the run
    expect(within(dialog).queryByRole('button', { name: 'Cancel' })).not.toBeInTheDocument();
    await user.click(within(dialog).getByRole('button', { name: 'Do not cancel the run' }));
    expect(screen.queryByRole('radio')).not.toBeInTheDocument();
  });

  it('keeps the plain dialog, with no talk of cancelling, for a run that ended', async () => {
    const { user } = await mount({ status: 'SUCCEEDED' });
    await open(user);
    const dialog = screen.getByRole('dialog');
    expect(within(dialog).getByRole('heading', { name: 'Reset to a point' })).toBeVisible();
    expect(dialog).toHaveTextContent('This run stays as it is.');
    expect(dialog).not.toHaveTextContent(/has not ended|cancels it/);
    expect(within(dialog).getByRole('button', { name: 'Confirm: reset the run' })).toBeVisible();
    expect(within(dialog).getByRole('button', { name: 'Cancel' })).toBeVisible();
  });

  it('asks for the acknowledgement whatever the plan says, because it cannot know what will complete, and sends it with the cancellation', async () => {
    const { user, platform } = await mount({ status: 'RUNNING' });
    // at `ship` the plan says nothing runs again
    await ready(user);
    expect(screen.getByRole('region', { name: 'What the reset does' })).toHaveTextContent('Runs again: nothing');
    const box = screen.getByRole('checkbox', { name: /including any that complete before this run ends/ });
    expect(box).not.toBeChecked();
    expect(screen.getByText(/The Studio cannot tell which steps will complete while the run is being cancelled/)).toBeVisible();
    await user.click(confirm());
    expect(await screen.findByText('Confirm that you know steps may run again.')).toBeVisible();
    expect(posts(platform, '/resets')).toHaveLength(0);
    await user.click(box);
    await user.click(confirm());
    await waitFor(() => expect(posts(platform, '/resets')).toHaveLength(1));
    const sent = posts(platform, '/resets')[0]!;
    expect(JSON.parse(sent.init.body as string)).toEqual({ sequence: 9, reason: REASON, acknowledgeReexecution: true, sourcePolicy: 'CANCEL_SOURCE' });
    expect(new Headers(sent.init.headers).get('Idempotency-Key')).toBeTruthy();
    // the plan is read as it is for any reset: the policy is not part of what is previewed
    expect(JSON.parse(posts(platform, '/reset-previews')[0]!.init.body as string)).toEqual({ sequence: 9 });
  });

  it('explains why it asks, to someone using a screen reader as well, and not before the plan of the point has been read', async () => {
    const { user, release } = await mount({ status: 'RUNNING', holdPreview: true });
    await openCancel(user);
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument();
    await user.click(screen.getByRole('radio', { name: /After step ship/ }));
    expect(await screen.findByText('Reading what the reset would do…')).toBeVisible();
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument();
    release();
    const box = await screen.findByRole('checkbox', { name: /before this run ends/ });
    expect(box).toHaveAccessibleDescription('The Studio cannot tell which steps will complete while the run is being cancelled, so this is always asked.');
  });

  it('asks once, not twice, when the plan asks for the acknowledgement as well', async () => {
    const { user, platform } = await mount({ status: 'RUNNING' });
    await ready(user, /After step charge/);
    expect(screen.getAllByRole('checkbox')).toHaveLength(1);
    await user.click(screen.getByRole('checkbox', { name: /including any that complete before this run ends/ }));
    await user.click(confirm());
    await waitFor(() => expect(posts(platform, '/resets')).toHaveLength(1));
    expect(JSON.parse(posts(platform, '/resets')[0]!.init.body as string)).toEqual({ sequence: 5, reason: REASON, acknowledgeReexecution: true, sourcePolicy: 'CANCEL_SOURCE' });
  });

  it('forgets the acknowledgement when another point is chosen, as the plain reset does', async () => {
    const { user, platform } = await mount({ status: 'RUNNING' });
    await readyAcknowledged(user);
    await user.click(screen.getByRole('radio', { name: /The start of the run/ }));
    await waitFor(() => expect(posts(platform, '/reset-previews')).toHaveLength(2));
    expect(await screen.findByRole('checkbox', { name: /before this run ends/ })).not.toBeChecked();
  });

  it('never sends the source policy for a run that ended, so an older platform never sees the field', async () => {
    const { user, platform } = await mount({ status: 'SUCCEEDED' });
    await open(user);
    await user.click(screen.getByRole('radio', { name: /After step ship/ }));
    await screen.findByRole('region', { name: 'What the reset does' });
    await user.type(screen.getByLabelText('Reason'), REASON);
    await user.click(screen.getByRole('button', { name: 'Confirm: reset the run' }));
    await waitFor(() => expect(posts(platform, '/resets')).toHaveLength(1));
    const body = JSON.parse(posts(platform, '/resets')[0]!.init.body as string) as object;
    expect(body).not.toHaveProperty('sourcePolicy');
    // and, for a run that ended, asks for no acknowledgement the plan did not ask for
    expect(body).toHaveProperty('acknowledgeReexecution', false);
  });

  it('does not take the person anywhere when the run is only being cancelled, and says what happens next', async () => {
    const { user, platform } = await mount({ status: 'RUNNING', withList: true, routes: register(PENDING) });
    await readyAcknowledged(user);
    await user.click(confirm());
    const dialog = await screen.findByRole('dialog');
    await waitFor(() => expect(dialog).toHaveTextContent('The run is being cancelled.'));
    expect(dialog).toHaveTextContent('The new run is made once it has ended; you will find it under Resets made from this run. If the reset can no longer be made by then, that list says why.');
    // there is no new run to go to, and the person is not told one was made
    expect(screen.getByTestId('where')).toHaveTextContent('/');
    expect(within(dialog).queryByRole('button', { name: 'Open the new run' })).not.toBeInTheDocument();
    expect(dialog).not.toHaveTextContent('The reset was made');
    expect(document.querySelector('[aria-live="polite"]')).toHaveTextContent('The run is being cancelled. The new run is made when it has ended.');
    // the register is read again, and shows the reset as waiting
    const list = await screen.findByRole('region', { name: 'Resets made from this run' });
    expect(list).toHaveTextContent('A reset to point 5');
    expect(list).toHaveTextContent('waiting for this run to end');
    expect(platform.calls.filter((call) => call.method === 'GET' && call.url.endsWith('/runs/run-1/resets'))).toHaveLength(2);
    await user.click(within(dialog).getByRole('button', { name: 'Close' }));
    expect(screen.queryByText('The run is being cancelled.')).not.toBeInTheDocument();
    expect(screen.getByTestId('where')).toHaveTextContent('/');
  });

  it('goes to the new run when the run had ended by the time the request was answered', async () => {
    const { user } = await mount({ status: 'RUNNING', routes: register(RESET) });
    await readyAcknowledged(user);
    await user.click(confirm());
    await waitFor(() => expect(screen.getByTestId('where')).toHaveTextContent('/runs/run-2'));
    expect(screen.queryByText('The run is being cancelled.')).not.toBeInTheDocument();
  });

  it('goes to the new run for an answer in a state it does not know, when the answer has a run', async () => {
    const { user } = await mount({ status: 'RUNNING', routes: register({ ...RESET, state: 'SOMETHING_NEWER' }) });
    await readyAcknowledged(user);
    await user.click(confirm());
    await waitFor(() => expect(screen.getByTestId('where')).toHaveTextContent('/runs/run-2'));
    expect(screen.queryByText('The run is being cancelled.')).not.toBeInTheDocument();
  });

  it('says why in words when the answer is a reset that failed, and shows a reason it does not know as written', async () => {
    for (const [failureReason, words] of [
      ['BOUNDARY_NO_LONGER_ELIGIBLE', 'the point is not one the run could be reset to once it had ended'],
      ['SOMETHING_NEWER', 'SOMETHING_NEWER'],
    ] as const) {
      const { user, unmount } = await mount({ status: 'RUNNING', routes: register({ ...FAILED, failureReason }) });
      await readyAcknowledged(user);
      await user.click(confirm());
      const dialog = await screen.findByRole('dialog');
      await waitFor(() => expect(dialog).toHaveTextContent(`The reset could not be made: ${words}.`));
      expect(screen.getByTestId('where')).toHaveTextContent('/');
      expect(dialog).not.toHaveTextContent('The reset was made');
      expect(within(dialog).queryByRole('button', { name: 'Open the new run' })).not.toBeInTheDocument();
      await user.click(within(dialog).getByRole('button', { name: 'Close' }));
      expect(screen.queryByText(/The reset could not be made: /)).not.toBeInTheDocument();
      // what is announced is the short sentence, and it is announced
      expect(document.querySelector('[aria-live="polite"]')).toHaveTextContent('The reset could not be made.');
      unmount();
    }
  });

  it('says in words that the platform has not enabled it, tells the person to wait for the run to end, and stays open', async () => {
    const { user } = await mount({
      status: 'RUNNING',
      routes: {
        'POST /runs/run-1/resets': () =>
          json({ code: 'VALIDATION_FAILED', message: 'The reset cannot be made.', retryable: false, remediation: 'Ask for `sourcePolicy: REQUIRE_TERMINAL`.', details: { reason: 'RESET_CANCEL_SOURCE_NOT_AVAILABLE' } }, 422),
      },
    });
    await readyAcknowledged(user);
    await user.click(confirm());
    const alert = await screen.findByText(/This platform has not been set up to cancel a run for a reset\./);
    expect(alert).toHaveTextContent('Wait for the run to end, then reset it.');
    expect(alert).toHaveTextContent('RESET_CANCEL_SOURCE_NOT_AVAILABLE');
    // the words are the Studio's: the platform's advice is for someone who writes requests
    expect(alert).not.toHaveTextContent('REQUIRE_TERMINAL');
    expect(alert).not.toHaveTextContent('The reset cannot be made.');
    expect(screen.getByRole('radio', { name: /After step ship/ })).toBeVisible();
    expect(screen.getByTestId('where')).toHaveTextContent('/');
  });

  it('says in words that the namespace has as many resets waiting as it may, and stays open', async () => {
    const { user } = await mount({
      status: 'RUNNING',
      routes: { 'POST /runs/run-1/resets': () => json({ code: 'QUOTA_EXCEEDED', message: 'Too many.', retryable: true, traceId: 'c'.repeat(32), details: { reason: 'PENDING_RESET_LIMIT', limit: 16 } }, 429) },
    });
    await readyAcknowledged(user);
    await user.click(confirm());
    const alert = await screen.findByText(/This namespace already has as many resets waiting for a run to end as it may\./);
    expect(alert).toHaveTextContent('Wait until one of them has been made or has failed, then try again.');
    expect(alert).toHaveTextContent('QUOTA_EXCEEDED');
    expect(alert).toHaveTextContent('PENDING_RESET_LIMIT');
    // what a person quotes to whoever runs the platform is kept
    expect(alert).toHaveTextContent(`Trace ${'c'.repeat(32)}`);
    expect(alert).not.toHaveTextContent('Too many.');
    expect(screen.getByRole('radio', { name: /After step ship/ })).toBeVisible();
  });

  it('shows any other refusal as the platform worded it', async () => {
    const { user } = await mount({
      status: 'RUNNING',
      routes: { 'POST /runs/run-1/resets': () => json({ code: 'VALIDATION_FAILED', message: 'That point cannot be used.', retryable: false, details: { reason: 'RESET_POINT_NOT_ELIGIBLE' } }, 422) },
    });
    await readyAcknowledged(user);
    await user.click(confirm());
    expect(await screen.findByText(/That point cannot be used\./)).toBeVisible();
    expect(screen.queryByText(/has not been set up to cancel/)).not.toBeInTheDocument();
    expect(screen.queryByText(/as many resets waiting/)).not.toBeInTheDocument();
  });

  it('keeps the key of an identical request sent again, with the policy in it, and changes it when the request changes', async () => {
    const { user, platform } = await mount({ status: 'RUNNING', routes: { 'POST /runs/run-1/resets': () => json({ code: 'UNAVAILABLE', message: 'Try again.', retryable: true }, 503) } });
    await readyAcknowledged(user);
    await user.click(confirm());
    await screen.findByText(/Try again\./);
    await user.click(confirm());
    await waitFor(() => expect(posts(platform, '/resets')).toHaveLength(2));
    const key = (index: number) => new Headers(posts(platform, '/resets')[index]!.init.headers).get('Idempotency-Key');
    const body = (index: number) => JSON.parse(posts(platform, '/resets')[index]!.init.body as string) as Record<string, unknown>;
    expect(key(0)).toBeTruthy();
    expect(key(1)).toBe(key(0));
    expect(body(1)).toEqual(body(0));
    expect(body(1)['sourcePolicy']).toBe('CANCEL_SOURCE');
    await user.type(screen.getByLabelText('Reason'), ' Again.');
    await user.click(confirm());
    await waitFor(() => expect(posts(platform, '/resets')).toHaveLength(3));
    expect(key(2)).not.toBe(key(0));
    expect(body(2)['sourcePolicy']).toBe('CANCEL_SOURCE');
  });

  it('keeps asking for the cancellation when the run ends while the dialog is open, so a retry after a lost answer is the same request', async () => {
    const { user, platform } = await mount({ status: 'RUNNING', routes: { 'POST /runs/run-1/resets': () => json({ code: 'UNAVAILABLE', message: 'Try again.', retryable: true }, 503) } });
    await readyAcknowledged(user);
    await user.click(confirm());
    await screen.findByText(/Try again\./);
    // the cancellation the first request asked for has ended the run
    await user.click(screen.getByRole('button', { name: 'the run ended' }));
    expect(within(screen.getByRole('dialog')).getByRole('heading', { name: 'Cancel the run and reset it when it ends' })).toBeVisible();
    await user.click(confirm());
    await waitFor(() => expect(posts(platform, '/resets')).toHaveLength(2));
    const key = (index: number) => new Headers(posts(platform, '/resets')[index]!.init.headers).get('Idempotency-Key');
    expect(key(1)).toBe(key(0));
    expect(JSON.parse(posts(platform, '/resets')[1]!.init.body as string)).toEqual(JSON.parse(posts(platform, '/resets')[0]!.init.body as string));
    // once the dialog is closed, the offer is the one for a run that ended
    await user.click(screen.getByRole('button', { name: 'Do not cancel the run' }));
    expect(screen.getByRole('button', { name: 'Reset to a point…' })).toBeVisible();
    expect(screen.queryByRole('button', { name: CANCEL })).not.toBeInTheDocument();
  });
});

describe('Resets made from a run', () => {
  async function list({ permissions = ['run:read'], items = [RESET], answer }: { permissions?: string[]; items?: unknown[]; answer?: () => Response } = {}) {
    const platform = fakePlatform({ 'GET /runs/run-1/resets': answer ?? (() => ({ items })) });
    const app = createApp({ fetch: platform.fetcher });
    await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions } as never));
    app.namespace.set('payments');
    return { platform, ...renderApp(<ResetsMade runId="run-1" />, { app }) };
  }

  it('links each reset to the run it made', async () => {
    await list();
    const link = await screen.findByRole('link', { name: 'A run reset to point 5' });
    expect(link).toHaveAttribute('href', '/runs/run-2');
    expect(screen.getByRole('region', { name: 'Resets made from this run' })).toHaveTextContent('created');
  });

  it('shows nothing for a platform that does not serve the register, and says so for any other failure', async () => {
    const missing = await list({ answer: () => json({ code: 'NOT_FOUND', message: 'No such route.', retryable: false }, 404) });
    // the request failed, and the failure is what is on show: nothing
    await waitFor(() => expect(missing.app.queries.getQueryState(['run', 'run-1', 'resets', 'payments'])?.status).toBe('error'));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    missing.unmount();
    await list({ answer: () => json({ code: 'UNAVAILABLE', message: 'Try again.', retryable: true }, 503) });
    expect(await screen.findByRole('alert')).toBeVisible();
  });

  it('shows nothing when there is none, and asks nothing of someone who may not read runs', async () => {
    const { platform, unmount } = await list({ items: [] });
    await waitFor(() => expect(platform.calls).toHaveLength(1));
    expect(screen.queryByRole('region')).not.toBeInTheDocument();
    unmount();
    const denied = await list({ permissions: [] });
    expect(denied.platform.calls).toHaveLength(0);
    expect(screen.queryByRole('region')).not.toBeInTheDocument();
  });

  it('shows a reset that waits for its run to end as waiting, a reset that failed with why, and links only the one that made a run', async () => {
    await list({ items: [PENDING, { ...FAILED, resetId: 'reset-2', sequence: 3 }, { ...RESET, resetId: 'reset-3', sequence: 1 }] });
    const section = await screen.findByRole('region', { name: 'Resets made from this run' });
    const [waiting, failed, created] = Array.from(section.querySelectorAll('li')) as HTMLElement[];
    expect(waiting).toHaveTextContent('A reset to point 5 (waiting for this run to end');
    expect(within(waiting!).queryByRole('link')).not.toBeInTheDocument();
    expect(failed).toHaveTextContent('A reset to point 3 (could not be made: the point is not one the run could be reset to once it had ended');
    expect(within(failed!).queryByRole('link')).not.toBeInTheDocument();
    expect(created).toHaveTextContent('A run reset to point 1 (created');
    expect(within(created!).getByRole('link', { name: 'A run reset to point 1' })).toHaveAttribute('href', '/runs/run-2');
    // the state is said by words, not by the name the platform gives it
    expect(section).not.toHaveTextContent(/PENDING|pending source|FAILED/);
  });

  it('words every reason a reset can fail for, and shows a name it does not know as written', async () => {
    const reasons: [string | null, string][] = [
      ['BOUNDARY_NO_LONGER_ELIGIBLE', 'the point is not one the run could be reset to once it had ended'],
      ['ADMISSION_REFUSED', 'a quota or the platform refused the new run'],
      ['BUSINESS_ID_IN_USE', 'another open run holds the business identifier'],
      ['AUTHORITY_REVOKED', 'the person who asked no longer has the permissions a reset needs'],
      ['REEXECUTION_NOT_ACKNOWLEDGED', 'steps completed after the point and the request did not say they may run again'],
      ['REAPPLY_TOO_MANY', 'there were too many signals and updates to deliver again'],
      ['CONCURRENCY_PERMIT_UNAVAILABLE', 'the limit on runs at once left no room for the new run'],
      ['RUN_PURGED', 'the run was deleted before the new run could be made'],
      ['SOMETHING_NEWER', 'SOMETHING_NEWER'],
      // a name that is also a property of every object is still only a name
      ['constructor', 'constructor'],
      ['x'.repeat(200), 'x'.repeat(60) + '…'],
      [null, 'no reason was recorded'],
      ['', 'no reason was recorded'],
    ];
    await list({ items: reasons.map(([failureReason], index) => ({ ...FAILED, resetId: `reset-${index}`, sequence: index + 1, failureReason })) });
    const section = await screen.findByRole('region', { name: 'Resets made from this run' });
    const items = Array.from(section.querySelectorAll('li')).map((item) => item.querySelector('.hint')?.textContent ?? '');
    expect(items).toHaveLength(reasons.length);
    reasons.forEach(([, words], index) => expect(items[index]).toMatch(new RegExp(`^\\(could not be made: ${words}, `)));
  });

  it('shows a state it does not know as written, and not as an error', async () => {
    await list({ items: [{ ...RESET, state: 'SNOOZED', newRunId: null }] });
    expect(await screen.findByRole('region', { name: 'Resets made from this run' })).toHaveTextContent('A reset to point 5 (SNOOZED');
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('reads the list again when the window is focused again, even right after it was read, and shows what the engine made meanwhile', async () => {
    let items: unknown[] = [PENDING];
    const { platform } = await list({ answer: () => json({ items }, 200) });
    expect(await screen.findByText(/waiting for this run to end/)).toBeVisible();
    expect(platform.calls).toHaveLength(1);
    items = [RESET];
    fireEvent(document, new Event('visibilitychange', { bubbles: true }));
    expect(await screen.findByRole('link', { name: 'A run reset to point 5' })).toHaveAttribute('href', '/runs/run-2');
    expect(platform.calls).toHaveLength(2);
    expect(screen.queryByText(/waiting for this run to end/)).not.toBeInTheDocument();
  });

  it('does not read the list on a timer', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const { platform, app } = await list({ items: [PENDING] });
      expect(await screen.findByText(/waiting for this run to end/)).toBeVisible();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10 * 60_000);
      });
      expect(platform.calls).toHaveLength(1);
      // nor is a timer asked for: the register is read again when the person looks, not while nobody does
      const observers = app.queries.getQueryCache().find({ queryKey: ['run', 'run-1', 'resets', 'payments'] })?.observers ?? [];
      expect(observers).toHaveLength(1);
      expect(observers[0]!.options.refetchInterval).toBeUndefined();
    } finally {
      vi.useRealTimers();
    }
  });
});

const SIGNAL: ReapplyEntry = { sequence: 9, kind: 'SIGNAL', name: 'payment.confirmed', outcome: 'WILL_TRY' };
const UPDATE: ReapplyEntry = { sequence: 12, kind: 'UPDATE', operation: 'balance.replace', outcome: 'WILL_TRY' };

/** A platform whose preview lists `candidates` for a reset that asks for signals, updates or both, and none otherwise. */
function withCandidates(candidates: ReapplyEntry[], report: ReapplyEntry[] = []) {
  return {
    'POST /runs/run-1/reset-previews': (init: RequestInit) => {
      const body = JSON.parse(init.body as string) as { sequence: number; reapply?: string };
      return { ...planAt(body.sequence), reapply: body.reapply === undefined ? [] : candidates };
    },
    'POST /runs/run-1/resets': () => json({ ...RESET, reapplyReport: report }, 202),
  };
}

describe('Delivering signals and updates again', () => {
  it('offers the four choices, with nothing delivered again as the one chosen, and sends none of them by default', async () => {
    const { user, platform } = await mount({ routes: withCandidates([SIGNAL]) });
    await open(user);
    const group = screen.getByRole('group', { name: /Deliver again what the run received after that point/ });
    expect(group).toHaveTextContent('go to the new run in the order they arrived');
    expect(screen.getByRole('radio', { name: /Nothing: the new run does not receive them/ })).toBeChecked();
    for (const name of [/The signals the run received/, /The updates whose arguments were kept/, /Both, in the order they arrived/]) {
      expect(screen.getByRole('radio', { name })).not.toBeChecked();
    }
    await user.click(screen.getByRole('radio', { name: /After step ship/ }));
    const plan = await screen.findByRole('region', { name: 'What the reset does' });
    expect(plan).not.toHaveTextContent('Delivered again');
    await user.type(screen.getByLabelText('Reason'), REASON);
    await user.click(screen.getByRole('button', { name: 'Confirm: reset the run' }));
    await waitFor(() => expect(posts(platform, '/resets')).toHaveLength(1));
    expect(JSON.parse(posts(platform, '/resets')[0]!.init.body as string)).toEqual({ sequence: 9, reason: REASON, acknowledgeReexecution: false });
    expect(JSON.parse(posts(platform, '/reset-previews')[0]!.init.body as string)).toEqual({ sequence: 9 });
  });

  it('reads the plan again for the policy chosen and lists what would be delivered, in order, in words', async () => {
    const { user, platform } = await mount({ routes: withCandidates([SIGNAL, UPDATE, { ...SIGNAL, sequence: 14, name: 'payment.settled', outcome: 'NOT_REAPPLIED', reason: 'NOT_RETAINED' }]) });
    await open(user);
    await user.click(screen.getByRole('radio', { name: /After step charge/ }));
    await user.click(await screen.findByRole('checkbox', { name: /will run again/ }));
    await user.click(screen.getByRole('radio', { name: /Both, in the order they arrived/ }));
    await waitFor(() => expect(posts(platform, '/reset-previews')).toHaveLength(2));
    expect(JSON.parse(posts(platform, '/reset-previews')[1]!.init.body as string)).toEqual({ sequence: 5, reapply: 'ALL' });
    const plan = await screen.findByRole('region', { name: 'What the reset does' });
    const items = Array.from(plan.querySelectorAll('li')).map((item) => item.textContent);
    expect(items.slice(-3)).toEqual([
      'Signal payment.confirmed (event 9): will be tried',
      'Update balance.replace (event 12): will be tried',
      'Signal payment.settled (event 14): not delivered: its content is no longer kept',
    ]);
    expect(plan).toHaveTextContent('Delivered again, in this order:');
    // what was acknowledged is about the steps, and stays when the policy changes
    expect(screen.getByRole('checkbox', { name: /will run again/ })).toBeChecked();
  });

  it('says so when nothing came after the point', async () => {
    const { user } = await mount({ routes: withCandidates([]) });
    await open(user);
    await user.click(screen.getByRole('radio', { name: /After step ship/ }));
    await user.click(await screen.findByRole('radio', { name: /The signals the run received/ }));
    expect(await screen.findByText(/nothing came after this point/)).toBeVisible();
  });

  it('sends the policy with the reset and says what came of each before taking the person to the new run', async () => {
    const report: ReapplyEntry[] = [
      { ...SIGNAL, outcome: 'REAPPLIED' },
      { sequence: 16, kind: 'SIGNAL', name: 'payment.settled', outcome: 'REJECTED', reason: 'NOT_WAITING' },
      { ...UPDATE, outcome: 'NOT_REAPPLIED', reason: 'PREVIOUS_SIGNAL_NOT_DELIVERED' },
    ];
    const { user, platform } = await mount({ routes: withCandidates([SIGNAL], report) });
    await open(user);
    await user.click(screen.getByRole('radio', { name: /After step ship/ }));
    await user.click(screen.getByRole('radio', { name: /The signals the run received/ }));
    await screen.findByRole('region', { name: 'What the reset does' });
    await user.type(screen.getByLabelText('Reason'), REASON);
    await user.click(screen.getByRole('button', { name: 'Confirm: reset the run' }));
    const delivered = await screen.findByRole('region', { name: 'What was delivered again' });
    expect(JSON.parse(posts(platform, '/resets')[0]!.init.body as string)).toEqual({ sequence: 9, reason: REASON, acknowledgeReexecution: false, reapply: 'SIGNALS' });
    expect(Array.from(delivered.querySelectorAll('li')).map((item) => item.textContent)).toEqual([
      'Signal payment.confirmed (event 9): delivered',
      'Signal payment.settled (event 16): rejected: no step of the new run was waiting for it',
      'Update balance.replace (event 12): not delivered: an earlier signal was not delivered, and a later one would reach its wait',
    ]);
    // the person is not taken away before reading it
    expect(screen.getByTestId('where')).toHaveTextContent('/');
    expect(screen.getByRole('dialog')).toHaveTextContent('The reset was made. The new run starts from that point.');
    await user.click(screen.getByRole('button', { name: 'Open the new run' }));
    await waitFor(() => expect(screen.getByTestId('where')).toHaveTextContent('/runs/run-2'));
    expect(screen.queryByRole('region', { name: 'What was delivered again' })).not.toBeInTheDocument();
  });

  it('can be closed without going to the new run', async () => {
    const { user } = await mount({ routes: withCandidates([SIGNAL], [{ ...SIGNAL, outcome: 'REAPPLIED' }]) });
    await open(user);
    await user.click(screen.getByRole('radio', { name: /After step ship/ }));
    await user.click(screen.getByRole('radio', { name: /The signals the run received/ }));
    await screen.findByRole('region', { name: 'What the reset does' });
    await user.type(screen.getByLabelText('Reason'), REASON);
    await user.click(screen.getByRole('button', { name: 'Confirm: reset the run' }));
    await screen.findByRole('region', { name: 'What was delivered again' });
    await user.click(screen.getByRole('button', { name: 'Close' }));
    expect(screen.queryByRole('region', { name: 'What was delivered again' })).not.toBeInTheDocument();
    expect(screen.getByTestId('where')).toHaveTextContent('/');
  });

  it('goes straight to the new run when nothing was delivered again', async () => {
    const { user } = await mount({ routes: withCandidates([SIGNAL], []) });
    await open(user);
    await user.click(screen.getByRole('radio', { name: /After step ship/ }));
    await user.click(screen.getByRole('radio', { name: /The signals the run received/ }));
    await screen.findByRole('region', { name: 'What the reset does' });
    await user.type(screen.getByLabelText('Reason'), REASON);
    await user.click(screen.getByRole('button', { name: 'Confirm: reset the run' }));
    await waitFor(() => expect(screen.getByTestId('where')).toHaveTextContent('/runs/run-2'));
  });

  it('says what the platform answers when it does not serve the choice', async () => {
    const { user } = await mount({
      routes: {
        'POST /runs/run-1/reset-previews': (init: RequestInit) => {
          const body = JSON.parse(init.body as string) as { sequence: number; reapply?: string };
          if (body.reapply === undefined) return planAt(body.sequence);
          return json({ code: 'VALIDATION_FAILED', message: 'The reset cannot be made.', retryable: false, remediation: 'Ask for `reapply: NONE`, or ask the operator of the platform to enable reset reapply.', details: { reason: 'RESET_REAPPLY_NOT_AVAILABLE' } }, 422);
        },
      },
    });
    await open(user);
    await user.click(screen.getByRole('radio', { name: /After step ship/ }));
    await user.click(screen.getByRole('radio', { name: /Both, in the order they arrived/ }));
    expect(await screen.findByText(/ask the operator of the platform to enable reset reapply/)).toBeVisible();
    // going back to nothing reads the plan again and the dialog is usable
    await user.click(screen.getByRole('radio', { name: /Nothing: the new run does not receive them/ }));
    expect(await screen.findByRole('region', { name: 'What the reset does' })).toBeVisible();
    expect(screen.queryByText(/enable reset reapply/)).not.toBeInTheDocument();
  });

  it('words every outcome and every reason, and an unknown one as the platform wrote it', () => {
    const base = { sequence: 3, kind: 'SIGNAL', name: 'n' } as const;
    expect(entryLabel({ ...base, outcome: 'WILL_TRY' })).toBe('Signal n (event 3): will be tried');
    expect(entryLabel({ ...base, outcome: 'REAPPLIED' })).toBe('Signal n (event 3): delivered');
    expect(entryLabel({ ...base, outcome: 'REJECTED' })).toBe('Signal n (event 3): rejected');
    expect(entryLabel({ ...base, outcome: 'NOT_REAPPLIED' })).toBe('Signal n (event 3): not delivered');
    const reasons: [ReapplyEntry['reason'], string][] = [
      ['LEGACY_EVENT', 'it was recorded before the platform said what it was'],
      ['NOT_RETAINED', 'its content is no longer kept'],
      ['PREVIOUS_SIGNAL_NOT_DELIVERED', 'an earlier signal was not delivered, and a later one would reach its wait'],
      ['NOT_WAITING', 'no step of the new run was waiting for it'],
      ['UPDATE_REJECTED', 'the validator of the update declined it'],
      ['REVISION_CONFLICT', 'the state was not at the revision the update was applied to'],
      ['UPDATE_NOT_ACCEPTED', 'the declaration does not accept the update'],
    ];
    for (const [reason, words] of reasons) expect(entryLabel({ ...base, outcome: 'REJECTED', reason })).toBe(`Signal n (event 3): rejected: ${words}`);
    expect(entryLabel({ ...base, outcome: 'REJECTED', reason: 'SOMETHING_NEWER' })).toBe('Signal n (event 3): rejected: SOMETHING_NEWER');
    // an Update is named by its operation, and a long name is cut
    expect(entryLabel({ sequence: 4, kind: 'UPDATE', operation: 'balance.replace', outcome: 'REAPPLIED' })).toBe('Update balance.replace (event 4): delivered');
    expect(entryLabel({ ...base, name: 'x'.repeat(200), outcome: 'REAPPLIED' }).length).toBeLessThan(100);
    // a name that is missing is not invented
    expect(entryLabel({ sequence: 5, kind: 'UPDATE', outcome: 'REAPPLIED' })).toBe('Update ? (event 5): delivered');
    // a kind, an outcome and a reason a later platform adds are shown as written, and are not an error
    expect(entryLabel({ sequence: 6, kind: 'QUERY', operation: 'later.thing', outcome: 'SOMEDAY', reason: 'FUTURE' })).toBe('QUERY later.thing (event 6): SOMEDAY: FUTURE');
    expect(entryLabel({ sequence: 7, kind: 'QUERY', outcome: 'REAPPLIED' })).toBe('QUERY ? (event 7): delivered');
  });
});

describe('The report of a reset in the register', () => {
  it('lists what was delivered again under the reset that did it, and nothing under one that did not', async () => {
    const report: ReapplyEntry[] = [{ ...SIGNAL, outcome: 'REAPPLIED' }, { ...UPDATE, outcome: 'REJECTED', reason: 'REVISION_CONFLICT' }];
    const platform = fakePlatform({ 'GET /runs/run-1/resets': () => ({ items: [{ ...RESET, reapplyReport: report }, { ...RESET, resetId: 'reset-0', sequence: 3, reapplyReport: [] }] }) });
    const app = createApp({ fetch: platform.fetcher });
    await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions: ['run:read'] } as never));
    app.namespace.set('payments');
    renderApp(<ResetsMade runId="run-1" />, { app });
    const summary = await screen.findByText('Delivered again (2)');
    const section = screen.getByRole('region', { name: 'Resets made from this run' });
    expect(section.querySelectorAll('details')).toHaveLength(1);
    await userEvent.setup().click(summary);
    expect(section).toHaveTextContent('Signal payment.confirmed (event 9): delivered');
    expect(section).toHaveTextContent('Update balance.replace (event 12): rejected: the state was not at the revision the update was applied to');
  });
});
