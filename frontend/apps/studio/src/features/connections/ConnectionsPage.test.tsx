import { act, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import { createApp } from '../../app/context';
import { ConnectionsPage } from './ConnectionsPage';

const canary = 'postgres-password-private';
async function mount(failure?: 'secret' | 'connection', permissions = ['namespace:read', 'database:manage', 'secret:read', 'secret:manage']) {
  let saved: Record<string, unknown> | undefined;
  const platform = fakePlatform({
    'GET /namespaces': () => ({ items: [{ id: 'ns-1', slug: 'payments', environment: 'prod' }, { id: 'ns-2', slug: 'other', environment: 'dev' }] }),
    'GET /namespaces/ns-1/connections': () => ({ items: saved ? [saved] : [] }),
    'GET /namespaces/ns-2/connections': () => ({ items: [] }),
    'DELETE /namespaces/ns-1/connections/orders-db': () => { saved = undefined; return { status: 'DELETED' }; },
    'GET /namespaces/ns-1/secrets': () => ({ items: [{ name: 'existing-access', version: 1, provider: 'openbao' }, { name: 'legacy-access', version: 1, provider: 'local' }] }),
    'PUT *': (init, url) => {
      const body = JSON.parse(init.body as string) as Record<string, unknown>;
      if (url.includes('/secrets/')) return failure === 'secret'
        ? new Response(JSON.stringify({ code: 'UNAVAILABLE', message: canary }), { status: 503 })
        : { name: url.split('/').at(-1), version: 1, provider: 'openbao', keyId: null };
      if (failure === 'connection') return new Response(JSON.stringify({ code: 'REVISION_CONFLICT', message: 'Refresh the connection and retry.' }), { status: 409 });
      saved = { ...body, id: 'connection-1', name: 'orders-db', revision: 1 }; return saved;
    },
  });
  const app = createApp({ fetch: platform.fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions } as never));
  app.namespace.set('payments'); renderApp(<ConnectionsPage />, { app });
  return { app, platform, user: userEvent.setup() };
}
async function enter(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole('button', { name: 'New connection' }));
  await user.type(screen.getByLabelText('Name'), 'orders-db');
  await user.type(screen.getByLabelText('Host'), 'db.example.com');
  await user.type(screen.getByLabelText('Database'), 'orders');
}
async function credentials(user: ReturnType<typeof userEvent.setup>) {
  await user.selectOptions(screen.getByLabelText('Credentials'), 'new');
  await user.type(screen.getByLabelText('Username'), 'orders-reader');
  await user.type(screen.getByLabelText('Password'), canary);
}
describe('Database connections', () => {
  it('writes new credentials to OpenBao before saving a reference and clears their values', async () => {
    const { user, platform, app } = await mount(); await enter(user); await credentials(user);
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await screen.findByRole('button', { name: 'Edit connection orders-db' });
    const writes = platform.calls.filter((c) => c.method === 'PUT');
    expect(writes).toHaveLength(2);
    expect(JSON.parse(writes[0]?.init.body as string)).toMatchObject({ requiredProvider: 'openbao', expectedVersion: 0, requestId: expect.any(String) as unknown });
    expect(writes[0]?.init.body).toContain(canary);
    expect(writes[1]?.init.body).not.toContain(canary);
    expect(JSON.parse(writes[1]?.init.body as string)).toMatchObject({ expectedRevision: 0, config: { driver: 'postgresql', tls: { mode: 'verify-full' } } });
    expect(document.body.textContent).not.toContain(canary);
    expect(JSON.stringify(app.queries.getQueryCache().getAll().map(q => q.state.data))).not.toContain(canary);
    expect(screen.queryByLabelText('Password')).not.toBeInTheDocument();
  });
  it('retains a saved secret and the connection request identity after a partial failure', async () => {
    const { user, platform } = await mount('connection'); await enter(user); await credentials(user);
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await screen.findByText(/Credentials saved/);
    expect(screen.queryByLabelText('Password')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await waitFor(() => expect(platform.calls.filter(c => c.method === 'PUT')).toHaveLength(3));
    const writes = platform.calls.filter(c => c.method === 'PUT');
    expect(writes[1]?.init.body).toBe(writes[2]?.init.body);
    expect(writes.filter(c => c.url.includes('/secrets/'))).toHaveLength(1);
  });
  it('retries an uncertain secret write with the same receipt and hides raw provider errors', async () => {
    const { user, platform } = await mount('secret'); await enter(user); await credentials(user);
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await screen.findByText(/Credentials could not be saved/);
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await screen.findByText(/Credentials could not be saved/);
    const writes = platform.calls.filter(c => c.method === 'PUT');
    expect(writes).toHaveLength(2);
    expect(writes[0]?.init.body).toBe(writes[1]?.init.body);
    expect(writes.every(c => c.url.includes('/secrets/'))).toBe(true);
    expect(document.body.textContent).not.toContain(canary);
  });
  it('uses existing OpenBao references and keeps legacy values unavailable until migration', async () => {
    const { user, platform } = await mount(); await enter(user);
    await user.selectOptions(screen.getByLabelText('Secret reference'), 'existing-access');
    expect(screen.getByRole('option', { name: /legacy-access/ })).toBeDisabled();
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await screen.findByRole('button', { name: 'Edit connection orders-db' });
    expect(platform.calls.filter(c => c.method === 'PUT')).toHaveLength(1);
  });
  it('discards credential inputs when switching namespace and hides management from viewers', async () => {
    const { user, app, platform } = await mount(); await enter(user); await credentials(user);
    act(() => app.namespace.set('other'));
    await screen.findByText('No database connections yet.');
    expect(screen.queryByLabelText('Password')).not.toBeInTheDocument();
    expect(platform.calls.every(c => c.method === 'GET')).toBe(true);
  });
  it('requires an explicit delete action with the observed revision and a stable request', async () => {
    const { user, platform } = await mount(); await enter(user);
    await user.selectOptions(screen.getByLabelText('Secret reference'), 'existing-access');
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await user.click(await screen.findByRole('button', { name: 'Delete connection orders-db' }));
    expect(platform.calls.some(c => c.method === 'DELETE')).toBe(false);
    await user.click(screen.getByRole('button', { name: 'Delete connection' }));
    await screen.findByText('No database connections yet.');
    expect(JSON.parse(platform.calls.find(c => c.method === 'DELETE')?.init.body as string)).toMatchObject({ expectedRevision: 1 });
  });
  it('does not show create actions to a viewer', async () => {
    await mount(undefined, ['namespace:read']);
    await screen.findByText('No database connections yet.');
    expect(screen.queryByRole('button', { name: 'New connection' })).not.toBeInTheDocument();
  });
});
