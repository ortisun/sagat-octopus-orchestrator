import { act, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import { createApp } from '../../app/context';
import type { ConnectorInput } from './api';
import { IntegrationsPage } from './IntegrationsPage';

const canary = 'private-password-do-not-echo';
async function mount(options: { permissions?: string[]; failure?: 'secret' | 'connector' } = {}) {
  let saved: Record<string, unknown> | undefined;
  let policy = { rules: ['existing.example.com'], ports: [443], allowHttp: false };
  const platform = fakePlatform({
    'GET /namespaces': () => ({ items: [{ id: 'ns-1', slug: 'payments' }, { id: 'ns-2', slug: 'other' }] }),
    'GET /namespaces/ns-1/connectors': () => ({ items: saved ? [saved] : [] }),
    'GET /namespaces/ns-2/connectors': () => ({ items: [] }),
    'GET /namespaces/ns-1/egress-policy': () => policy,
    'PUT /namespaces/ns-1/egress-policy': (init) => { policy = JSON.parse(init.body as string) as typeof policy; return policy; },
    'PUT /namespaces/ns-1/connectors/conn-1': (init) => { saved = { ...saved, ...(JSON.parse(init.body as string) as Record<string, unknown>) }; return saved; },
    'PUT *': () => options.failure === 'secret' ? new Response(JSON.stringify({ code: 'INVALID_ARGUMENT', message: canary }), { status: 422, headers: { 'content-type': 'application/json' } }) : { name: 'saved', version: 1 },
    'POST /namespaces/ns-1/connectors': (init) => {
      if (options.failure === 'connector') return new Response(JSON.stringify({ code: 'CONFLICT', message: 'Name already exists.' }), { status: 409, headers: { 'content-type': 'application/json' } });
      saved = { ...(JSON.parse(init.body as string) as ConnectorInput), id: 'conn-1', namespaceId: 'ns-1', health: { consumed: 0, deadLettered: 0 } }; return saved;
    },
  });
  const app = createApp({ fetch: platform.fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions: options.permissions ?? ['event:read', 'event:manage', 'secret:manage'] } as never));
  app.namespace.set('payments');
  renderApp(<IntegrationsPage />, { app });
  return { app, platform, user: userEvent.setup() };
}
async function kafka(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole('button', { name: 'New integration' }));
  await user.type(screen.getByLabelText('Name'), 'orders-kafka');
  await user.type(screen.getByLabelText('Servers'), 'tls://broker.example.com:9093');
  await user.type(screen.getByLabelText('Topic'), 'orders');
}
async function credentials(user: ReturnType<typeof userEvent.setup>) {
  await user.selectOptions(screen.getByLabelText('Authentication'), 'sasl');
  await user.type(screen.getByLabelText('Username'), 'service-user');
  await user.type(screen.getByLabelText('Password'), canary);
}

describe('Integrations', () => {
  it('stores credentials only through the secrets API, then attaches the reference to a paused connector', async () => {
    const { user, platform, app } = await mount(); await kafka(user); await credentials(user);
    await user.click(screen.getByRole('button', { name: 'Save integration' }));
    await screen.findByRole('button', { name: 'Edit integration orders-kafka' });
    const writes = platform.calls.filter((call) => call.method !== 'GET');
    expect(writes.map((call) => call.method)).toEqual(['PUT', 'POST']);
    expect(writes[0]?.url).toContain('/namespaces/ns-1/secrets/integration-');
    expect(JSON.parse(writes[0]?.init.body as string)).toMatchObject({ requestId: expect.any(String), expectedVersion: 0 });
    expect(JSON.parse((JSON.parse(writes[0]?.init.body as string) as { value: string }).value) as unknown).toEqual({ mechanism: 'SCRAM-SHA-256', username: 'service-user', password: canary });
    const body = JSON.parse(writes[1]?.init.body as string) as ConnectorInput;
    expect(body).toMatchObject({ kind: 'KAFKA', status: 'PAUSED', settings: { topic: 'orders', servers: ['tls://broker.example.com:9093'], startFrom: 'earliest' } });
    expect(body.secretName).toMatch(/^integration-/);
    expect(writes[1]?.init.body).not.toContain(canary);
    expect(JSON.stringify(app.queries.getQueryData(['integrations', 'ns-1']))).not.toContain(canary);
    expect(document.body.textContent).not.toContain(canary);
    await user.click(screen.getByRole('button', { name: 'Edit integration orders-kafka' }));
    expect(screen.getByLabelText('Authentication')).toHaveValue('existing');
    expect(screen.queryByLabelText('Password')).not.toBeInTheDocument();
  });
  it('supports NATS-specific settings without credentials in public settings', async () => {
    const { user, platform } = await mount(); await kafka(user);
    await user.selectOptions(screen.getByLabelText('Type'), 'NATS');
    expect(screen.queryByLabelText('Topic')).not.toBeInTheDocument();
    await user.type(screen.getByLabelText('Stream'), 'ORDERS');
    await user.type(screen.getByLabelText('Durable consumer'), 'sagat');
    await user.click(screen.getByRole('button', { name: 'Save integration' }));
    await waitFor(() => expect(platform.calls.some((call) => call.method === 'POST')).toBe(true));
    const body = JSON.parse(platform.calls.find((call) => call.method === 'POST')?.init.body as string) as ConnectorInput;
    expect(body.settings).toEqual({ servers: ['tls://broker.example.com:9093'], stream: 'ORDERS', consumer: 'sagat' });
    expect(body.secretName).toBeUndefined();
  });
  it('refuses embedded URL credentials and plaintext credential transport', async () => {
    const { user, platform } = await mount(); await kafka(user); await credentials(user);
    await user.clear(screen.getByLabelText('Servers')); await user.type(screen.getByLabelText('Servers'), 'user:password@broker.example.com:9092');
    expect(screen.getByRole('button', { name: 'Save integration' })).toBeDisabled();
    await user.clear(screen.getByLabelText('Servers')); await user.type(screen.getByLabelText('Servers'), 'broker.example.com:9092');
    expect(screen.getByRole('alert')).toHaveTextContent('Use TLS');
    expect(screen.getByRole('button', { name: 'Save integration' })).toBeDisabled();
    expect(platform.calls.every((call) => call.method === 'GET')).toBe(true);
  });
  it('does not expose credential-request errors or proceed when storing credentials fails', async () => {
    const { user, platform } = await mount({ failure: 'secret' }); await kafka(user); await credentials(user);
    await user.click(screen.getByRole('button', { name: 'Save integration' }));
    expect(await screen.findByText(/Credentials could not be saved/)).toBeVisible();
    expect(document.body.textContent).not.toContain(canary);
    expect(platform.calls.some((call) => call.method === 'POST')).toBe(false);
  });
  it('reuses the credential receipt on retry and allocates another after editing the value', async () => {
    const { user, platform } = await mount({ failure: 'secret' }); await kafka(user); await credentials(user);
    await user.click(screen.getByRole('button', { name: 'Save integration' }));
    await screen.findByText(/Credentials could not be saved/);
    await user.click(screen.getByRole('button', { name: 'Save integration' }));
    await screen.findByText(/Credentials could not be saved/);
    let writes = platform.calls.filter((call) => call.method === 'PUT');
    expect(writes).toHaveLength(2);
    expect(writes[0]?.url).toBe(writes[1]?.url);
    expect(writes[0]?.init.body).toBe(writes[1]?.init.body);
    await user.type(screen.getByLabelText('Password'), '-changed');
    await user.click(screen.getByRole('button', { name: 'Save integration' }));
    await screen.findByText(/Credentials could not be saved/);
    writes = platform.calls.filter((call) => call.method === 'PUT');
    expect(writes[2]?.url).not.toBe(writes[1]?.url);
    expect(JSON.parse(writes[2]?.init.body as string).requestId).not.toBe(JSON.parse(writes[1]?.init.body as string).requestId);
    expect(document.body.textContent).not.toContain(canary);
  });
  it('retains only the saved reference when connector creation fails and reuses it on retry', async () => {
    const { user, platform } = await mount({ failure: 'connector' }); await kafka(user); await credentials(user);
    await user.click(screen.getByRole('button', { name: 'Save integration' }));
    await screen.findByText(/Credentials saved/);
    await screen.findByRole('alert');
    expect(screen.queryByLabelText('Password')).not.toBeInTheDocument();
    expect(screen.getByLabelText<HTMLInputElement>('Secret reference').value).toMatch(/^integration-/);
    await user.click(screen.getByRole('button', { name: 'Save integration' }));
    await waitFor(() => expect(platform.calls.filter((call) => call.method === 'POST')).toHaveLength(2));
    expect(platform.calls.filter((call) => call.method === 'PUT')).toHaveLength(1);
  });
  it('hides editing and secret inputs according to permissions', async () => {
    await mount({ permissions: ['event:read'] });
    await screen.findByText('No integrations configured yet.');
    expect(screen.queryByRole('button', { name: 'New integration' })).not.toBeInTheDocument();
  });
  it('allows managing existing references without granting secret writes', async () => {
    const { user } = await mount({ permissions: ['event:read', 'event:manage'] }); await kafka(user);
    expect(screen.queryByRole('option', { name: 'SASL username and password' })).not.toBeInTheDocument();
    expect(screen.getByRole('option', { name: 'Existing secret reference' })).toBeInTheDocument();
  });
  it('discards the open form when changing namespace', async () => {
    const { user, app, platform } = await mount(); await kafka(user); await credentials(user);
    act(() => app.namespace.set('other'));
    await screen.findByText('No integrations configured yet.');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Password')).not.toBeInTheDocument();
    expect(platform.calls.some((call) => call.method !== 'GET')).toBe(false);
  });
});

 it('updates connection settings and status without reading or rewriting credentials', async () => {
    const { user, platform } = await mount(); await kafka(user);
    await user.click(screen.getByRole('button', { name: 'Save integration' }));
    await user.click(await screen.findByRole('button', { name: 'Edit integration orders-kafka' }));
    await user.clear(screen.getByLabelText('Topic')); await user.type(screen.getByLabelText('Topic'), 'new-orders');
    await user.selectOptions(screen.getByLabelText('Status'), 'ACTIVE');
    await user.click(screen.getByRole('button', { name: 'Save integration' }));
    await screen.findByText('Active');
    const call = platform.calls.find((item) => item.url.endsWith('/connectors/conn-1'));
    expect(JSON.parse(call?.init.body as string) as unknown).toMatchObject({ settings: { topic: 'new-orders' }, status: 'ACTIVE' });
    expect(platform.calls.some((item) => item.url.includes('/secrets'))).toBe(false);
 });
 it('keeps existing network rules visible and saves only an explicit administrator change', async () => {
    const { user, platform } = await mount({ permissions: ['event:read', 'event:manage', 'egress:manage'] });
    await user.click(await screen.findByText('Network access'));
    expect(await screen.findByLabelText('Allowed hosts or networks')).toHaveValue('existing.example.com');
    await user.type(screen.getByLabelText('Allowed hosts or networks'), '\nbroker.example.com');
    await user.type(screen.getByLabelText('Allowed ports'), ', 9093');
    await user.click(screen.getByRole('button', { name: 'Save network access' }));
    await screen.findByText('Network access saved.');
    const call = platform.calls.find((item) => item.method === 'PUT');
    expect(JSON.parse(call?.init.body as string) as unknown).toEqual({ rules: ['existing.example.com', 'broker.example.com'], ports: [443, 9093], allowHttp: false });
 });
 it('serializes AWS credentials separately from queue settings', async () => {
    const { user, platform } = await mount(); await kafka(user);
    await user.selectOptions(screen.getByLabelText('Type'), 'SQS');
    await user.type(screen.getByLabelText('Queue URL'), 'https://sqs.us-east-1.amazonaws.com/123/orders');
    await user.type(screen.getByLabelText('AWS region'), 'us-east-1');
    await user.selectOptions(screen.getByLabelText('Authentication'), 'aws');
    await user.type(screen.getByLabelText('Access key ID'), 'test-key');
    await user.type(screen.getByLabelText('Secret access key'), canary);
    await user.click(screen.getByRole('button', { name: 'Save integration' }));
    await screen.findByRole('button', { name: 'Edit integration orders-kafka' });
    expect(platform.calls.find((item) => item.method === 'POST')?.init.body).not.toContain(canary);
    expect(platform.calls.find((item) => item.method === 'PUT')?.init.body).toContain('secretAccessKey');
 });
