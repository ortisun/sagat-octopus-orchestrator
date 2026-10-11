import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { emptyDocument } from '../../model/ir';
import type { IrDocument } from '../../model/ir';
import { NodesTable, VIRTUAL_ABOVE } from './NodesTable';

const none = new Map<string, number>();

function chain(count: number): IrDocument {
  return {
    ...emptyDocument('big-flow'),
    entrypoint: 'n0',
    nodes: Array.from({ length: count }, (_, i) => ({ id: `n${i}`, type: 'activity', activity: `act.${i}`, ...(i < count - 1 ? { next: `n${i + 1}` } : {}) })),
  };
}

function table(doc: IrDocument, over: Partial<Parameters<typeof NodesTable>[0]> = {}) {
  const handlers = { onEdit: vi.fn(), onDelete: vi.fn(), onSetFirst: vi.fn(), onConnect: vi.fn() };
  render(<NodesTable doc={doc} selectedId={undefined} problems={none} advice={none} editable caption="Nodes" {...handlers} {...over} />);
  return handlers;
}

describe('NodesTable', () => {
  it('puts every row in the page up to the size where windowing starts', () => {
    table(chain(VIRTUAL_ABOVE));
    expect(screen.getAllByRole('row')).toHaveLength(VIRTUAL_ABOVE + 1);
    expect(screen.getByRole('table')).not.toHaveAttribute('aria-rowcount');
  });

  it('puts only the rows in view in the page above that, and says how many there are for assistive technology', () => {
    table(chain(5_000));
    const rows = screen.getAllByRole('row');
    expect(rows.length).toBeLessThan(100);
    expect(screen.getByRole('table')).toHaveAttribute('aria-rowcount', '5001');
    expect(screen.getByTestId('row-count')).toHaveTextContent('5000 nodes');
    // The scrolling region is reachable by keyboard and has a name.
    expect(screen.getByRole('region', { name: 'Nodes (scrolls)' })).toHaveAttribute('tabindex', '0');
  });

  it('filters across all the nodes, not only those in view', async () => {
    table(chain(5_000));
    await userEvent.type(screen.getByRole('searchbox', { name: 'Filter nodes' }), 'n4999');
    expect(screen.getByTestId('row-n4999')).toBeInTheDocument();
    expect(screen.getByTestId('row-count')).toHaveTextContent('1 of 5000 nodes');
  });

  it('calls the actions of a row with the node of that row', async () => {
    const user = userEvent.setup();
    const handlers = table(chain(3));
    const row = screen.getByTestId('row-n1');
    await user.click(within(row).getByRole('button', { name: 'Edit n1' }));
    await user.click(within(row).getByRole('button', { name: 'Delete n1' }));
    await user.click(within(row).getByRole('button', { name: 'Set n1 as first step' }));
    await user.click(within(row).getByRole('button', { name: 'Connect n1…' }));
    expect(handlers.onEdit).toHaveBeenCalledWith('n1');
    expect(handlers.onDelete).toHaveBeenCalledWith('n1');
    expect(handlers.onSetFirst).toHaveBeenCalledWith('n1');
    expect(handlers.onConnect).toHaveBeenCalledWith('n1');
  });

  it('does not offer to connect a node that has no way out, or to make the first step the first step again', () => {
    table({ ...chain(2), nodes: [...chain(2).nodes, { id: 'z', type: 'end' }] });
    expect(screen.getByRole('button', { name: 'Connect z…' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Set n0 as first step' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Set n1 as first step' })).toBeEnabled();
  });

  it('has no editing actions when it only displays', () => {
    table(chain(2), { editable: false, inspectable: false });
    expect(screen.queryAllByRole('button')).toHaveLength(0);
  });

  it('says in words, not by colour alone, what is wrong with a node', () => {
    table(chain(2), { problems: new Map([['n0', 2]]), advice: new Map([['n1', 1]]) });
    expect(screen.getByTestId('row-n0')).toHaveTextContent('2 problems');
    expect(screen.getByTestId('row-n1')).toHaveTextContent('1 advice');
  });

  it('marks the selected row for assistive technology', () => {
    table(chain(3), { selectedId: 'n2' });
    expect(screen.getByTestId('row-n2')).toHaveAttribute('aria-current', 'true');
    expect(screen.getByTestId('row-n1')).not.toHaveAttribute('aria-current');
  });
});
