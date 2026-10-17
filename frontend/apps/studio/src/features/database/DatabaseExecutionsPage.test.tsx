import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import { createApp } from '../../app/context';
import { DatabaseExecutionsPage } from './DatabaseExecutionsPage';

const permissions = ['namespace:read', 'database:use', 'database:adhoc', 'run:start', 'run:read'];
async function setup() {
  let requests = 0;
  const platform = fakePlatform({
    'GET /namespaces': () => ({ items: [{ id: 'ns', slug: 'payments' }, { id: 'other', slug: 'other' }] }),
    'GET *': () => ({ items: [{ id: 'conn', name: 'orders', revision: 1, status: 'ACTIVE' }] }),
    'POST *': () => ++requests === 1 ? new Response(JSON.stringify({ code: 'UNAVAILABLE', message: 'Unavailable' }), { status: 503 }) : { runId: 'run-1', origin: 'AD_HOC_STEP' },
  });
  const app = createApp({ fetch: platform.fetcher }); await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions } as never));
  app.namespace.set('payments'); renderApp(<DatabaseExecutionsPage />, { app });
  const user = userEvent.setup(); await screen.findByRole('option', { name: 'orders · revision 1' });
  await user.selectOptions(screen.getByLabelText('Connection'), 'orders');
  return { platform, app, user };
}
describe('Ad hoc database editor', () => {
  it('blocks malformed parameters and freezes both the key and body after an uncertain acknowledgement', async () => {
    const { platform, user } = await setup();
    const parameters = screen.getByLabelText('Typed parameters (JSON)');
    fireEvent.change(parameters, { target: { value: '[' } });
    expect(screen.getByRole('button', { name: 'Run execution' })).toBeDisabled();
    fireEvent.change(parameters, { target: { value: '[{"literal":{"type":"json","value":{"id":9007199254740993}}}]' } });
    expect(screen.getByRole('button', { name: 'Run execution' })).toBeDisabled();
    expect(parameters).toHaveValue('[{"literal":{"type":"json","value":{"id":9007199254740993}}}]');
    fireEvent.change(parameters, { target: { value: '[{"type":"integer","expression":"input.number"}]' } });
    fireEvent.change(screen.getByLabelText('Run input (JSON)'), { target: { value: '{"number":"9223372036854775807"}' } });
    await user.click(screen.getByRole('button', { name: 'Run execution' }));
    await screen.findByRole('button', { name: 'Retry same execution' });
    expect(screen.getByLabelText('SQL statement')).toBeDisabled();
    expect(screen.queryByRole('button', { name: 'Edit refused request' })).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Retry same execution' }));
    await screen.findByRole('link', { name: 'Open run' });
    const writes = platform.calls.filter(c => c.method === 'POST'); expect(writes).toHaveLength(2);
    expect(writes[0]?.init.body).toEqual(writes[1]?.init.body);
    expect(new Headers(writes[0]?.init.headers).get('Idempotency-Key')).toEqual(new Headers(writes[1]?.init.headers).get('Idempotency-Key'));
    const body = JSON.parse(writes[0]?.init.body as string) as { definition: { triggers?: unknown; nodes: unknown[] } };
    expect(body.definition.triggers).toBeUndefined(); expect(body.definition.nodes).toHaveLength(1);
    expect(screen.queryByLabelText('SQL statement')).not.toBeInTheDocument();
  });
  it('restricts workflow nodes and writes, and discards unsent SQL on namespace change', async () => {
    const { user, app } = await setup();
    expect(screen.getByRole('option', { name: /Execute ·/ })).toBeDisabled();
    fireEvent.change(screen.getByLabelText('SQL statement'), { target: { value: 'SELECT private_canary' } });
    await user.selectOptions(screen.getByLabelText('Execution mode'), 'workflow');
    await user.click(screen.getByRole('button', { name: 'Add node' }));
    expect(screen.queryByRole('button', { name: 'Script' })).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Database' }));
    expect(screen.getByRole('heading', { name: 'Database: database_2' })).toBeInTheDocument();
    act(() => app.namespace.set('other'));
    await waitFor(() => expect(screen.getByLabelText('SQL statement')).toHaveValue('SELECT 1::bigint'));
    expect(document.body.textContent).not.toContain('private_canary');
  });
});
