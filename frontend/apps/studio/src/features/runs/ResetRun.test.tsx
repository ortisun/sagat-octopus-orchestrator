import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useLocation } from 'react-router';
import { describe, expect, it } from 'vitest';
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

function json(body: unknown, status: number) {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function Where() {
  return <p data-testid="where">{useLocation().pathname}</p>;
}

async function mount({ permissions = ['run:operate', 'run:start', 'run:read'], status = 'SUCCEEDED', businessId, routes = {}, holdPreview = false }: { permissions?: string[]; status?: string; businessId?: string; routes?: Routes; holdPreview?: boolean } = {}) {
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
      <ResetRun runId="run-1" status={status} businessId={businessId ?? null} />
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

/** The requests that changed something, in order. */
function posts(platform: Awaited<ReturnType<typeof mount>>['platform'], suffix: string) {
  return platform.calls.filter((call) => call.method === 'POST' && call.url.split('?')[0]!.endsWith(suffix));
}

describe('Reset to a point', () => {
  it('is offered only to someone who may operate and start runs, and only for a run that ended', async () => {
    for (const permissions of [['run:operate'], ['run:start'], ['run:read']]) {
      const { unmount } = await mount({ permissions });
      expect(screen.queryByRole('button', { name: 'Reset to a point…' })).not.toBeInTheDocument();
      unmount();
    }
    const { unmount } = await mount({ status: 'RUNNING' });
    expect(screen.queryByRole('button', { name: 'Reset to a point…' })).not.toBeInTheDocument();
    unmount();
    await mount();
    expect(screen.getByRole('button', { name: 'Reset to a point…' })).toBeVisible();
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
    expect(screen.getByRole('checkbox', { name: /will run again/ })).not.toBeChecked();
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
