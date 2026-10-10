import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { mountEditor, putsOf, settle } from '../../../test/workflowsHarness';

const row = (id: string) => screen.getByTestId(`row-${id}`);
const leadsTo = (id: string) => within(row(id)).getAllByRole('cell')[2]!.textContent;

async function opened(options: Parameters<typeof mountEditor>[0] = {}) {
  const mounted = await mountEditor({ view: 'list', ...options });
  await screen.findByRole('heading', { level: 1, name: 'order-flow' });
  return mounted;
}

describe('opening a draft', () => {
  it('shows the draft of the workflow with its name, its nodes and the state of saving', async () => {
    await opened();
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('order-flow');
    expect(screen.getByText('Not published')).toBeInTheDocument();
    expect(screen.getByTestId('save-state')).toHaveTextContent('Saved · revision 1');
    expect(screen.getAllByRole('row')).toHaveLength(4);
    expect(row('check')).toHaveTextContent('Condition');
    expect(leadsTo('check')).toBe('If true → pay; If false → done');
    expect(row('check')).toHaveTextContent('First step');
  });

  it('shows the active version when the workflow is published', async () => {
    await opened({ state: { activeVersion: 4 } });
    expect(screen.getByText('Active version 4')).toBeInTheDocument();
  });

  it('says so when the workflow does not exist, with the trace id and a way back', async () => {
    await mountEditor({
      behaviour: {
        get: () =>
          new Response(JSON.stringify({ code: 'NOT_FOUND', message: 'There is no such workflow.', traceId: 'a'.repeat(32) }), {
            status: 404,
            headers: { 'content-type': 'application/json' },
          }),
      },
    });
    expect(await screen.findByRole('alert')).toHaveTextContent('There is no such workflow.');
    expect(screen.getByRole('alert')).toHaveTextContent('a'.repeat(32));
    expect(screen.getByRole('link', { name: 'Back to the workflows' })).toHaveAttribute('href', '/workflows');
    expect(screen.queryByRole('button', { name: 'Try again' })).not.toBeInTheDocument();
  });

  it('keeps what it cannot read of the draft and says so, instead of losing it on the next save', async () => {
    const { platform, user } = await opened({
      state: {
        ir: {
          ...(await import('../../../test/workflowsHarness')).sampleIr(),
          nodes: [
            { id: 'done', type: 'end' },
            { type: 'end', note: 'no id' },
          ],
        },
      },
    });
    expect(screen.getByText(/1 entry of "nodes" in this draft cannot be read/)).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Add node' }));
    await user.click(await screen.findByRole('button', { name: 'Timer' }));
    await waitFor(() => expect(putsOf(platform)).toHaveLength(1));
    expect(putsOf(platform)[0]?.body.ir['nodes']).toEqual([
      { id: 'done', type: 'end' },
      { id: 'timer_1', type: 'timer', durationMs: 60_000 },
      { type: 'end', note: 'no id' },
    ]);
  });
});

describe('adding nodes', () => {
  it('adds a node with the keyboard: Alt+N opens the menu of kinds and choosing one adds it and selects it', async () => {
    const { user } = await opened();
    await user.keyboard('{Alt>}n{/Alt}');
    const dialog = await screen.findByRole('dialog', { name: 'Add a node' });
    expect(within(dialog).getAllByRole('button')).toHaveLength(12);
    await user.click(within(dialog).getByRole('button', { name: 'Timer' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Add a node' })).not.toBeInTheDocument());
    expect(row('timer_1')).toBeInTheDocument();
    expect(screen.getByRole('heading', { level: 2, name: 'Timer: timer_1' })).toBeInTheDocument();
    expect(screen.getByTestId('edit-notice')).toHaveTextContent('Added Timer "timer_1"');
  });

  it('clears what it said about a change once that is no longer news, and not before', async () => {
    const { user } = await opened({ timings: { validateMs: 15, autosaveMs: 60_000, noticeMs: 250 } });
    await user.keyboard('{Alt>}n{/Alt}');
    await user.click(within(await screen.findByRole('dialog', { name: 'Add a node' })).getByRole('button', { name: 'Timer' }));
    const notice = screen.getByTestId('edit-notice');
    await waitFor(() => expect(notice).toHaveTextContent('Added Timer "timer_1"'));
    await waitFor(() => expect(notice).toHaveTextContent(/^$/), { timeout: 2000 });
  });

  it('adds a node after the selected one, in its first free way out', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    await user.click(screen.getByRole('button', { name: 'Add node' }));
    const dialog = await screen.findByRole('dialog', { name: 'Add a node' });
    expect(dialog).toHaveTextContent('after "pay"');
    await user.click(within(dialog).getByRole('button', { name: 'Timer' }));
    // `pay` already leads to `done`: the new node goes between them.
    await waitFor(() => expect(leadsTo('pay')).toBe('Next → timer_1'));
    expect(leadsTo('timer_1')).toBe('Next → done');
    expect(screen.getByTestId('edit-notice')).toHaveTextContent('between "pay" and "done"');
  });

  it('adds the first node of an empty workflow as its first step', async () => {
    const { user } = await opened({
      state: {
        ir: { apiVersion: 'sagat-octopus-orchestrator.io/v1', kind: 'Workflow', metadata: { name: 'order-flow' }, entrypoint: '', triggers: [{ kind: 'manual' }], nodes: [] },
      },
    });
    expect(screen.getByText(/has no nodes yet/)).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Add node' }));
    await user.click(await screen.findByRole('button', { name: 'Step' }));
    expect(row('activity_1')).toHaveTextContent('First step');
  });

  it('closes the menu with Escape and gives focus back to the button that opened it', async () => {
    const { user } = await opened();
    const opener = screen.getByRole('button', { name: 'Add node' });
    await user.click(opener);
    const dialog = await screen.findByRole('dialog', { name: 'Add a node' });
    // A browser turns Escape into this event on a modal dialog; jsdom does not.
    fireEvent(dialog, new Event('cancel', { cancelable: true }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Add a node' })).not.toBeInTheDocument());
    expect(opener).toHaveFocus();
  });
});

describe('connecting', () => {
  it('connects with the inspector: Next: choose a node', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    await user.selectOptions(screen.getByRole('combobox', { name: 'On error' }), 'done');
    expect(leadsTo('pay')).toBe('Next → done; On error → done');
  });

  it('removes a link by choosing the dash', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    await user.selectOptions(screen.getByRole('combobox', { name: 'Next' }), '—');
    expect(leadsTo('pay')).toBe('—');
  });

  it('refuses a link that would close a cycle, says why in a live region, and changes nothing', async () => {
    const { user, platform } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    const onError = screen.getByRole('combobox', { name: 'On error' });
    await user.selectOptions(onError, 'check');
    const notice = screen.getByTestId('edit-notice');
    expect(notice.closest('[role="status"]')).not.toBeNull();
    expect(notice).toHaveTextContent('Not changed. That link would make a cycle');
    expect(onError).toHaveValue('');
    expect(leadsTo('pay')).toBe('Next → done');
    await settle();
    expect(putsOf(platform)).toHaveLength(0);
    expect(screen.getByTestId('save-state')).toHaveTextContent('Saved · revision 1');
  });

  it('says the same refusal again when it is tried again', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    const onError = screen.getByRole('combobox', { name: 'On error' });
    await user.selectOptions(onError, 'check');
    const first = screen.getByTestId('edit-notice').firstElementChild;
    await user.selectOptions(onError, 'check');
    // The message is a new element each time, which is what makes a live region speak it again.
    expect(screen.getByTestId('edit-notice').firstElementChild).not.toBe(first);
  });

  it('connects from the list with Connect…, choosing the way out and the node', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Connect pay…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Connect…' });
    await user.selectOptions(within(dialog).getByRole('combobox', { name: 'Link' }), 'On error');
    await user.selectOptions(within(dialog).getByRole('combobox', { name: 'Leads to' }), 'done');
    await user.click(within(dialog).getByRole('button', { name: 'Connect' }));
    expect(leadsTo('pay')).toBe('Next → done; On error → done');
    expect(screen.getByTestId('edit-notice')).toHaveTextContent('Connected "pay" to "done" (On error)');
  });

  it('keeps the Connect… dialog open and says why when the link is refused', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Connect pay…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Connect…' });
    await user.selectOptions(within(dialog).getByRole('combobox', { name: 'Link' }), 'On error');
    await user.selectOptions(within(dialog).getByRole('combobox', { name: 'Leads to' }), 'check');
    await user.click(within(dialog).getByRole('button', { name: 'Connect' }));
    expect(within(dialog).getByRole('alert')).toHaveTextContent('would make a cycle');
    expect(leadsTo('pay')).toBe('Next → done');
  });

  it('builds a list of branches in order, with add and remove', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Add node' }));
    await user.click(await screen.findByRole('button', { name: 'Parallel' }));
    expect(screen.getByRole('heading', { level: 2, name: 'Parallel: parallel_1' })).toBeInTheDocument();
    const add = () => screen.getByRole('combobox', { name: 'Add a branch' });
    await user.selectOptions(add(), 'pay');
    await user.click(screen.getByRole('button', { name: 'Add branch' }));
    await user.selectOptions(add(), 'done');
    await user.click(screen.getByRole('button', { name: 'Add branch' }));
    expect(leadsTo('parallel_1')).toBe('Branch 1 → pay; Branch 2 → done');
    await user.click(screen.getByRole('button', { name: 'Remove branch 1, pay' }));
    expect(leadsTo('parallel_1')).toBe('Branch 1 → done');
  });
});

describe('renaming and deleting', () => {
  it('renames a node and every link to it follows', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    const id = screen.getByRole('textbox', { name: 'Id' });
    await user.clear(id);
    await user.type(id, 'charge{Enter}');
    expect(screen.getByRole('heading', { level: 2, name: 'Step: charge' })).toBeInTheDocument();
    expect(leadsTo('check')).toBe('If true → charge; If false → done');
    expect(screen.queryByTestId('row-pay')).not.toBeInTheDocument();
    expect(screen.getByTestId('edit-notice')).toHaveTextContent('Renamed "pay" to "charge"');
  });

  it('refuses a name that is taken or not an id, and says why next to the field', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    const id = screen.getByRole('textbox', { name: 'Id' });
    await user.clear(id);
    await user.type(id, 'done{Enter}');
    expect(screen.getByRole('alert')).toHaveTextContent('There is already a node "done"');
    expect(screen.getByRole('heading', { level: 2, name: 'Step: pay' })).toBeInTheDocument();
    await user.clear(id);
    await user.type(id, 'bad id{Enter}');
    expect(screen.getByRole('alert')).toHaveTextContent('1 to 64 letters, digits');
    expect(leadsTo('check')).toBe('If true → pay; If false → done');
  });

  it('deletes a node and the links to it, announces it, and undo brings it back', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Delete pay' }));
    expect(screen.queryByTestId('row-pay')).not.toBeInTheDocument();
    expect(leadsTo('check')).toBe('If false → done');
    expect(screen.getByTestId('edit-notice')).toHaveTextContent('Deleted Step "pay" and 1 link to it');
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    expect(leadsTo('check')).toBe('If true → pay; If false → done');
    expect(leadsTo('pay')).toBe('Next → done');
  });

  it('deletes from the inspector too', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    await user.click(screen.getByRole('button', { name: 'Delete node' }));
    expect(screen.queryByTestId('row-pay')).not.toBeInTheDocument();
    // Nothing is selected any more: the inspector is the workflow's.
    expect(screen.getByRole('heading', { level: 2, name: 'Workflow settings' })).toBeInTheDocument();
  });

  it('moves the first step to the next node when the first one is deleted', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Delete check' }));
    expect(row('pay')).toHaveTextContent('First step');
  });

  it('sets another node as the first step', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Set pay as first step' }));
    expect(row('pay')).toHaveTextContent('First step');
    expect(row('check')).not.toHaveTextContent('First step');
    expect(screen.getByRole('button', { name: 'Set pay as first step' })).toBeDisabled();
  });
});

describe('undo and redo', () => {
  it('undoes and redoes with the buttons and with Ctrl+Z, Ctrl+Shift+Z and Ctrl+Y', async () => {
    const { user } = await opened();
    expect(screen.getByRole('button', { name: 'Undo' })).toBeDisabled();
    await user.click(screen.getByRole('button', { name: 'Delete pay' }));
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    expect(screen.getAllByRole('row')).toHaveLength(2);
    await user.keyboard('{Control>}z{/Control}');
    expect(screen.getAllByRole('row')).toHaveLength(3);
    expect(screen.getByTestId('edit-notice')).toHaveTextContent('Undone.');
    await user.keyboard('{Control>}{Shift>}z{/Shift}{/Control}');
    expect(screen.getAllByRole('row')).toHaveLength(2);
    await user.keyboard('{Control>}z{/Control}{Control>}z{/Control}');
    expect(screen.getAllByRole('row')).toHaveLength(4);
    expect(screen.getByRole('button', { name: 'Undo' })).toBeDisabled();
    await user.keyboard('{Control>}y{/Control}');
    expect(screen.getAllByRole('row')).toHaveLength(3);
    await user.click(screen.getByRole('button', { name: 'Redo' }));
    expect(screen.getAllByRole('row')).toHaveLength(2);
    expect(screen.getByRole('button', { name: 'Redo' })).toBeDisabled();
  });

  it('folds typing in one field into one step to undo', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    const activity = screen.getByRole('textbox', { name: 'Activity' });
    await user.clear(activity);
    await user.type(activity, 'payment.capture');
    expect(row('pay')).toHaveTextContent('payment.capture');
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    expect(row('pay')).toHaveTextContent('payment.submit');
    expect(screen.getByRole('button', { name: 'Undo' })).toBeDisabled();
  });

  it('does not undo the document while the person is typing in a field', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    const activity = screen.getByRole('textbox', { name: 'Activity' });
    await user.click(activity);
    await user.keyboard('{Control>}z{/Control}');
    expect(screen.queryByTestId('row-done')).not.toBeInTheDocument();
  });
});
