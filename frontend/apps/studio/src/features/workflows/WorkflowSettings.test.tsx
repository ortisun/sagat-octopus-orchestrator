import { screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Route, Routes } from 'react-router';
import { createApp } from '../../app/context';
import type { IrDocument } from '../../model/ir';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import { WorkflowSettings } from './WorkflowSettings';

const WORKFLOW_ID = '019b5d6e-0000-7000-8000-0000000000aa';
const doc: IrDocument = {
  apiVersion: 'sagat-octopus-orchestrator.io/v1',
  kind: 'Workflow',
  metadata: { name: 'classified-order' },
  entrypoint: 'done',
  inputSchema: { type: 'object', 'x-sagat-classification': 'public', properties: { orderId: { type: 'string' }, customer: { type: 'string', 'x-sagat-classification': 'pii' } } },
  nodes: [{ id: 'done', type: 'end' }],
};

describe('workflow classification report', () => {
  it('shows platform-computed effective levels, inherited-open warnings and classified diff findings', async () => {
    const platform = fakePlatform({
      [`POST /workflows/${WORKFLOW_ID}/validate`]: () => ({
        valid: true,
        problems: [],
        warnings: [{ severity: 'warning', code: 'CLASSIFICATION_INHERITED_OPEN', message: 'This declared location inherits an open level.', path: '/inputSchema/properties/orderId' }],
        classification: [
          { carrier: 'inputSchema', path: '', level: 'public', declared: true },
          { carrier: 'inputSchema', path: '/properties/orderId', level: 'public', declared: false },
          { carrier: 'inputSchema', path: '/properties/customer', level: 'pii', declared: true },
          { carrier: 'nodeOutputSchema', nodeId: 'charge', bodyName: 'retry-path', path: '/properties/receipt', level: 'internal', declared: true },
        ],
        classificationTruncated: true,
      }),
      [`GET /workflows/${WORKFLOW_ID}/diff`]: () => ({
        from: { kind: 'version', number: 1 },
        to: { kind: 'draft', revision: 2 },
        identical: false,
        nodes: { added: [], removed: [], changed: [] },
        edges: { added: [], removed: [] },
        changes: [],
        breaking: [],
        attention: [
          { code: 'CLASSIFICATION_RELAXED', message: 'Customer email changes from pii to internal.', path: '/inputSchema/properties/customer' },
          { code: 'CLASSIFICATION_TIGHTENED', message: 'Order id changes from public to internal.', path: '/inputSchema/properties/orderId' },
        ],
      }),
    });
    const app = createApp({ fetch: platform.fetcher });
    await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions: ['definition:read', 'definition:write'] } as never));
    app.namespace.set('payments');
    renderApp(
      <Routes>
        <Route path="/workflows/:workflowId" element={<WorkflowSettings doc={doc} disabled={false} change={vi.fn()} />} />
      </Routes>,
      { app, path: `/workflows/${WORKFLOW_ID}` },
    );

    const report = screen.getByRole('region', { name: 'Classification report' });
    const review = within(report).getByRole('button', { name: 'Review effective classification' });
    review.focus();
    await userEvent.setup().keyboard('{Enter}');
    const table = await within(report).findByRole('table', { name: 'Effective classification of declared schema locations' });
    const rows = within(table).getAllByRole('row').slice(1);
    expect(rows).toHaveLength(4);
    expect(within(rows[0] as HTMLElement).getAllByRole('cell').map((cell) => cell.textContent)).toEqual(['Workflow input', '/ (root)', '○ public', 'Declared here']);
    expect(within(rows[1] as HTMLElement).getAllByRole('cell').map((cell) => cell.textContent)).toEqual(['Workflow input', '/properties/orderId', '○ public', 'Inherited']);
    expect(within(rows[2] as HTMLElement).getAllByRole('cell').map((cell) => cell.textContent)).toEqual(['Workflow input', '/properties/customer', '△ pii', 'Declared here']);
    expect(within(rows[3] as HTMLElement).getAllByRole('cell').map((cell) => cell.textContent)).toEqual(['Step charge output in body retry-path', '/properties/receipt', '□ internal', 'Declared here']);
    expect(within(report).getByRole('alert')).toHaveTextContent('Inherited open levels (1)');
    expect(within(report).getByRole('alert')).toHaveTextContent('/inputSchema/properties/orderId');
    expect(within(report).getByText(/limited to 500 locations/)).toBeInTheDocument();
    expect(within(report).getByRole('heading', { name: /Classification changes need attention \(2\)/ })).toBeInTheDocument();
    expect(report).toHaveTextContent('CLASSIFICATION_RELAXED');
    expect(report).toHaveTextContent('CLASSIFICATION_TIGHTENED');
  });
});
