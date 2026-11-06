import { act, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import { createApp } from '../../app/context';
import { ConnectionsPage } from './ConnectionsPage';

const canary = 'postgres-password-private';
const LAKE_DRIVER = { id: 'iceberg', catalogTypes: ['rest'], storageTypes: ['s3'], modes: ['append'] };
async function mount(failure?: 'secret' | 'connection' | 'provider', permissions = ['namespace:read', 'database:manage', 'secret:read', 'secret:manage'], seed?: Record<string, unknown>, lake: boolean | 'without' = false) {
  let saved: Record<string, unknown> | undefined = seed;
  const drivers = () => ({ items: lake === 'without' ? [{ id: 'postgresql' }] : [{ id: 'postgresql' }, LAKE_DRIVER] });
  const platform = fakePlatform({
    'GET /namespaces': () => ({ items: [{ id: 'ns-1', slug: 'payments', environment: 'prod' }, { id: 'ns-2', slug: 'other', environment: 'dev' }] }),
    'GET /namespaces/ns-1/connections': () => ({ items: saved ? [saved] : [] }),
    'GET /namespaces/ns-2/connections': () => ({ items: [] }),
    ...(lake ? { 'GET /namespaces/ns-1/database-drivers': drivers, 'GET /namespaces/ns-2/database-drivers': drivers } : {}),
    'DELETE /namespaces/ns-1/connections/orders-db': () => { saved = undefined; return { status: 'DELETED' }; },
    'GET /namespaces/ns-1/secrets': () => ({ items: [{ name: 'existing-access', version: 1, provider: 'openbao' }, { name: 'legacy-access', version: 1, provider: 'local' }] }),
    'PUT *': (init, url) => {
      const body = JSON.parse(init.body as string) as Record<string, unknown>;
      if (url.includes('/secrets/') && failure === 'provider') return { name: url.split('/').at(-1), version: 1, provider: 'local', keyId: null };
      if (url.includes('/secrets/')) return failure === 'secret'
        ? new Response(JSON.stringify({ code: 'UNAVAILABLE', message: canary }), { status: 503 })
        : { name: url.split('/').at(-1), version: 1, provider: 'openbao', keyId: null };
      if (failure === 'connection') return new Response(JSON.stringify({ code: 'REVISION_CONFLICT', message: 'Refresh the connection and retry.' }), { status: 409 });
      saved = { ...body, id: 'connection-1', name: url.split('/').at(-1), revision: Number(body.expectedRevision) + 1 }; return saved;
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
  it('lists a lake connection it has no form for by its driver and offers neither an edit nor a probe for it', async () => {
    const lake = {
      id: 'connection-2', namespaceId: 'ns-1', name: 'lake-main', revision: 3, status: 'ACTIVE', createdAt: '2026-11-04T10:00:00Z', updatedAt: '2026-11-04T10:00:00Z',
      config: { driver: 'iceberg', catalog: { type: 'glue', glue: { region: 'us-east-1', catalogId: '123456789012' } }, storage: { type: 's3', s3: { region: 'us-east-1' }, credentials: 'static' }, warehouseLocations: ['s3://warehouse/'], catalogSecret: 'catalog-access' },
    };
    await mount(undefined, ['namespace:read', 'database:manage', 'run:read', 'secret:read'], lake);
    await screen.findByText('lake-main');
    expect(screen.getByText('iceberg')).toBeInTheDocument();
    expect(screen.getByText('catalog-access')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Delete connection lake-main' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Edit connection lake-main' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Probe connection lake-main' })).not.toBeInTheDocument();
    expect(document.body.textContent).not.toContain('undefined');
  });
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

const clientSecret = 'catalog-client-secret-private';
const secretKey = 'storage-secret-key-private';
type User = ReturnType<typeof userEvent.setup>;
const LAKE_PERMISSIONS = ['namespace:read', 'database:manage', 'secret:read', 'secret:manage'];
async function openLake(user: User) {
  await user.click(await screen.findByRole('button', { name: 'New connection' }));
  await user.selectOptions(await screen.findByLabelText('Connection type'), 'iceberg');
}
async function fillLake(user: User) {
  await user.type(screen.getByLabelText('Name'), 'lake-main');
  await user.type(screen.getByLabelText('Catalog URL'), 'https://catalog.example.com');
  await user.type(screen.getByLabelText('Warehouse'), 'analytics');
  await user.type(screen.getByLabelText('Token URL'), 'https://auth.example.com/token');
  await user.type(screen.getByLabelText('Region'), 'us-east-1');
  await user.type(screen.getByLabelText('Warehouse locations'), 's3://lake/analytics/');
}
async function catalogSecret(user: User) {
  await user.selectOptions(screen.getByLabelText('Catalog credentials'), 'new');
  await user.type(screen.getByLabelText('Catalog client id'), 'catalog-client');
  await user.type(screen.getByLabelText('Catalog client secret'), clientSecret);
}
async function storageSecret(user: User) {
  await user.selectOptions(screen.getByLabelText('Storage credentials'), 'new');
  await user.type(screen.getByLabelText('Storage access key id'), 'AKIAEXAMPLE');
  await user.type(screen.getByLabelText('Storage secret access key'), secretKey);
}
async function newSecrets(user: User) { await catalogSecret(user); await storageSecret(user); }
const putsOf = (platform: Awaited<ReturnType<typeof mount>>['platform']) => platform.calls.filter(c => c.method === 'PUT');
describe('Iceberg connections', () => {
  it('offers the kind only when the deployment lists the driver', async () => {
    const { user } = await mount(); await user.click(await screen.findByRole('button', { name: 'New connection' }));
    await screen.findByLabelText('Host');
    expect(screen.queryByLabelText('Connection type')).not.toBeInTheDocument();
  });
  it('writes the catalog and storage credentials to OpenBao first and saves only their references', async () => {
    const { user, platform, app } = await mount(undefined, LAKE_PERMISSIONS, undefined, true); await openLake(user); await fillLake(user); await newSecrets(user);
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await screen.findByRole('button', { name: 'Edit connection lake-main' });
    const writes = putsOf(platform);
    expect(writes).toHaveLength(3);
    expect(writes.slice(0, 2).every(c => c.url.includes('/secrets/lake-'))).toBe(true);
    expect(writes[0]?.init.body).toContain(clientSecret);
    expect(JSON.parse(writes[0]?.init.body as string)).toMatchObject({ requiredProvider: 'openbao', expectedVersion: 0, requestId: expect.any(String) as unknown });
    expect(writes[1]?.init.body).toContain(secretKey);
    const connection = JSON.parse(writes[2]?.init.body as string) as { config: Record<string, unknown> };
    expect(connection).toMatchObject({ expectedRevision: 0, status: 'PAUSED', config: {
      driver: 'iceberg', allowedModes: ['append'], egressCeiling: 'internal', warehouseLocations: ['s3://lake/analytics/'],
      catalog: { type: 'rest', rest: { uri: 'https://catalog.example.com', warehouse: 'analytics', auth: { type: 'oauth2', tokenUri: 'https://auth.example.com/token' }, accessDelegation: 'none' } },
      storage: { type: 's3', s3: { region: 'us-east-1' }, credentials: 'static' },
    } });
    expect(connection.config).toMatchObject({ catalogSecret: expect.stringMatching(/^lake-catalog-/) as unknown, storageSecret: expect.stringMatching(/^lake-storage-/) as unknown });
    expect(writes[2]?.init.body).not.toContain(clientSecret);
    expect(connection.config).not.toHaveProperty('limits');
    expect(writes[2]?.init.body).not.toContain(secretKey);
    expect(document.body.textContent).not.toContain(clientSecret);
    expect(JSON.stringify(app.queries.getQueryCache().getAll().map(q => q.state.data))).not.toContain(secretKey);
  });
  it('shows no credential value when the connection is opened again, only the references', async () => {
    const { user } = await mount(undefined, LAKE_PERMISSIONS, undefined, true); await openLake(user); await fillLake(user); await newSecrets(user);
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await user.click(await screen.findByRole('button', { name: 'Edit connection lake-main' }));
    expect(screen.getByLabelText('Catalog URL')).toHaveValue('https://catalog.example.com');
    expect(screen.getByLabelText('Name')).toBeDisabled();
    expect(screen.getByLabelText<HTMLSelectElement>('Catalog secret reference').value).toMatch(/^lake-catalog-/);
    expect(screen.getByLabelText<HTMLSelectElement>('Storage secret reference').value).toMatch(/^lake-storage-/);
    expect(screen.queryByLabelText('Catalog client secret')).not.toBeInTheDocument();
    expect(document.body.textContent).not.toContain(clientSecret);
    expect(document.body.textContent).not.toContain(secretKey);
    expect(screen.getByText(/Saving settings does not test connectivity/)).toBeInTheDocument();
  });
  it('reuses the saved secrets and the connection request identity after the connection fails to save', async () => {
    const { user, platform } = await mount('connection', LAKE_PERMISSIONS, undefined, true); await openLake(user); await fillLake(user); await newSecrets(user);
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await screen.findByText(/Credentials saved/);
    expect(screen.queryByLabelText('Catalog client secret')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await waitFor(() => expect(putsOf(platform)).toHaveLength(4));
    const writes = putsOf(platform);
    expect(writes.filter(c => c.url.includes('/secrets/'))).toHaveLength(2);
    expect(writes[2]?.init.body).toBe(writes[3]?.init.body);
  });
  it('retries an uncertain secret write with the same receipt and hides what the provider said', async () => {
    const { user, platform } = await mount('secret', LAKE_PERMISSIONS, undefined, true); await openLake(user); await fillLake(user); await newSecrets(user);
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await screen.findByText(/Credentials could not be saved/);
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await waitFor(() => expect(putsOf(platform)).toHaveLength(2));
    const writes = putsOf(platform);
    expect(writes[0]?.init.body).toBe(writes[1]?.init.body);
    expect(writes.every(c => c.url.includes('/secrets/'))).toBe(true);
    expect(document.body.textContent).not.toContain(canary);
  });
  it('takes vended storage credentials only from a catalog that delegates access, and then asks for no storage secret', async () => {
    const { user, platform } = await mount(undefined, LAKE_PERMISSIONS, undefined, true); await openLake(user); await fillLake(user);
    expect(screen.queryByRole('option', { name: 'Credentials vended by the catalog' })).not.toBeInTheDocument();
    await user.click(screen.getByLabelText('The catalog vends storage credentials'));
    await user.selectOptions(screen.getByLabelText('Storage access'), 'vended');
    expect(screen.queryByLabelText('Storage credentials')).not.toBeInTheDocument();
    await catalogSecret(user);
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await screen.findByRole('button', { name: 'Edit connection lake-main' });
    const config = (JSON.parse(putsOf(platform)[1]?.init.body as string) as { config: Record<string, unknown> }).config;
    expect(config).toMatchObject({ catalog: { rest: { accessDelegation: 'vended-credentials' } }, storage: { credentials: 'vended' } });
    expect(config).not.toHaveProperty('storageSecret');
    await user.click(screen.getByRole('button', { name: 'Edit connection lake-main' }));
    await user.click(screen.getByLabelText('The catalog vends storage credentials'));
    expect(screen.getByLabelText('Storage access')).toHaveValue('static');
    expect(screen.getByLabelText('Storage credentials')).toBeInTheDocument();
  });
  it('disables the pii ceiling for an actor without the reveal permission and says why', async () => {
    const { user } = await mount(undefined, LAKE_PERMISSIONS, undefined, true); await openLake(user);
    expect(screen.getByRole('option', { name: 'Personal data (pii)' })).toBeDisabled();
    expect(screen.getByText(/you do not hold/)).toBeInTheDocument();
  });
  it('lets an actor with the reveal permission choose the pii ceiling and sends it', async () => {
    const { user, platform } = await mount(undefined, [...LAKE_PERMISSIONS, 'payload:reveal-pii'], undefined, true); await openLake(user); await fillLake(user);
    await user.selectOptions(screen.getByLabelText('Highest data level a step may write'), 'pii');
    await newSecrets(user);
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await screen.findByRole('button', { name: 'Edit connection lake-main' });
    expect(JSON.parse(putsOf(platform)[2]?.init.body as string)).toMatchObject({ config: { egressCeiling: 'pii' } });
  });
  it('refuses a plaintext catalog outside development, a credential in a URL and a location of another store before saving', async () => {
    const { user, platform } = await mount(undefined, LAKE_PERMISSIONS, undefined, true); await openLake(user); await fillLake(user); await newSecrets(user);
    const save = screen.getByRole('button', { name: 'Save connection' });
    expect(save).toBeEnabled();
    const url = screen.getByLabelText('Catalog URL');
    await user.clear(url); await user.type(url, 'http://catalog.example.com');
    expect(screen.getByText('Use an https URL.')).toBeInTheDocument(); expect(save).toBeDisabled();
    await user.clear(url); await user.type(url, 'https://reader:hunter2@catalog.example.com');
    expect(screen.getByText(/Credentials belong in a secret/)).toBeInTheDocument(); expect(save).toBeDisabled();
    await user.clear(url); await user.type(url, 'https://catalog.example.com');
    expect(save).toBeEnabled();
    const locations = screen.getByLabelText('Warehouse locations');
    await user.clear(locations); await user.type(locations, 'gs://lake/analytics/');
    expect(screen.getByText(/Enter 1 to 16 distinct s3:\/\/ prefixes/)).toBeInTheDocument(); expect(save).toBeDisabled();
    expect(putsOf(platform)).toHaveLength(0);
  });
  it('needs at least one write mode and sends the modes in the contract order', async () => {
    const { user, platform } = await mount(undefined, LAKE_PERMISSIONS, undefined, true); await openLake(user); await fillLake(user); await newSecrets(user);
    await user.click(screen.getByLabelText('append'));
    expect(screen.getByText('Allow at least one mode.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save connection' })).toBeDisabled();
    await user.click(screen.getByLabelText('upsert')); await user.click(screen.getByLabelText('append'));
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await screen.findByRole('button', { name: 'Edit connection lake-main' });
    expect(JSON.parse(putsOf(platform)[2]?.init.body as string)).toMatchObject({ config: { allowedModes: ['append', 'upsert'] } });
  });
  it('does not offer to create a secret to an actor who cannot manage secrets', async () => {
    const { user } = await mount(undefined, ['namespace:read', 'database:manage', 'secret:read'], undefined, true); await openLake(user);
    expect(screen.queryByRole('option', { name: 'Create a secret' })).not.toBeInTheDocument();
  });
});

const SAVED_LAKE = {
  id: 'connection-3', namespaceId: 'ns-1', name: 'lake-main', revision: 4, status: 'ACTIVE', createdAt: '2026-11-04T10:00:00Z', updatedAt: '2026-11-04T10:00:00Z',
  config: {
    driver: 'iceberg',
    catalog: { type: 'rest', rest: { uri: 'https://catalog.example.com', warehouse: 'analytics', prefix: 'prod', auth: { type: 'bearer' }, accessDelegation: 'vended-credentials', remoteSigning: true } },
    storage: { type: 's3', s3: { region: 'us-east-1', pathStyle: true }, credentials: 'vended' },
    warehouseLocations: ['s3://lake/a/', 's3://lake/b/'], allowedModes: ['append', 'overwrite'], catalogSecret: 'existing-access', egressCeiling: 'public', limits: { maxRecords: 5000 },
  },
};
describe('Editing an Iceberg connection', () => {
  it('shows what was saved and saves it back unchanged, against the revision it read', async () => {
    const { user, platform } = await mount(undefined, LAKE_PERMISSIONS, SAVED_LAKE, true);
    await user.click(await screen.findByRole('button', { name: 'Edit connection lake-main' }));
    expect(screen.getByLabelText('Catalog authentication')).toHaveValue('bearer');
    expect(screen.getByLabelText('Prefix (optional)')).toHaveValue('prod');
    expect(screen.getByLabelText('Storage access')).toHaveValue('vended');
    expect(screen.getByLabelText('Warehouse locations')).toHaveValue('s3://lake/a/\ns3://lake/b/');
    expect(screen.getByLabelText('Catalog secret reference')).toHaveValue('existing-access');
    expect(screen.getByLabelText('Highest data level a step may write')).toHaveValue('public');
    expect(screen.getByLabelText('Maximum records')).toHaveValue(5000);
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await waitFor(() => expect(putsOf(platform)).toHaveLength(1));
    expect(JSON.parse(putsOf(platform)[0]?.init.body as string)).toEqual({ requestId: expect.any(String) as unknown, expectedRevision: 4, status: 'ACTIVE', config: SAVED_LAKE.config });
  });
  it('does not let an actor without the reveal permission save a connection whose ceiling is pii', async () => {
    const pii = { ...SAVED_LAKE, config: { ...SAVED_LAKE.config, egressCeiling: 'pii' } };
    const { user } = await mount(undefined, LAKE_PERMISSIONS, pii, true);
    await user.click(await screen.findByRole('button', { name: 'Edit connection lake-main' }));
    expect(screen.getByLabelText('Highest data level a step may write')).toHaveValue('pii');
    expect(screen.getByRole('button', { name: 'Save connection' })).toBeDisabled();
  });
});
describe('Iceberg connection input rules', () => {
  it('refuses credentials in the query of a URL and a repeated warehouse location', async () => {
    const { user } = await mount(undefined, LAKE_PERMISSIONS, undefined, true); await openLake(user); await fillLake(user); await newSecrets(user);
    const save = screen.getByRole('button', { name: 'Save connection' });
    const url = screen.getByLabelText('Catalog URL');
    await user.clear(url); await user.type(url, 'https://catalog.example.com?token=abc');
    expect(screen.getByText(/Credentials belong in a secret/)).toBeInTheDocument(); expect(save).toBeDisabled();
    await user.clear(url); await user.type(url, 'https://catalog.example.com');
    const locations = screen.getByLabelText('Warehouse locations');
    await user.clear(locations); await user.type(locations, 's3://lake/a/{enter}s3://lake/a/');
    expect(screen.getByText(/Enter 1 to 16 distinct s3:\/\/ prefixes/)).toBeInTheDocument(); expect(save).toBeDisabled();
  });
  it('allows a plaintext catalog only in a development namespace', async () => {
    const { user, app } = await mount(undefined, LAKE_PERMISSIONS, undefined, true);
    act(() => app.namespace.set('other'));
    await openLake(user); await fillLake(user); await newSecrets(user);
    const url = screen.getByLabelText('Catalog URL');
    await user.clear(url); await user.type(url, 'http://localhost:8181');
    expect(screen.queryByText('Use an https URL.')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save connection' })).toBeEnabled();
  });
  it('treats a secret authority that is not OpenBao as a failure and saves no connection', async () => {
    const { user, platform } = await mount('provider', LAKE_PERMISSIONS, undefined, true); await openLake(user); await fillLake(user); await newSecrets(user);
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await screen.findByText(/Credentials could not be saved/);
    expect(putsOf(platform).every(c => c.url.includes('/secrets/'))).toBe(true);
  });
  it('writes a secret whose values changed under a new receipt', async () => {
    const { user, platform } = await mount('secret', LAKE_PERMISSIONS, undefined, true); await openLake(user); await fillLake(user); await newSecrets(user);
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await screen.findByText(/Credentials could not be saved/);
    await user.type(screen.getByLabelText('Catalog client id'), '-2');
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await waitFor(() => expect(putsOf(platform)).toHaveLength(2));
    const [first, second] = putsOf(platform).map(c => JSON.parse(c.init.body as string) as { requestId: string });
    expect(first?.requestId).not.toBe(second?.requestId);
  });
});

describe('Choosing the kind of a connection', () => {
  it('offers PostgreSQL alone when the deployment lists no lake driver', async () => {
    const { user } = await mount(undefined, undefined, undefined, 'without'); await user.click(await screen.findByRole('button', { name: 'New connection' }));
    await screen.findByLabelText('Host');
    expect(screen.queryByLabelText('Connection type')).not.toBeInTheDocument();
  });
  it('keeps the kind of a PostgreSQL connection it edits, even where the lake is offered', async () => {
    const postgres = {
      id: 'connection-4', namespaceId: 'ns-1', name: 'orders-db', revision: 2, status: 'ACTIVE', createdAt: '2026-11-04T10:00:00Z', updatedAt: '2026-11-04T10:00:00Z',
      config: { driver: 'postgresql', host: 'db.example.com', port: 5432, database: 'orders', tls: { mode: 'verify-full' }, secretName: 'existing-access', limits: {} },
    };
    const { user } = await mount(undefined, undefined, postgres, true);
    await user.click(await screen.findByRole('button', { name: 'Edit connection orders-db' }));
    await screen.findByLabelText('Host');
    expect(screen.queryByLabelText('Connection type')).not.toBeInTheDocument();
  });
});

describe('Iceberg connection rules the first tests missed', () => {
  it('writes no storage secret for credentials the catalog will vend, even if one was typed first', async () => {
    const { user, platform } = await mount(undefined, LAKE_PERMISSIONS, undefined, true); await openLake(user); await fillLake(user);
    await storageSecret(user);
    await user.click(screen.getByLabelText('The catalog vends storage credentials'));
    await user.selectOptions(screen.getByLabelText('Storage access'), 'vended');
    await catalogSecret(user);
    await user.click(screen.getByRole('button', { name: 'Save connection' }));
    await screen.findByRole('button', { name: 'Edit connection lake-main' });
    const writes = putsOf(platform);
    expect(writes).toHaveLength(2);
    expect(writes.some(c => (c.init.body as string).includes(secretKey))).toBe(false);
    expect(JSON.parse(writes[1]?.init.body as string)).toMatchObject({ config: { storage: { credentials: 'vended' } } });
  });
  it('refuses more than sixteen warehouse locations and an empty catalog URL', async () => {
    const { user } = await mount(undefined, LAKE_PERMISSIONS, undefined, true); await openLake(user); await fillLake(user); await newSecrets(user);
    const save = screen.getByRole('button', { name: 'Save connection' });
    expect(save).toBeEnabled();
    const locations = screen.getByLabelText('Warehouse locations');
    await user.clear(locations); await user.click(locations);
    await user.paste(Array.from({ length: 17 }, (_, i) => `s3://lake/${i}/`).join('\n'));
    expect(screen.getByText(/Enter 1 to 16 distinct s3:\/\/ prefixes/)).toBeInTheDocument(); expect(save).toBeDisabled();
    await user.clear(locations); await user.click(locations);
    await user.paste(Array.from({ length: 16 }, (_, i) => `s3://lake/${i}/`).join('\n'));
    expect(save).toBeEnabled();
    await user.clear(screen.getByLabelText('Catalog URL'));
    expect(save).toBeDisabled();
  });
  it('offers no kind to choose when it edits a PostgreSQL connection after a new connection fetched the driver list', async () => {
    const postgres = {
      id: 'connection-4', namespaceId: 'ns-1', name: 'orders-db', revision: 2, status: 'ACTIVE', createdAt: '2026-11-04T10:00:00Z', updatedAt: '2026-11-04T10:00:00Z',
      config: { driver: 'postgresql', host: 'db.example.com', port: 5432, database: 'orders', tls: { mode: 'verify-full' }, secretName: 'existing-access', limits: {} },
    };
    const { user } = await mount(undefined, undefined, postgres, true);
    await user.click(await screen.findByRole('button', { name: 'New connection' }));
    await screen.findByLabelText('Connection type');
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    await user.click(await screen.findByRole('button', { name: 'Edit connection orders-db' }));
    await screen.findByLabelText('Host');
    expect(screen.queryByLabelText('Connection type')).not.toBeInTheDocument();
  });
});
