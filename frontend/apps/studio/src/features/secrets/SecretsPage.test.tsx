import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import { createApp } from '../../app/context';
import { SecretsPage } from './SecretsPage';
const canary = 'private-postgres-password-017';
async function mount(fail = false) {
  const secrets = [{ name: 'legacy-access', provider: 'local', version: 1 }, { name: 'orders-access', provider: 'openbao', version: 3 }];
  const platform = fakePlatform({
    'GET /namespaces': () => ({ items: [{ id: 'ns-1', slug: 'payments' }] }),
    'GET /namespaces/ns-1/secrets': () => ({ items: secrets }),
    'PUT *': (init, url) => {
      if (fail) return new Response(JSON.stringify({ message: canary }), { status: 503 });
      const body = JSON.parse(init.body as string) as { expectedVersion: number };
      const saved = { name: url.split('/').at(-1)!, version: body.expectedVersion + 1, provider: 'openbao' };
      const previous = secrets.findIndex(s => s.name === saved.name); if (previous >= 0) secrets[previous] = saved; else secrets.push(saved);
      return saved;
    },
    'POST /namespaces/ns-1/secrets/legacy-access/migrate': () => { secrets[0] = { name: 'legacy-access', version: 2, provider: 'openbao' }; return secrets[0]; },
  });
  const app = createApp({ fetch: platform.fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions: ['secret:read', 'secret:manage'] } as never));
  app.namespace.set('payments'); renderApp(<SecretsPage />, { app });
  return { app, platform, user: userEvent.setup() };
}
describe('Secrets', () => {
  it('rotates using the current version without fetching or displaying its old value', async () => {
    const { user, app, platform } = await mount();
    await user.click(await screen.findByRole('button', { name: 'Rotate secret orders-access' }));
    expect(screen.getByLabelText('Password')).toHaveValue('');
    await user.type(screen.getByLabelText('Username'), 'orders-user');
    await user.type(screen.getByLabelText('Password'), canary);
    await user.click(screen.getByRole('button', { name: 'Save secret' }));
    await screen.findByRole('button', { name: 'Rotate secret orders-access' });
    expect(JSON.parse(platform.calls.find(c => c.method === 'PUT')?.init.body as string)).toMatchObject({ expectedVersion: 3, requiredProvider: 'openbao' });
    expect(screen.queryByLabelText('Password')).not.toBeInTheDocument();
    expect(document.body.textContent).not.toContain(canary);
    expect(JSON.stringify(app.queries.getQueryData(['secrets', 'ns-1']))).not.toContain(canary);
    expect(platform.calls.filter(c => c.method === 'GET').every(c => !c.url.includes('/secrets/orders-access'))).toBe(true);
  });
  it('migrates through the facade without asking the browser to read a value', async () => {
    const { user, platform } = await mount();
    await user.click(await screen.findByRole('button', { name: 'Migrate secret legacy-access' }));
    expect(screen.queryByLabelText('Password')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Migrate to OpenBao' }));
    await screen.findByRole('button', { name: 'Rotate secret legacy-access' });
    const body = JSON.parse(platform.calls.find(c => c.method === 'POST')?.init.body as string) as Record<string, unknown>;
    expect(body.expectedVersion).toBe(1); expect(body.value).toBeUndefined();
  });
  it('retains the receipt on failure without showing provider diagnostics', async () => {
    const { user, platform } = await mount(true);
    await user.click(await screen.findByRole('button', { name: 'New secret' }));
    await user.type(screen.getByLabelText('Secret name'), 'new-access');
    await user.selectOptions(screen.getByLabelText('Credential format'), 'opaque');
    await user.type(screen.getByLabelText('Secret value'), canary);
    await user.click(screen.getByRole('button', { name: 'Save secret' })); await screen.findByRole('alert');
    await user.click(screen.getByRole('button', { name: 'Save secret' })); await screen.findByRole('alert');
    const writes = platform.calls.filter(c => c.method === 'PUT');
    expect(writes).toHaveLength(2); expect(writes[0]?.init.body).toBe(writes[1]?.init.body);
    expect(document.body.textContent).not.toContain(canary);
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByLabelText('Secret value')).not.toBeInTheDocument();
  });
});
