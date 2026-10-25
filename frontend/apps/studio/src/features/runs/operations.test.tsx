import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { openRunPage, openStream } from '../../../test/runPage';
import { installRunsDom } from '../../../test/runsDom';
import { ADMIN, OPERATOR, VIEWER, changesOf, failure, graphOf, jsonResponse, node, runOf, sagaRun } from '../../../test/runs';
import type { RunPlatformInit } from '../../../test/runs';

let undo: () => void;
beforeAll(() => {
  undo = installRunsDom();
});
afterAll(() => undo());

const REASON = 'The warehouse API is back up';
const card = (id: string) => screen.getByTestId(`rf__node-${id}`);
const pick = (element: HTMLElement) => fireEvent.click(element);

/** The saga run, plus a plain step that waits for a person because nobody knows whether it happened. */
function withCharge(): Pick<RunPlatformInit, 'graph'> {
  const { nodes, edges } = sagaRun();
  nodes.push(node('charge', 'WAITING', { waitingOn: 'operator', attempt: 1, failureCode: 'OUTCOME_UNKNOWN' }));
  return { graph: graphOf(nodes, edges, { status: 'MANUAL_REVIEW' }) };
}

async function panelOf(id: string, permissions = OPERATOR, init: RunPlatformInit = {}) {
  const view = await openRunPage({ ...withCharge(), ...init }, permissions);
  await waitFor(() => card(id));
  pick(within(card(id)).getByText(id));
  const panel = await screen.findByRole('complementary', { name: `Details of ${id}` });
  return { ...view, panel };
}

const actions = (panel: HTMLElement, id: string) => within(within(panel).getByRole('list', { name: `Actions for ${id}` }));
const state = (button: HTMLElement) => (button.getAttribute('aria-disabled') === 'true' ? 'disabled' : 'enabled');

describe('which decisions are offered', () => {
  it('lets an operator retry a compensation that waits for a person, and says why the rest are not offered', async () => {
    const { panel } = await panelOf('reserve~c');
    const list = actions(panel, 'reserve~c');
    expect(state(list.getByRole('button', { name: 'Retry compensation' }))).toBe('enabled');
    expect(state(list.getByRole('button', { name: 'Retry step' }))).toBe('disabled');
    expect(list.getByText('This is a compensation: use Retry compensation.')).toBeInTheDocument();
    expect(state(list.getByRole('button', { name: 'Skip compensation' }))).toBe('disabled');
    expect(state(list.getByRole('button', { name: 'Resolve step' }))).toBe('disabled');
    expect(list.getAllByText('You need the saga:override permission to do this.')).toHaveLength(2);
  });

  it('says a reason that every action shares once, under the list, and describes each button with it', async () => {
    const { nodes, edges } = sagaRun();
    const finished = { run: runOf({ status: 'FAILED', finishedAt: '2026-10-10T10:05:00Z' }), graph: graphOf(nodes, edges, { status: 'FAILED' }) };
    const { panel } = await panelOf('reserve~c', ADMIN, finished);
    const buttons = actions(panel, 'reserve~c').getAllByRole('button');
    expect(buttons).toHaveLength(5);
    const sentences = within(panel).getAllByText('The run is finished.');
    expect(sentences).toHaveLength(1);
    for (const button of buttons) expect(button.getAttribute('aria-describedby')).toBe(sentences[0]?.id);
  });

  it('lets an administrator skip or resolve it as well', async () => {
    const { panel } = await panelOf('reserve~c', ADMIN);
    const list = actions(panel, 'reserve~c');
    for (const name of ['Retry compensation', 'Skip compensation', 'Resolve step']) expect(state(list.getByRole('button', { name }))).toBe('enabled');
    expect(state(list.getByRole('button', { name: 'Retry step' }))).toBe('disabled');
  });

  it('offers nothing for a step that is not waiting for a decision, and says so for each', async () => {
    const { panel } = await panelOf('pay', ADMIN);
    const list = actions(panel, 'pay');
    for (const name of ['Retry step', 'Retry compensation', 'Skip compensation', 'Resolve step']) expect(state(list.getByRole('button', { name }))).toBe('disabled');
    expect(list.getAllByText('This step is not waiting for a decision.').length).toBeGreaterThanOrEqual(2);
    expect(list.getByText('Only a compensation (an id ending in ~c) can be retried this way: use Retry step.')).toBeInTheDocument();
  });

  it('asks for saga:override to retry a step whose effect may already have happened', async () => {
    const { panel, unmount } = await panelOf('charge', OPERATOR);
    expect(state(actions(panel, 'charge').getByRole('button', { name: 'Retry step' }))).toBe('disabled');
    expect(actions(panel, 'charge').getByText(/may already have happened \(OUTCOME_UNKNOWN\)\. Retrying it needs the saga:override permission/)).toBeInTheDocument();
    unmount();
    const admin = await panelOf('charge', ADMIN);
    expect(state(actions(admin.panel, 'charge').getByRole('button', { name: 'Retry step' }))).toBe('enabled');
  });

  it('gives a viewer nothing to press on a step, with the permission each action lacks', async () => {
    const { panel } = await panelOf('reserve~c', VIEWER);
    const list = actions(panel, 'reserve~c');
    for (const button of list.getAllByRole('button')) expect(state(button)).toBe('disabled');
    // Retry step, retry compensation and holding a step all need run:operate.
    expect(list.getAllByText('You need the run:operate permission to do this.')).toHaveLength(3);
  });

  it('lists the run-level actions by the state of the run: a running run can be paused, not resumed', async () => {
    const { user } = await openRunPage({ run: runOf({ status: 'RUNNING' }), graph: graphOf(sagaRun().nodes, sagaRun().edges, { status: 'RUNNING' }) });
    await user.click(await screen.findByRole('button', { name: 'Run actions' }));
    const list = within(screen.getByRole('list', { name: 'Run actions' }));
    expect(state(list.getByRole('button', { name: 'Pause run' }))).toBe('enabled');
    expect(state(list.getByRole('button', { name: 'Cancel run' }))).toBe('enabled');
    expect(state(list.getByRole('button', { name: 'Resume run' }))).toBe('disabled');
    expect(list.getByText('The run is not paused.')).toBeInTheDocument();
  });

  it('can resume a paused run and not pause it again', async () => {
    const { user } = await openRunPage({ run: runOf({ status: 'PAUSED', paused: true }), graph: graphOf(sagaRun().nodes, sagaRun().edges, { status: 'PAUSED' }) });
    await user.click(await screen.findByRole('button', { name: 'Run actions' }));
    const list = within(screen.getByRole('list', { name: 'Run actions' }));
    expect(state(list.getByRole('button', { name: 'Resume run' }))).toBe('enabled');
    expect(state(list.getByRole('button', { name: 'Pause run' }))).toBe('disabled');
    expect(list.getByText('The run is already paused.')).toBeInTheDocument();
  });

  it('offers to run it again only once it has ended', async () => {
    const { user, unmount } = await openRunPage({ run: runOf({ status: 'RUNNING' }) }, [...ADMIN, 'run:start']);
    await user.click(await screen.findByRole('button', { name: 'Run actions' }));
    const running = within(screen.getByRole('list', { name: 'Run actions' }));
    expect(state(running.getByRole('button', { name: 'Run again as new' }))).toBe('disabled');
    expect(running.getByText('The run has not ended: starting it again would do its work twice.')).toBeInTheDocument();
    unmount();
    const { nodes, edges } = sagaRun();
    const over = await openRunPage({ run: runOf({ status: 'FAILED', finishedAt: '2026-10-10T10:05:00Z' }), graph: graphOf(nodes, edges, { status: 'FAILED' }) }, [...ADMIN, 'run:start']);
    await over.user.click(await screen.findByRole('button', { name: 'Run actions' }));
    expect(state(within(screen.getByRole('list', { name: 'Run actions' })).getByRole('button', { name: 'Run again as new' }))).toBe('enabled');
  });

  it('refuses everything on a run that is over', async () => {
    const { nodes, edges } = sagaRun();
    const { user } = await openRunPage({ run: runOf({ status: 'FAILED', finishedAt: '2026-10-10T10:05:00Z' }), graph: graphOf(nodes, edges, { status: 'FAILED' }) }, ADMIN);
    await user.click(await screen.findByRole('button', { name: 'Run actions' }));
    const list = within(screen.getByRole('list', { name: 'Run actions' }));
    for (const name of ['Pause run', 'Resume run', 'Cancel run']) expect(state(list.getByRole('button', { name }))).toBe('disabled');
    expect(list.getAllByText('The run is finished.')).toHaveLength(3);
  });

  it('does not open a dialog for an action that is not available', async () => {
    const { panel, user } = await panelOf('reserve~c');
    await user.click(actions(panel, 'reserve~c').getByRole('button', { name: 'Skip compensation' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});

describe('deciding', () => {
  async function dialogFor(action: string, id = 'reserve~c', permissions = ADMIN, init: RunPlatformInit = {}) {
    const view = await panelOf(id, permissions, init);
    await view.user.click(actions(view.panel, id).getByRole('button', { name: action }));
    const dialog = await screen.findByRole('dialog');
    return { ...view, dialog, within: within(dialog) };
  }
  const reasonBox = (dialog: HTMLElement) => within(dialog).getByRole('textbox', { name: 'Reason' });

  it('says in plain words what the platform will do, and that the decision is recorded with the name and the reason', async () => {
    const { dialog } = await dialogFor('Retry compensation');
    expect(within(dialog).getByRole('heading', { name: 'Retry compensation: reserve~c' })).toBeInTheDocument();
    expect(within(dialog).getByText('Asks for another attempt at undoing step reserve~c; the earlier attempts stay in the journal.')).toBeInTheDocument();
    expect(within(dialog).getByText('This decision is recorded with your name and reason.')).toBeInTheDocument();
  });

  it('says what skipping, resolving and cancelling do', async () => {
    const skip = await dialogFor('Skip compensation');
    expect(within(skip.dialog).getByText(/Gives up on undoing step reserve~c\. The saga ends partly undone and cannot be compensated again\./)).toBeInTheDocument();
    skip.unmount();
    const resolve = await dialogFor('Resolve step');
    expect(within(resolve.dialog).getByText(/Declares that step reserve~c did or did not happen\. This overrides what the platform knows\./)).toBeInTheDocument();
    resolve.unmount();
    const cancel = await openRunPage({}, ADMIN);
    await cancel.user.click(await screen.findByRole('button', { name: 'Run actions' }));
    await cancel.user.click(screen.getByRole('button', { name: 'Cancel run' }));
    expect(within(await screen.findByRole('dialog')).getByText(/Stops the run\. Completed steps of a saga are compensated according to its cancel policy/)).toBeInTheDocument();
  });

  it('asks for a reason of 8 to 2000 characters, counted after trimming, and sends nothing without one', async () => {
    const { dialog, user, platform } = await dialogFor('Retry compensation');
    const confirm = within(dialog).getByRole('button', { name: 'Confirm: Retry compensation' });
    await user.click(confirm);
    expect(await within(dialog).findByText(/Write at least 8 characters/)).toBeInTheDocument();
    expect(reasonBox(dialog)).toHaveAttribute('aria-invalid', 'true');
    expect(reasonBox(dialog)).toHaveFocus();
    await user.type(reasonBox(dialog), '   short   ');
    await user.click(confirm);
    expect(within(dialog).getByText(/Write at least 8 characters/)).toBeInTheDocument();
    expect(within(dialog).getByText(/5 \/ 2000/)).toBeInTheDocument();
    expect(platform.operations).toHaveLength(0);
    await user.clear(reasonBox(dialog));
    await user.paste('x'.repeat(2001));
    await user.click(confirm);
    expect(within(dialog).getByText(/At most 2000 characters \(1 too many\)/)).toBeInTheDocument();
    expect(platform.operations).toHaveLength(0);
  });

  it('keeps the reason of a step decision to one line of plain text, as the platform requires', async () => {
    const { dialog, user, platform } = await dialogFor('Retry compensation');
    await user.type(reasonBox(dialog), 'first line{Enter}second line');
    await user.click(within(dialog).getByRole('button', { name: 'Confirm: Retry compensation' }));
    expect(within(dialog).getByText('Write the reason as plain text on one line.')).toBeInTheDocument();
    expect(platform.operations).toHaveLength(0);
  });

  it('retries a compensation: the request has the action, the step, the reason and the version of the run the person saw', async () => {
    const { dialog, user, platform } = await dialogFor('Retry compensation');
    await user.type(reasonBox(dialog), `  ${REASON}  `);
    await user.click(within(dialog).getByRole('button', { name: 'Confirm: Retry compensation' }));
    await waitFor(() => expect(platform.operations).toHaveLength(1));
    const [sent] = platform.operations;
    expect(sent?.body).toEqual({ action: 'RETRY_COMPENSATION', stepId: 'reserve~c', reason: REASON, expectedRunVersion: 10 });
    expect(sent?.key).toMatch(/^[0-9a-f-]{36}$/);
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(await screen.findByText(/Retry compensation requested for reserve~c\./)).toBeInTheDocument();
  });

  it('holds a step that has not started for a person, and says what that does', async () => {
    const { nodes, edges } = sagaRun();
    nodes.push(node('ship', 'READY', { attempt: 0 }));
    const { panel, user, platform } = await panelOf('ship', OPERATOR, { graph: graphOf(nodes, edges, { status: 'RUNNING' }), run: runOf({ status: 'RUNNING' }) });
    await user.click(actions(panel, 'ship').getByRole('button', { name: 'Mark for manual review' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText(/Holds step ship for a person: no worker is given it and nothing more happens to it until someone retries or settles it/)).toBeInTheDocument();
    await user.type(reasonBox(dialog), 'the provider asked us to wait');
    await user.click(within(dialog).getByRole('button', { name: 'Confirm: Mark for manual review' }));
    await waitFor(() => expect(platform.operations).toHaveLength(1));
    expect(platform.operations[0]?.body).toEqual({ action: 'MARK_MANUAL_REVIEW', stepId: 'ship', reason: 'the provider asked us to wait', expectedRunVersion: 10 });
  });

  it('retries a step that waits for its retry now, and says so', async () => {
    const { nodes, edges } = sagaRun();
    nodes.push(node('ship', 'RETRY_WAIT', { attempt: 1, failureCode: 'PAYMENT_DECLINED' }));
    const { panel, user, platform } = await panelOf('ship', OPERATOR, { graph: graphOf(nodes, edges, { status: 'RUNNING' }), run: runOf({ status: 'RUNNING' }) });
    await user.click(actions(panel, 'ship').getByRole('button', { name: 'Retry step' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText(/Starts the next attempt of step ship now, without waiting for the delay of its retry/)).toBeInTheDocument();
    await user.type(reasonBox(dialog), 'the provider is back up');
    await user.click(within(dialog).getByRole('button', { name: 'Confirm: Retry step' }));
    await waitFor(() => expect(platform.operations).toHaveLength(1));
    expect(platform.operations[0]?.body).toMatchObject({ action: 'RETRY_STEP', stepId: 'ship' });
  });

  it('starts a run again and takes the person to the new one', async () => {
    const { nodes, edges } = sagaRun();
    const NEW_RUN = '019b5d6e-0000-7000-8000-0000000000ff';
    const { user, platform } = await openRunPage(
      {
        run: runOf({ status: 'FAILED', finishedAt: '2026-10-10T10:05:00Z' }),
        graph: graphOf(nodes, edges, { status: 'FAILED' }),
        operate: () => jsonResponse({ operationId: '019b5d6e-0000-7000-8000-0000000000cc', accepted: true, runStatus: 'FAILED', runVersion: 10, newRunId: NEW_RUN }, 202),
      },
      [...ADMIN, 'run:start'],
    );
    await user.click(await screen.findByRole('button', { name: 'Run actions' }));
    await user.click(within(screen.getByRole('list', { name: 'Run actions' })).getByRole('button', { name: 'Run again as new' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText(/Starts a new run of the same version of this workflow with the same input\. This run stays as it is/)).toBeInTheDocument();
    await user.type(reasonBox(dialog), 'the provider fixed the bug');
    await user.click(within(dialog).getByRole('button', { name: 'Confirm: Run again as new' }));
    await waitFor(() => expect(platform.operations).toHaveLength(1));
    expect(platform.operations[0]?.body).toMatchObject({ action: 'RERUN_AS_NEW', reason: 'the provider fixed the bug' });
    expect(platform.operations[0]?.body).not.toHaveProperty('stepId');
    // The page went to the new run: it asks the platform for it.
    await waitFor(() => expect(platform.calls.some((call) => call.url.includes(NEW_RUN))).toBe(true));
  });

  it('does not edit the graph itself: the card changes when the platform says so', async () => {
    const { dialog, user, platform } = await dialogFor('Retry compensation');
    await user.type(reasonBox(dialog), REASON);
    await user.click(within(dialog).getByRole('button', { name: 'Confirm: Retry compensation' }));
    await waitFor(() => expect(platform.operations).toHaveLength(1));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    // The operation was accepted, and the graph still says what the platform last said.
    expect(within(card('reserve~c')).getByText('Needs a person')).toBeInTheDocument();
    expect(within(card('reserve~c')).queryByText('Running')).not.toBeInTheDocument();
    // The effect arrives the way every change does.
    const pipe = await openStream(platform);
    pipe.send(changesOf(10, 12, [node('reserve~c', 'RUNNING', { kind: 'COMPENSATION', forward: 'reserve', attempt: 3 })], { events: [] }));
    await waitFor(() => expect(within(card('reserve~c')).getByText('Running')).toBeInTheDocument());
    expect(within(card('reserve~c')).queryByText('Needs a person')).not.toBeInTheDocument();
  });

  it('asks for the step id to be typed back to skip a compensation, and sends nothing until it is', async () => {
    const { dialog, user, platform } = await dialogFor('Skip compensation');
    await user.type(reasonBox(dialog), REASON);
    await user.type(within(dialog).getByRole('textbox', { name: 'Type the step id to confirm' }), 'reserve');
    await user.click(within(dialog).getByRole('button', { name: 'Confirm: Skip compensation' }));
    expect(await within(dialog).findByText('Type the step id exactly as shown to confirm.')).toBeInTheDocument();
    expect(platform.operations).toHaveLength(0);
    await user.type(within(dialog).getByRole('textbox', { name: 'Type the step id to confirm' }), '~c');
    await user.click(within(dialog).getByRole('button', { name: 'Confirm: Skip compensation' }));
    await waitFor(() => expect(platform.operations).toHaveLength(1));
    expect(platform.operations[0]?.body).toEqual({ action: 'SKIP_COMPENSATION', stepId: 'reserve~c', reason: REASON, expectedRunVersion: 10 });
  });

  it('warns that a step whose effect may already have happened will happen twice, and asks for its id to retry it', async () => {
    const { panel, platform, user } = await panelOf('charge', ADMIN);
    await user.click(actions(panel, 'charge').getByRole('button', { name: 'Retry step' }));
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent(/may already have happened \(OUTCOME_UNKNOWN\): if it did, it will happen twice/);
    await user.type(within(dialog).getByRole('textbox', { name: 'Reason' }), REASON);
    await user.click(within(dialog).getByRole('button', { name: 'Confirm: Retry step' }));
    expect(within(dialog).getByText('Type the step id exactly as shown to confirm.')).toBeInTheDocument();
    expect(platform.operations).toHaveLength(0);
    await user.type(within(dialog).getByRole('textbox', { name: 'Type the step id to confirm' }), 'charge');
    await user.click(within(dialog).getByRole('button', { name: 'Confirm: Retry step' }));
    await waitFor(() => expect(platform.operations).toHaveLength(1));
    expect(platform.operations[0]?.body).toMatchObject({ action: 'RETRY_STEP', stepId: 'charge' });
  });

  it('asks what happened to the step to resolve it, and the error code only if it did not happen', async () => {
    const { dialog, user, platform } = await dialogFor('Resolve step');
    await user.type(reasonBox(dialog), REASON);
    await user.type(within(dialog).getByRole('textbox', { name: 'Type the step id to confirm' }), 'reserve~c');
    await user.click(within(dialog).getByRole('button', { name: 'Confirm: Resolve step' }));
    expect(await within(dialog).findByText('Choose what happened to the step.')).toBeInTheDocument();
    expect(within(dialog).queryByRole('textbox', { name: /Error code/ })).not.toBeInTheDocument();
    expect(platform.operations).toHaveLength(0);

    await user.click(within(dialog).getByRole('radio', { name: /It did not happen/ }));
    const code = within(dialog).getByRole('textbox', { name: 'Error code (optional)' });
    await user.type(code, 'not a code!');
    expect(within(dialog).getByText(/Use letters, digits and _ \. : - only/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole('button', { name: 'Confirm: Resolve step' }));
    expect(platform.operations).toHaveLength(0);
    await user.clear(code);
    await user.type(code, 'PAYMENT_NEVER_SENT');
    await user.click(within(dialog).getByRole('button', { name: 'Confirm: Resolve step' }));
    await waitFor(() => expect(platform.operations).toHaveLength(1));
    expect(platform.operations[0]?.body).toEqual({ action: 'RESOLVE_STEP', stepId: 'reserve~c', outcome: 'FAILED', errorCode: 'PAYMENT_NEVER_SENT', reason: REASON, expectedRunVersion: 10 });
  });

  it('resolving as "it happened" sends no error code, even if one was typed before the choice changed', async () => {
    const { dialog, user, platform } = await dialogFor('Resolve step');
    await user.type(reasonBox(dialog), REASON);
    await user.type(within(dialog).getByRole('textbox', { name: 'Type the step id to confirm' }), 'reserve~c');
    await user.click(within(dialog).getByRole('radio', { name: /It did not happen/ }));
    await user.type(within(dialog).getByRole('textbox', { name: 'Error code (optional)' }), 'SOME_CODE');
    await user.click(within(dialog).getByRole('radio', { name: /It happened/ }));
    await user.click(within(dialog).getByRole('button', { name: 'Confirm: Resolve step' }));
    await waitFor(() => expect(platform.operations).toHaveLength(1));
    expect(platform.operations[0]?.body).toEqual({ action: 'RESOLVE_STEP', stepId: 'reserve~c', outcome: 'SUCCEEDED', reason: REASON, expectedRunVersion: 10 });
  });

  it('cancels the run: the request has no step, and the reason may have several lines', async () => {
    const { user, platform } = await openRunPage({}, OPERATOR);
    await user.click(await screen.findByRole('button', { name: 'Run actions' }));
    await user.click(screen.getByRole('button', { name: 'Cancel run' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByRole('heading', { name: 'Cancel run' })).toBeInTheDocument();
    await user.type(reasonBox(dialog), 'Customer cancelled{Enter}ticket 4411');
    await user.click(within(dialog).getByRole('button', { name: 'Confirm: Cancel run' }));
    await waitFor(() => expect(platform.operations).toHaveLength(1));
    expect(platform.operations[0]?.body).toEqual({ action: 'CANCEL', reason: 'Customer cancelled\nticket 4411', expectedRunVersion: 10 });
  });

  it('pauses and resumes the run', async () => {
    const paused = await openRunPage({ run: runOf({ status: 'RUNNING' }), graph: graphOf(sagaRun().nodes, sagaRun().edges, { status: 'RUNNING' }) });
    await paused.user.click(await screen.findByRole('button', { name: 'Run actions' }));
    await paused.user.click(screen.getByRole('button', { name: 'Pause run' }));
    let dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText(/no new step is started or handed to a worker until someone resumes it/)).toBeInTheDocument();
    await paused.user.type(reasonBox(dialog), 'Deploying the worker');
    await paused.user.click(within(dialog).getByRole('button', { name: 'Confirm: Pause run' }));
    await waitFor(() => expect(paused.platform.operations).toHaveLength(1));
    expect(paused.platform.operations[0]?.body).toEqual({ action: 'PAUSE', reason: 'Deploying the worker', expectedRunVersion: 10 });
    paused.unmount();

    const resumed = await openRunPage({ run: runOf({ status: 'PAUSED', paused: true }), graph: graphOf(sagaRun().nodes, sagaRun().edges, { status: 'PAUSED' }) });
    await resumed.user.click(await screen.findByRole('button', { name: 'Run actions' }));
    await resumed.user.click(screen.getByRole('button', { name: 'Resume run' }));
    dialog = await screen.findByRole('dialog');
    await resumed.user.type(reasonBox(dialog), 'Deploy finished');
    await resumed.user.click(within(dialog).getByRole('button', { name: 'Confirm: Resume run' }));
    await waitFor(() => expect(resumed.platform.operations).toHaveLength(1));
    expect(resumed.platform.operations[0]?.body).toEqual({ action: 'RESUME', reason: 'Deploy finished', expectedRunVersion: 10 });
  });

  it('works from the actions menu of a row of the table, with the keyboard', async () => {
    const { user, platform } = await openRunPage(withCharge(), ADMIN);
    await waitFor(() => card('pay'));
    await user.click(screen.getByRole('tab', { name: 'Table' }));
    const trigger = await screen.findByRole('button', { name: 'Actions for reserve~c' });
    trigger.focus();
    await user.keyboard('{Enter}');
    await user.click(within(screen.getByRole('list', { name: 'Actions for reserve~c' })).getByRole('button', { name: 'Retry compensation' }));
    const dialog = await screen.findByRole('dialog');
    await user.type(reasonBox(dialog), REASON);
    // From the reason to Cancel, then to Confirm, and press it.
    await user.tab();
    await user.tab();
    expect(within(dialog).getByRole('button', { name: 'Confirm: Retry compensation' })).toHaveFocus();
    await user.keyboard('{Enter}');
    await waitFor(() => expect(platform.operations).toHaveLength(1));
    expect(platform.operations[0]?.body).toMatchObject({ action: 'RETRY_COMPENSATION', stepId: 'reserve~c' });
    // Focus goes back to where the decision started.
    await waitFor(() => expect(screen.getByRole('button', { name: 'Actions for reserve~c' })).toHaveFocus());
  });

  it('returns focus to the button that opened the dialog when it is cancelled', async () => {
    const { dialog, user, panel } = await dialogFor('Retry compensation');
    const opener = actions(panel, 'reserve~c').getByRole('button', { name: 'Retry compensation' });
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    await waitFor(() => expect(opener).toHaveFocus());
  });
});

describe('when the platform does not take the decision', () => {
  async function submit(init: RunPlatformInit, action = 'Retry compensation', typeId = false) {
    const view = await panelOf('reserve~c', ADMIN, init);
    await view.user.click(actions(view.panel, 'reserve~c').getByRole('button', { name: action }));
    const dialog = await screen.findByRole('dialog');
    await view.user.type(within(dialog).getByRole('textbox', { name: 'Reason' }), REASON);
    if (typeId) await view.user.type(within(dialog).getByRole('textbox', { name: 'Type the step id to confirm' }), 'reserve~c');
    await view.user.click(within(dialog).getByRole('button', { name: `Confirm: ${action}` }));
    return { ...view, dialog };
  }

  it('shows a refusal for lack of permission as that, with the trace id, and keeps the dialog open', async () => {
    const { dialog, platform } = await submit({ operate: () => failure(403, 'FORBIDDEN', 'You are not allowed to do this.') });
    const alert = await within(dialog).findByRole('alert');
    expect(alert).toHaveTextContent('Retry compensation failed.');
    expect(alert).toHaveTextContent('You are not allowed to do this.');
    expect(alert).toHaveTextContent('b'.repeat(32));
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(platform.operations).toHaveLength(1);
  });

  it('shows a validation error as the platform worded it', async () => {
    const { dialog } = await submit({ operate: () => failure(422, 'VALIDATION_FAILED', 'errorCode is not a valid failure code') });
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('errorCode is not a valid failure code');
  });

  it('says "The run changed while you were deciding" on a 409 about the version, refreshes on request, and lets the person confirm again', async () => {
    let n = 0;
    const view = await submit({
      operate: () => {
        n += 1;
        return n === 1 ? failure(409, 'CONFLICT', 'the draft was changed since it was read; fetch it again and retry', { reason: 'REVISION_CONFLICT', currentRevision: 14 }) : jsonResponse({ operationId: '019b5d6e-0000-7000-8000-0000000000cc', accepted: true, runStatus: 'MANUAL_REVIEW', runVersion: 15 }, 202);
      },
    });
    const alert = await within(view.dialog).findByRole('alert');
    expect(alert).toHaveTextContent('The run changed while you were deciding.');
    // The platform's wording is about drafts; the Studio's is about the run.
    expect(alert).not.toHaveTextContent('draft');
    expect(alert).toHaveTextContent('b'.repeat(32));
    // The run has moved on to 14; refreshing reads it again.
    view.platform.state.graph = { ...view.platform.state.graph, projectionSeq: 14 };
    await view.user.click(within(view.dialog).getByRole('button', { name: 'Refresh and decide again' }));
    await waitFor(() => expect(view.platform.state.graphReads).toBe(2));
    await waitFor(() => expect(within(view.dialog).queryByRole('alert')).not.toBeInTheDocument());
    await view.user.click(within(view.dialog).getByRole('button', { name: 'Confirm: Retry compensation' }));
    await waitFor(() => expect(view.platform.operations).toHaveLength(2));
    expect(view.platform.operations[1]?.body).toMatchObject({ expectedRunVersion: 14 });
    // The request changed (another version), so it is another request: another key.
    expect(view.platform.operations[1]?.key).not.toBe(view.platform.operations[0]?.key);
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });

  it('says another operator may have been first when the step is no longer waiting for a decision', async () => {
    const { dialog } = await submit({ operate: () => failure(409, 'CONFLICT', 'the step is not in a state that allows this', { reason: 'STATE_CONFLICT' }) });
    const alert = await within(dialog).findByRole('alert');
    expect(alert).toHaveTextContent('the step is not in a state that allows this. Another operator may have decided first');
    expect(within(dialog).getByRole('button', { name: 'Refresh and decide again' })).toBeInTheDocument();
  });

  it('sends the same request with the same idempotency key when the person tries again after a failure to reach the platform, and a new key when something changed', async () => {
    let n = 0;
    const view = await submit({
      operate: () => {
        n += 1;
        return n <= 2 ? failure(503, 'UNAVAILABLE', 'The platform is not available right now.') : jsonResponse({ operationId: '019b5d6e-0000-7000-8000-0000000000cc', accepted: true }, 202);
      },
    });
    await within(view.dialog).findByRole('alert');
    await view.user.click(within(view.dialog).getByRole('button', { name: 'Try again' }));
    await waitFor(() => expect(view.platform.operations).toHaveLength(2));
    expect(view.platform.operations[1]?.key).toBe(view.platform.operations[0]?.key);
    expect(view.platform.operations[1]?.body).toEqual(view.platform.operations[0]?.body);
    // The person changes the reason: that is not the same request any more.
    await within(view.dialog).findByRole('alert');
    await view.user.type(within(view.dialog).getByRole('textbox', { name: 'Reason' }), ' and the queue is empty');
    await view.user.click(within(view.dialog).getByRole('button', { name: 'Confirm: Retry compensation' }));
    await waitFor(() => expect(view.platform.operations).toHaveLength(3));
    expect(view.platform.operations[2]?.key).not.toBe(view.platform.operations[0]?.key);
    expect(view.platform.operations[2]?.body).toMatchObject({ reason: `${REASON} and the queue is empty` });
  });
});

describe('a decision that was not answered', () => {
  it('is sent again as it was, the version of the run included, even if the run moved on meanwhile', async () => {
    const view = await panelOf('reserve~c', ADMIN, {
      operate: (() => {
        let n = 0;
        return () => {
          n += 1;
          return n === 1 ? failure(503, 'UNAVAILABLE', 'The platform is not available right now.') : jsonResponse({ operationId: '019b5d6e-0000-7000-8000-0000000000cc', accepted: true }, 202);
        };
      })(),
    });
    await view.user.click(actions(view.panel, 'reserve~c').getByRole('button', { name: 'Retry compensation' }));
    const dialog = await screen.findByRole('dialog');
    await view.user.type(within(dialog).getByRole('textbox', { name: 'Reason' }), REASON);
    await view.user.click(within(dialog).getByRole('button', { name: 'Confirm: Retry compensation' }));
    await within(dialog).findByRole('alert');
    expect(view.platform.operations[0]?.body).toMatchObject({ expectedRunVersion: 10 });

    // Something unrelated happens to the run: its version is now 13. The first request may have been done.
    const pipe = await openStream(view.platform);
    await act(async () => {
      pipe.send(changesOf(10, 13, [node('notify', 'READY')]));
      await Promise.resolve();
    });
    await waitFor(() => expect(within(card('notify')).getByText('Ready')).toBeInTheDocument());

    await view.user.click(within(dialog).getByRole('button', { name: 'Try again' }));
    await waitFor(() => expect(view.platform.operations).toHaveLength(2));
    // The same request: the platform can answer it as it answered the first, instead of finding a conflict with itself.
    expect(view.platform.operations[1]?.key).toBe(view.platform.operations[0]?.key);
    expect(view.platform.operations[1]?.body).toEqual(view.platform.operations[0]?.body);
  });
});

describe('when the run moves while the person decides', () => {
  async function decideWhile(change: (pipe: Awaited<ReturnType<typeof openStream>>) => void) {
    const view = await panelOf('reserve~c', ADMIN);
    await view.user.click(actions(view.panel, 'reserve~c').getByRole('button', { name: 'Retry compensation' }));
    const dialog = await screen.findByRole('dialog');
    await view.user.type(within(dialog).getByRole('textbox', { name: 'Reason' }), REASON);
    const pipe = await openStream(view.platform);
    await act(async () => {
      change(pipe);
      await Promise.resolve();
    });
    return { ...view, dialog };
  }

  it('sends the latest version of the run when something unrelated happened meanwhile', async () => {
    const view = await decideWhile((pipe) => pipe.send(changesOf(10, 13, [node('notify', 'READY')])));
    await waitFor(() => expect(within(card('notify')).getByText('Ready')).toBeInTheDocument());
    await view.user.click(within(view.dialog).getByRole('button', { name: 'Confirm: Retry compensation' }));
    await waitFor(() => expect(view.platform.operations).toHaveLength(1));
    expect(view.platform.operations[0]?.body).toMatchObject({ expectedRunVersion: 13 });
  });

  it('does not send the decision when the step it is about changed, and asks the person to look at the change first', async () => {
    const view = await decideWhile((pipe) => pipe.send(changesOf(10, 12, [node('reserve~c', 'RUNNING', { kind: 'COMPENSATION', forward: 'reserve', attempt: 3 })])));
    expect(await within(view.dialog).findByText('The run changed while you were deciding.')).toBeInTheDocument();
    await view.user.click(within(view.dialog).getByRole('button', { name: 'Confirm: Retry compensation' }));
    expect(view.platform.operations).toHaveLength(0);
    await view.user.click(within(view.dialog).getByRole('button', { name: 'Review the change' }));
    // After looking, it says what is now true: there is nothing to decide.
    expect(await within(view.dialog).findByText('This is not available any more.')).toBeInTheDocument();
    expect(within(view.dialog).getByText('This step is not waiting for a decision.')).toBeInTheDocument();
    await view.user.click(within(view.dialog).getByRole('button', { name: 'Confirm: Retry compensation' }));
    expect(view.platform.operations).toHaveLength(0);
  });
});
