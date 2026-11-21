import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../test/render';
import { choose, chosen, expectUnavailable } from '../../test/ui';
import { createApp } from './context';
import { Shell } from './Shell';
import { NamespacesPage } from './NamespacesPage';
import { Route, Routes } from 'react-router';

const workspace = { id: '019b5d6e-0000-7000-8000-000000000002', slug: 'development', createdAt: '2026-10-01T00:00:00Z' };
const created = { id: '019b5d6e-0000-7000-8000-000000000003', workspaceId: workspace.id, slug: 'testing', environment: 'staging', createdAt: workspace.createdAt };
async function mount(options: { allowed?: boolean; failure?: number; empty?: boolean } = {}) {
  const platform = fakePlatform({
    'GET /namespaces': () => ({ items: [] }),
    'GET /workspaces': () => ({ items: options.empty ? [] : [workspace] }),
    [`POST /workspaces/${workspace.id}/namespaces`]: () => options.failure ? new Response(JSON.stringify({ code: options.failure === 409 ? 'CONFLICT' : 'FORBIDDEN', message: 'Request refused.', retryable: false, traceId: 'a'.repeat(32) }), { status: options.failure, headers: { 'content-type': 'application/json' } }) : created,
  });
  const app = createApp({ fetch: platform.fetcher });
  app.namespace.set(undefined);
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions: options.allowed === false ? [] : ['namespace:create'] } as never));
  renderApp(<Routes><Route element={<Shell />}><Route path="/namespaces" element={<NamespacesPage />} /></Route></Routes>, { app, path: '/namespaces' });
  return { app, platform, user: userEvent.setup() };
}

describe('namespace creation', () => {
  it('creates from an empty namespace list and immediately selects the result', async () => {
    const { app, platform, user } = await mount();
    await user.click(screen.getByRole('button', { name: 'New namespace' }));
    await waitFor(() => expect(chosen('Workspace')).toBe('development'));
    await user.type(screen.getByRole('textbox', { name: /^Namespace name/ }), 'testing');
    await choose(user, 'Environment', 'Staging');
    await user.click(screen.getByRole('button', { name: 'Create namespace' }));
    await waitFor(() => expect(app.namespace.current()).toBe('testing'));
    expect(chosen('Namespace')).toBe('testing');
    const call = platform.calls.find((item) => item.method === 'POST');
    expect(JSON.parse(call?.init.body as string)).toEqual({ slug: 'testing', environment: 'staging' });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
  it('does not offer creation without the server permission', async () => {
    const { platform } = await mount({ allowed: false });
    expect(screen.queryByRole('button', { name: 'New namespace' })).not.toBeInTheDocument();
    expect(platform.calls.some((call) => call.url.includes('workspaces'))).toBe(false);
  });
  it('prevents invalid names and resets the form after cancellation', async () => {
    const { user, platform } = await mount();
    await user.click(screen.getByRole('button', { name: 'New namespace' }));
    await waitFor(() => expect(chosen('Workspace')).toBe('development'));
    for (const name of ['ab', 'Bad-name', 'bad--name', 'bad-']) {
      await user.clear(screen.getByRole('textbox', { name: /^Namespace name/ }));
      await user.type(screen.getByRole('textbox', { name: /^Namespace name/ }), name);
      expectUnavailable(screen.getByRole('button', { name: 'Create namespace' }));
    }
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    await user.click(screen.getByRole('button', { name: 'New namespace' }));
    expect(screen.getByRole('textbox', { name: /^Namespace name/ })).toHaveValue('');
    expect(platform.calls.some((call) => call.method === 'POST')).toBe(false);
  });
  it.each([409, 403])('keeps the form and selection when the API refuses with %s', async (failure) => {
    const { user, app } = await mount({ failure });
    await user.click(screen.getByRole('button', { name: 'New namespace' }));
    await waitFor(() => expect(chosen('Workspace')).toBe('development'));
    await user.type(screen.getByRole('textbox', { name: /^Namespace name/ }), 'testing');
    await user.click(screen.getByRole('button', { name: 'Create namespace' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(failure === 409 ? 'already exists' : 'Request refused');
    expect(screen.getByRole('textbox', { name: /^Namespace name/ })).toHaveValue('testing');
    expect(app.namespace.current()).toBeUndefined();
  });
  it('explains when no workspace is available', async () => {
    const { user } = await mount({ empty: true });
    await user.click(screen.getByRole('button', { name: 'New namespace' }));
    expect(await screen.findByText(/Create a workspace through/)).toBeVisible();
    expectUnavailable(screen.getByRole('button', { name: 'Create namespace' }));
  });
});
