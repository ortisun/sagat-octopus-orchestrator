import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { mountEditor, putsOf, settle } from '../../../test/workflowsHarness';

async function opened(options: Parameters<typeof mountEditor>[0] = {}) {
  const mounted = await mountEditor({ view: 'list', ...options });
  await screen.findByRole('heading', { level: 1, name: 'order-flow' });
  return mounted;
}

describe('a viewer (definition:read only)', () => {
  const viewer = { permissions: ['definition:read'] };

  it('sees the workflow, with a banner that says why nothing can be changed', async () => {
    await opened(viewer);
    expect(screen.getByText(/You can read this workflow but not change it/)).toBeInTheDocument();
    expect(screen.getByTestId('save-state')).toHaveTextContent('Read only');
    expect(screen.getAllByRole('row')).toHaveLength(4);
  });

  it('has no controls that edit: no add, undo, redo, save, publish, delete, connect, set-first', async () => {
    await opened(viewer);
    for (const name of ['Add node', 'Undo', 'Redo', 'Save now', 'Publish…', /^Delete /, /^Connect /, /^Set .* as first step$/]) {
      expect(screen.queryByRole('button', { name })).not.toBeInTheDocument();
    }
    // It can still open a node to read it.
    expect(screen.getByRole('button', { name: 'Open pay' })).toBeInTheDocument();
  });

  it('shows the fields of a node but does not let them be changed', async () => {
    const { user } = await opened(viewer);
    await user.click(screen.getByRole('button', { name: 'Open pay' }));
    expect(screen.getByRole('textbox', { name: 'Activity' })).toBeDisabled();
    expect(screen.getByRole('textbox', { name: 'Id' })).toBeDisabled();
    expect(screen.getByRole('combobox', { name: 'Next' })).toBeDisabled();
    expect(screen.getByRole('textbox', { name: 'Timeout amount' })).toBeDisabled();
    expect(screen.queryByRole('button', { name: 'Delete node' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Rename' })).not.toBeInTheDocument();
  });

  it('shows the settings and the triggers read only', async () => {
    const { user } = await opened(viewer);
    expect(screen.getByRole('textbox', { name: 'Description' })).toBeDisabled();
    expect(screen.getByRole('combobox', { name: 'First step' })).toBeDisabled();
    const schema = screen.getByRole('textbox', { name: 'Input schema' });
    expect(schema).toHaveAttribute('readonly');
    const original = (schema as HTMLTextAreaElement).value;
    await user.type(schema, 'must not change');
    expect(schema).toHaveValue(original);
    expect(screen.queryByRole('button', { name: 'Add trigger' })).not.toBeInTheDocument();
  });

  it('ignores the shortcuts that edit, and never saves', async () => {
    const { user, platform } = await opened(viewer);
    await user.keyboard('{Alt>}n{/Alt}');
    await user.keyboard('{Control>}z{/Control}');
    await user.keyboard('{Control>}s{/Control}');
    await settle(150);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(putsOf(platform)).toHaveLength(0);
  });

  it('does not ask the platform to check the document (it would refuse), and says why', async () => {
    const { platform } = await opened(viewer);
    await settle(200);
    expect(platform.calls.filter((c) => c.url.endsWith('/validate'))).toHaveLength(0);
    expect(await screen.findByText(/checks a workflow for the people who can change it/)).toBeInTheDocument();
  });

  it('can still publish when it is allowed to and has not changed anything', async () => {
    await opened({ permissions: ['definition:read', 'definition:publish'] });
    await waitFor(() => expect(screen.getByRole('button', { name: 'Publish…' })).not.toHaveAttribute('aria-disabled'));
    expect(screen.queryByRole('button', { name: 'Save now' })).not.toBeInTheDocument();
  });

  it('is told it cannot see the workflow without definition:read, and asks the platform for nothing', async () => {
    const { platform } = await mountEditor({ permissions: ['run:read'] });
    expect(await screen.findByText(/needs the definition:read permission/)).toBeInTheDocument();
    expect(platform.calls.filter((c) => c.url.includes('/workflows/'))).toHaveLength(0);
  });
});

describe('roles, names and states', () => {
  it('names the regions of the editor and the controls that matter', async () => {
    await opened();
    expect(screen.getByRole('group', { name: 'Editor tools' })).toBeInTheDocument();
    expect(screen.getByRole('group', { name: 'View' })).toBeInTheDocument();
    expect(screen.getByRole('complementary', { name: 'Inspector' })).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Checks and versions' })).toBeInTheDocument();
    expect(screen.getByRole('tablist', { name: 'Checks and versions' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Validation', selected: true })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Versions', selected: false })).toBeInTheDocument();
    for (const name of ['Canvas', 'List']) expect(screen.getByRole('button', { name })).toHaveAttribute('aria-pressed');
    expect(screen.getByRole('button', { name: 'Keyboard help' })).toHaveAttribute('aria-keyshortcuts', '?');
    expect(screen.getByRole('button', { name: 'Validation and versions' })).toHaveAttribute('aria-controls', 'wf-dock-body');
  });

  it('has a table of the nodes with a caption, column headers and row headers', async () => {
    const { user } = await opened();
    const table = screen.getByRole('table', { name: 'Nodes of the workflow' });
    expect(
      within(table)
        .getAllByRole('columnheader')
        .map((h) => h.textContent),
    ).toEqual(['Id', 'Type', 'Key field', 'Leads to', 'Checks', 'Actions']);
    expect(
      within(table)
        .getAllByRole('rowheader')
        .map((h) => h.textContent?.replace(/\s+/g, ' ').trim()),
    ).toEqual(['check ▶ First step', 'pay', 'done']);
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    expect(screen.getByTestId('row-pay')).toHaveAttribute('aria-current', 'true');
    expect(screen.getByTestId('row-check')).not.toHaveAttribute('aria-current');
  });

  it('names every button of a row by the node it acts on', async () => {
    await opened();
    const row = screen.getByTestId('row-pay');
    expect(
      within(row)
        .getAllByRole('button')
        .map((b) => b.getAttribute('aria-label')),
    ).toEqual(['Edit pay', 'Connect pay…', 'Set pay as first step', 'Delete pay']);
  });

  it('moves focus to the inspector heading when a row is edited, and to the nodes heading after a delete', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    await waitFor(() => expect(screen.getByRole('heading', { level: 2, name: 'Step: pay' })).toHaveFocus());
    await user.click(screen.getByRole('button', { name: 'Delete pay' }));
    await waitFor(() => expect(screen.getByRole('heading', { level: 2, name: 'Nodes' })).toHaveFocus());
  });

  it('filters the list of nodes by id or type', async () => {
    const { user } = await opened();
    await user.type(screen.getByRole('searchbox', { name: 'Filter nodes' }), 'cond');
    expect(screen.getAllByRole('row')).toHaveLength(2);
    expect(screen.getByTestId('row-count')).toHaveTextContent('1 of 3 nodes');
    await user.clear(screen.getByRole('searchbox', { name: 'Filter nodes' }));
    await user.type(screen.getByRole('searchbox', { name: 'Filter nodes' }), 'zzz');
    expect(screen.getByText(/No node matches/)).toBeInTheDocument();
  });

  it('keeps hostile text as text: an id with markup is shown, not run', async () => {
    const hostile = '<img src=x onerror=alert(1)>';
    await opened({
      state: {
        ir: {
          apiVersion: 'sagat-octopus-orchestrator.io/v1',
          kind: 'Workflow',
          metadata: { name: 'order-flow' },
          entrypoint: hostile,
          nodes: [{ id: hostile, type: 'activity', activity: '<script>alert(2)</script>' }],
        },
      },
    });
    expect(document.querySelector('img')).toBeNull();
    expect(document.querySelector('script')).toBeNull();
    expect(screen.getByTestId(`row-${hostile}`)).toHaveTextContent(hostile);
    expect(screen.getByTestId(`row-${hostile}`)).toHaveTextContent('<script>alert(2)</script>');
  });
});

describe('keyboard help and dialogs', () => {
  it('opens with ? and lists the shortcuts, and closing it gives focus back to where it was', async () => {
    const { user } = await opened();
    const edit = screen.getByRole('button', { name: 'Edit pay' });
    edit.focus();
    await user.keyboard('?');
    const dialog = await screen.findByRole('dialog', { name: 'Keyboard help' });
    for (const keys of ['?', 'Alt + N', 'Ctrl/Cmd + Z', 'Ctrl/Cmd + S', 'Enter', 'Delete or Backspace']) expect(within(dialog).getByText(keys)).toBeInTheDocument();
    await user.click(within(dialog).getByRole('button', { name: 'Close' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(edit).toHaveFocus();
  });

  it('opens from the toolbar button, closes with Escape, and gives focus back to the button', async () => {
    const { user } = await opened();
    const opener = screen.getByRole('button', { name: 'Keyboard help' });
    await user.click(opener);
    const dialog = await screen.findByRole('dialog', { name: 'Keyboard help' });
    fireEvent(dialog, new Event('cancel', { cancelable: true }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(opener).toHaveFocus();
  });

  it('does not take the ? typed into a field', async () => {
    const { user } = await opened();
    await user.type(screen.getByRole('textbox', { name: 'Description' }), 'is it ready?');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Description' })).toHaveValue('is it ready?');
  });

  it('does not open the help over another dialog', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Add node' }));
    await screen.findByRole('dialog', { name: 'Add a node' });
    await user.keyboard('?');
    expect(screen.queryByRole('dialog', { name: 'Keyboard help' })).not.toBeInTheDocument();
  });

  it('opens the connect dialog and gives focus back to the row button', async () => {
    const { user } = await opened();
    const opener = screen.getByRole('button', { name: 'Connect pay…' });
    await user.click(opener);
    const dialog = await screen.findByRole('dialog', { name: 'Connect…' });
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(opener).toHaveFocus();
  });
});
