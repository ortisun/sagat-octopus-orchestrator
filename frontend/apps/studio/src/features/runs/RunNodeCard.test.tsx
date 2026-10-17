import { render, screen, within } from '@testing-library/react';
import { ReactFlowProvider } from '@xyflow/react';
import type { NodeProps } from '@xyflow/react';
import { describe, expect, it } from 'vitest';
import type { RuntimeNode } from '../../api/types';
import type { CardNode } from './canvas';
import { RunCard, RunChip } from './RunNodeCard';

function node(over: Partial<RuntimeNode> = {}): RuntimeNode {
  return { id: 'validate', type: 'activity', status: 'READY', ...over };
}

function props(value: RuntimeNode): NodeProps<CardNode> {
  return { data: { node: value, flash: 0 } } as NodeProps<CardNode>;
}

function full(value: RuntimeNode) {
  return render(
    <ReactFlowProvider>
      <RunCard {...props(value)} />
    </ReactFlowProvider>,
  );
}

describe('RunCard', () => {
  it('separates an 80-character identity, kind and state without shortening their text', () => {
    const id = 'i'.repeat(80);
    const forward = 'f'.repeat(64);
    const view = full(node({ id, status: 'WAITING', kind: 'COMPENSATION', forward }));
    const card = view.container.querySelector('.rn-card');

    expect(card).not.toBeNull();
    expect(within(card as HTMLElement).getByText(id)).toHaveClass('rn-id');
    expect(within(card as HTMLElement).getByText(`Undo of ${forward}`)).toHaveClass('rn-kind');
    expect(within(card as HTMLElement).getByText('Waiting')).toBeInTheDocument();
    expect(card?.querySelector('.rn-card-identity')).not.toBeNull();
    expect(card?.querySelector('.rn-card-kind')).not.toBeNull();
    expect(card?.querySelector('.rn-card-status')).not.toBeNull();
  });

  it('shows one short critical summary without a hover-only notes popup', () => {
    const view = full(node({ status: 'WAITING', waitingOn: 'operator', attempt: 4, failureCode: 'COMPENSATION_FAILED', dueAt: '2099-01-01T00:00:00Z' }));

    expect(screen.getByText('Needs a person')).toBeInTheDocument();
    expect(view.container.querySelector('.rn-notes')).toBeNull();
    expect(screen.queryByText('attempt 4')).not.toBeInTheDocument();
    expect(screen.queryByText('COMPENSATION_FAILED')).not.toBeInTheDocument();
  });

  it('names a bounded failure code and uses an honest summary for arbitrary longer text', () => {
    const first = full(node({ status: 'FAILED', failureCode: 'HTTP_500' }));
    expect(screen.getByText('Failure: HTTP_500')).toBeInTheDocument();
    first.unmount();

    full(node({ status: 'FAILED', failureCode: 'x'.repeat(200) }));
    expect(screen.getByText('Failure details available')).toBeInTheDocument();
    expect(screen.queryByText(/xxx/)).not.toBeInTheDocument();
  });

  it('states when an arbitrary identifier is beyond the canvas contract and exposes it accessibly', () => {
    const id = 'step-'.repeat(40);
    full(node({ id }));

    expect(screen.getByText('Long step ID — select for details')).toHaveAttribute('aria-label', id);
    expect(screen.queryByText(id)).not.toBeInTheDocument();
  });

  it('uses an explicit accessible summary when arbitrary type text exceeds the card contract', () => {
    const forward = 'forward-step-'.repeat(12);
    full(node({ id: 'undo', kind: 'COMPENSATION', forward }));

    expect(screen.getByText('Undo step — select for details')).toHaveAttribute('aria-label', `Undo of ${forward}`);
    expect(screen.queryByText(/…/)).not.toBeInTheDocument();
  });

  it('identifies an unknown runtime node as unsupported without hiding its status', () => {
    const type = 'futurePrimitive';
    full(node({ type, status: 'WAITING' }));

    expect(screen.getByText(`Unsupported node (${type})`)).toBeInTheDocument();
    expect(screen.getByText('Waiting')).toBeInTheDocument();
  });

  it('bounds an unknown long type on the card and preserves its accessible name', () => {
    const type = 'future-'.repeat(20);
    full(node({ type }));

    expect(screen.getByText('Unsupported node — select for details'))
      .toHaveAttribute('aria-label', `Unsupported node (${type})`);
  });
});

describe('RunChip', () => {
  it('keeps identity and state on separate rows in the simplified graph', () => {
    const id = 's'.repeat(80);
    const view = render(
      <ReactFlowProvider>
        <RunChip {...props(node({ id, status: 'SUCCEEDED' }))} />
      </ReactFlowProvider>,
    );
    const chip = view.container.querySelector('.rn-chipnode');

    expect(within(chip as HTMLElement).getByText(id)).toHaveClass('rn-id');
    expect(within(chip as HTMLElement).getByText('Succeeded')).toBeInTheDocument();
    expect(chip?.querySelector('.rn-card-status')).not.toBeNull();
  });
});
