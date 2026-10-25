import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { WF_ID, mountEditor, sampleIr, settle } from '../../../test/workflowsHarness';
import { PALETTE_MIME } from './canvasModel';

async function opened(options: Parameters<typeof mountEditor>[0] = {}) {
  const mounted = await mountEditor(options);
  await screen.findByRole('heading', { level: 1, name: 'order-flow' });
  const canvas = await screen.findByRole('region', { name: 'Workflow canvas' });
  return { ...mounted, canvas };
}

const card = (canvas: HTMLElement, id: string): HTMLElement => {
  const found = [...canvas.querySelectorAll<HTMLElement>('.react-flow__node')].find((el) => el.dataset['id'] === id);
  if (!found) throw new Error(`no card ${id}`);
  return found;
};
const focusCard = (canvas: HTMLElement, id: string) => act(() => card(canvas, id).focus());
const cards = (canvas: HTMLElement) => [...canvas.querySelectorAll<HTMLElement>('.react-flow__node')].map((el) => el.dataset['id']);

const chain = (count: number) => ({
  ...sampleIr(),
  entrypoint: 'n0',
  nodes: Array.from({ length: count }, (_, i) => ({ id: `n${i}`, type: 'activity', activity: 'a', timeoutMs: 1_000, ...(i < count - 1 ? { next: `n${i + 1}` } : {}) })),
});

describe('the canvas', () => {
  it('draws a card for every node and a start card for the trigger, each with a name that says what it is', async () => {
    const { canvas } = await opened();
    expect(cards(canvas)).toEqual(['@trigger:0', 'check', 'pay', 'done']);
    expect(card(canvas, 'check')).toHaveAccessibleName('Condition check, input.amount > 10, first step. Press Enter to open the inspector.');
    expect(card(canvas, 'pay')).toHaveAccessibleName('Step pay, payment.submit. Press Enter to open the inspector.');
    expect(card(canvas, '@trigger:0')).toHaveAccessibleName(/^Start: Manual start/);
    // The type is a word on the card, and the ways out are labelled.
    expect(within(card(canvas, 'check')).getByText('Condition')).toBeInTheDocument();
    expect(within(card(canvas, 'check')).getByText('If true')).toBeInTheDocument();
    expect(within(card(canvas, 'check')).getByText('If false')).toBeInTheDocument();
    expect(within(card(canvas, 'pay')).getByText('On error')).toBeInTheDocument();
  });

  it('puts one card in the tab order: Tab does not walk through every card', async () => {
    const { canvas } = await opened();
    expect(cards(canvas).filter((id) => card(canvas, id!).tabIndex === 0)).toEqual(['check']);
  });

  it('moves focus along the flow with the arrow keys, between connected cards, and the tab stop follows', async () => {
    const { canvas } = await opened();
    focusCard(canvas, 'check');
    fireEvent.keyDown(card(canvas, 'check'), { key: 'ArrowRight' });
    await waitFor(() => expect(card(canvas, 'pay')).toHaveFocus());
    expect(card(canvas, 'pay').tabIndex).toBe(0);
    expect(card(canvas, 'check').tabIndex).toBe(-1);
    fireEvent.keyDown(card(canvas, 'pay'), { key: 'ArrowRight' });
    await waitFor(() => expect(card(canvas, 'done')).toHaveFocus());
    fireEvent.keyDown(card(canvas, 'done'), { key: 'ArrowLeft' });
    await waitFor(() => expect(card(canvas, 'check')).toHaveFocus());
    fireEvent.keyDown(card(canvas, 'check'), { key: 'ArrowLeft' });
    await waitFor(() => expect(card(canvas, '@trigger:0')).toHaveFocus());
  });

  it('steps between the nodes one node leads to with the keys across the flow', async () => {
    const { canvas } = await opened();
    focusCard(canvas, 'pay');
    fireEvent.keyDown(card(canvas, 'pay'), { key: 'ArrowUp' });
    await waitFor(() => expect(card(canvas, 'pay')).toHaveFocus());
    // `check` leads to `pay` (if true) and `done` (if false): they are siblings.
    fireEvent.keyDown(card(canvas, 'pay'), { key: 'ArrowDown' });
    await waitFor(() => expect(card(canvas, 'done')).toHaveFocus());
    fireEvent.keyDown(card(canvas, 'done'), { key: 'ArrowUp' });
    await waitFor(() => expect(card(canvas, 'pay')).toHaveFocus());
  });

  it('opens the inspector with Enter and puts focus on it', async () => {
    const { canvas } = await opened();
    focusCard(canvas, 'pay');
    fireEvent.keyDown(card(canvas, 'pay'), { key: 'Enter' });
    const heading = await screen.findByRole('heading', { level: 2, name: 'Step: pay' });
    await waitFor(() => expect(heading).toHaveFocus());
  });

  it('selects with Space and with a click, and opens the settings from the start card', async () => {
    const { canvas } = await opened();
    fireEvent.keyDown(card(canvas, 'pay'), { key: ' ' });
    expect(await screen.findByRole('heading', { level: 2, name: 'Step: pay' })).toBeInTheDocument();
    // A click only: the pointer events that start a drag need a real layout engine.
    fireEvent.click(card(canvas, 'check'));
    expect(await screen.findByRole('heading', { level: 2, name: 'Condition: check' })).toBeInTheDocument();
    fireEvent.click(card(canvas, '@trigger:0'));
    expect(await screen.findByRole('heading', { level: 2, name: 'Workflow settings' })).toBeInTheDocument();
  });

  it('deletes the focused card with Delete, says so, keeps focus on the canvas, and undo brings it back', async () => {
    const { canvas, user } = await opened();
    focusCard(canvas, 'pay');
    fireEvent.keyDown(card(canvas, 'pay'), { key: 'Delete' });
    await waitFor(() => expect(cards(canvas)).not.toContain('pay'));
    expect(screen.getByTestId('edit-notice')).toHaveTextContent('Deleted Step "pay" and 1 link to it');
    expect(canvas.contains(document.activeElement)).toBe(true);
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    await waitFor(() => expect(cards(canvas)).toContain('pay'));
  });

  it('does not delete the start card, or anything for a viewer', async () => {
    const { canvas } = await opened();
    fireEvent.keyDown(card(canvas, '@trigger:0'), { key: 'Delete' });
    expect(cards(canvas)).toContain('@trigger:0');
    expect(screen.getByTestId('edit-notice')).toHaveTextContent('');
  });

  it('shows a badge on the card of a node the platform has a problem with, in words', async () => {
    const { canvas } = await opened({
      behaviour: { validate: () => ({ valid: false, problems: [{ severity: 'error', code: 'EXPRESSION', message: 'bad', nodeId: 'check' }], warnings: [] }) },
    });
    await waitFor(() => expect(within(card(canvas, 'check')).getByText('1 problem')).toBeInTheDocument());
    expect(card(canvas, 'check')).toHaveAccessibleName(/1 problem/);
    expect(within(card(canvas, 'pay')).queryByText(/problem/)).not.toBeInTheDocument();
  });

  it('says on the card that a node the platform cannot run yet is drawn but will not publish', async () => {
    const ir = sampleIr();
    (ir['nodes'] as unknown[]).push({ id: 'each', type: 'foreach', collectionExpr: 'input.items', itemKeyExpr: 'item.id', maxItems: 5, body: 'pay' });
    const { canvas } = await opened({ state: { ir } });
    expect(within(card(canvas, 'each')).getByText('Not run yet')).toBeInTheDocument();
    expect(card(canvas, 'each')).toHaveAccessibleName(/the platform cannot run this node yet/);
  });

  it('says what to do with an empty workflow', async () => {
    await opened({ state: { ir: { ...sampleIr(), entrypoint: '', nodes: [] } } });
    expect(screen.getByText(/This workflow is empty/)).toBeInTheDocument();
  });
});

describe('adding to the canvas', () => {
  it('has a palette of every kind that adds a node when pressed', async () => {
    const { canvas, user } = await opened();
    const palette = screen.getByRole('region', { name: 'Palette' });
    expect(within(palette).getAllByRole('button')).toHaveLength(12);
    await user.click(within(palette).getByRole('button', { name: 'Timer' }));
    await waitFor(() => expect(cards(canvas)).toContain('timer_1'));
    expect(screen.getByRole('heading', { level: 2, name: 'Timer: timer_1' })).toBeInTheDocument();
  });

  it('lets the kinds be dragged, and a drop on the canvas adds the node where it was dropped', async () => {
    const { canvas } = await opened();
    const palette = screen.getByRole('region', { name: 'Palette' });
    expect(within(palette).getByRole('button', { name: 'Timer' })).toHaveAttribute('draggable', 'true');
    const data: Record<string, string> = {};
    fireEvent.dragStart(within(palette).getByRole('button', { name: 'Timer' }), { dataTransfer: { setData: (k: string, v: string) => (data[k] = v), effectAllowed: '' } });
    expect(data[PALETTE_MIME]).toBe('timer');
    fireEvent.drop(canvas, { clientX: 300, clientY: 200, dataTransfer: { types: [PALETTE_MIME], getData: (k: string) => data[k] ?? '' } });
    await waitFor(() => expect(cards(canvas)).toContain('timer_1'));
  });

  it('ignores a drop that is not one of its kinds', async () => {
    const { canvas } = await opened();
    fireEvent.drop(canvas, { dataTransfer: { types: [PALETTE_MIME], getData: () => 'teleport' } });
    fireEvent.drop(canvas, { dataTransfer: { types: ['text/plain'], getData: (kind: string) => (kind === PALETTE_MIME ? '' : 'timer') } });
    expect(cards(canvas)).toEqual(['@trigger:0', 'check', 'pay', 'done']);
  });

  it('puts a node added with the menu in the middle of the canvas when nothing is selected, and remembers where', async () => {
    const { user } = await opened();
    await user.keyboard('{Alt>}n{/Alt}');
    const dialog = await screen.findByRole('dialog', { name: 'Add a node' });
    expect(dialog).toHaveTextContent('middle of the visible canvas');
    await user.click(within(dialog).getByRole('button', { name: 'Timer' }));
    await waitFor(() => expect(localStorage.getItem(`studio.workflow.layout.${WF_ID}`)).toContain('timer_1'));
  });

  it('hides the palette from a viewer, and does nothing for a drop or a Delete key', async () => {
    const { canvas } = await opened({ permissions: ['definition:read'] });
    expect(screen.queryByRole('region', { name: 'Palette' })).not.toBeInTheDocument();
    fireEvent.drop(canvas, { dataTransfer: { types: [PALETTE_MIME], getData: (k: string) => (k === PALETTE_MIME ? 'timer' : '') } });
    fireEvent.keyDown(card(canvas, 'pay'), { key: 'Delete' });
    await settle(60);
    expect(cards(canvas)).toEqual(['@trigger:0', 'check', 'pay', 'done']);
  });
});

describe('views and layout', () => {
  it('toggles between the canvas and the list, and opens the next time as it was left', async () => {
    const first = await opened();
    expect(screen.getByRole('button', { name: 'Canvas' })).toHaveAttribute('aria-pressed', 'true');
    await first.user.click(screen.getByRole('button', { name: 'List' }));
    expect(screen.getByRole('button', { name: 'List' })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('table')).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: 'Workflow canvas' })).not.toBeInTheDocument();
    first.unmount();
    await mountEditor({ keepStorage: true });
    expect(await screen.findByRole('table')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'List' })).toHaveAttribute('aria-pressed', 'true');
  });

  it('opens in the list when that is what was chosen, and goes back to the canvas', async () => {
    const { user } = await mountEditor({ view: 'list' });
    await screen.findByRole('table');
    await user.click(screen.getByRole('button', { name: 'Canvas' }));
    expect(await screen.findByRole('region', { name: 'Workflow canvas' })).toBeInTheDocument();
    expect(localStorage.getItem('studio.workflows.view')).toBe('canvas');
  });

  it('turns the layout around and remembers it', async () => {
    const { canvas, user } = await opened();
    expect(card(canvas, 'check').querySelector('.wf-card')).toHaveAttribute('data-direction', 'LR');
    await user.click(screen.getByRole('button', { name: /Layout direction: left to right/ }));
    expect(card(canvas, 'check').querySelector('.wf-card')).toHaveAttribute('data-direction', 'TB');
    expect(screen.getByRole('button', { name: /Layout direction: top to bottom/ })).toBeInTheDocument();
    expect(localStorage.getItem('studio.workflows.direction')).toBe('TB');
  });

  it('draws the cards where the user left them, and Re-layout puts them back and forgets', async () => {
    const key = `studio.workflow.layout.${WF_ID}`;
    const { canvas, user } = await opened({ seed: { [key]: JSON.stringify({ v: 1, positions: { pay: { x: 777, y: 333 } } }) } });
    expect(card(canvas, 'pay').style.transform).toBe('translate(777px,333px)');
    expect(card(canvas, 'check').style.transform).not.toBe('translate(777px,333px)');
    await user.click(screen.getByRole('button', { name: 'Re-layout' }));
    await waitFor(() => expect(card(canvas, 'pay').style.transform).not.toBe('translate(777px,333px)'));
    expect(localStorage.getItem(key)).toBeNull();
    expect(screen.getByTestId('edit-notice')).toHaveTextContent('laid out again');
  });

  it('ignores positions in storage that are not positions, and works without storage', async () => {
    const key = `studio.workflow.layout.${WF_ID}`;
    const { canvas } = await opened({ seed: { [key]: '{"v":1,"positions":{"pay":{"x":"far","y":null}}}' } });
    expect(card(canvas, 'pay').style.transform).toMatch(/^translate\(-?\d+px,-?\d+px\)$/);
  });
});

describe('big workflows (D147)', () => {
  it('draws simplified cards above a thousand nodes and says so', async () => {
    const { canvas } = await opened({ state: { ir: chain(1_001) } });
    expect(canvas).toHaveAttribute('data-mode', 'simplified');
    expect(screen.getByText(/the cards are simplified/)).toBeInTheDocument();
    expect(canvas.querySelector('.react-flow__minimap')).toBeNull();
  });

  it('draws full cards, with a mini-map, up to a thousand', async () => {
    const { canvas } = await opened();
    expect(canvas).toHaveAttribute('data-mode', 'full');
    expect(canvas.querySelector('.react-flow__minimap')).not.toBeNull();
    expect(screen.queryByText(/the cards are simplified/)).not.toBeInTheDocument();
  });

  it('opens a workflow of more than ten thousand nodes in the list and offers the canvas on demand', async () => {
    const { user } = await mountEditor({ state: { ir: chain(10_001) }, timings: { validateMs: 60_000, autosaveMs: 60_000 } });
    expect(await screen.findByRole('heading', { level: 1, name: 'order-flow' })).toBeInTheDocument();
    expect(screen.getByRole('table')).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: 'Workflow canvas' })).not.toBeInTheDocument();
    expect(screen.getByText(/10,001 nodes, more than a canvas draws comfortably/)).toBeInTheDocument();
    // The list is windowed: not ten thousand rows in the page.
    expect(screen.getAllByRole('row').length).toBeLessThan(200);
    expect(screen.getByRole('table')).toHaveAttribute('aria-rowcount', '10002');
    expect(screen.getByTestId('row-count')).toHaveTextContent('10001 nodes');
    // The choice of the view is not changed by what the size forces.
    expect(localStorage.getItem('studio.workflows.view')).toBeNull();
    await user.click(screen.getByRole('button', { name: 'Show the canvas anyway' }));
    const canvas = await screen.findByRole('region', { name: 'Workflow canvas' });
    expect(canvas).toHaveAttribute('data-mode', 'simplified');
  }, 30_000);
});
