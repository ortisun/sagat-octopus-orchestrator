import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Route, Routes } from 'react-router';
import { describe, expect, it } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../test/render';
import { createApp } from './context';
import { Landing, NamespaceGate } from './NamespaceGate';
import { Shell } from './Shell';

const namespace = { id: '019b5d6e-0000-7000-8000-000000000001', workspaceId: '019b5d6e-0000-7000-8000-000000000002', slug: 'production', environment: 'prod', createdAt: '2026-10-01T00:00:00Z' };

async function mount(path: string, { chosen = null, permissions }: { chosen?: string | null; permissions?: string[] } = {}) {
  const platform = fakePlatform({ 'GET /namespaces': () => ({ items: [namespace] }) });
  const app = createApp({ fetch: platform.fetcher });
  app.namespace.set(chosen ?? undefined);
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, ...(permissions ? { permissions } : {}) } as never));
  renderApp(
    <Routes>
      <Route path="/" element={<Landing />} />
      <Route element={<Shell />}>
        <Route path="namespaces" element={<h1>Namespace list</h1>} />
        <Route element={<NamespaceGate />}>
          <Route path="workflows" element={<h1>Workflow library</h1>} />
          <Route path="runs/:runId" element={<h1>One run</h1>} />
        </Route>
      </Route>
    </Routes>,
    { app, path },
  );
  return { app, user: userEvent.setup() };
}

describe('the pages of a namespace', () => {
  it('wait for a namespace to be chosen, and say how', async () => {
    await mount('/workflows');
    expect(await screen.findByRole('heading', { name: 'Choose a namespace', level: 1 })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Workflow library' })).not.toBeInTheDocument();
    expect(within(screen.getByRole('main')).getByRole('link', { name: 'Open the namespace list' })).toHaveAttribute('href', '/namespaces');
  });

  it('wait on a deep link too, and open it as soon as a namespace is chosen in the menu', async () => {
    const { user, app } = await mount('/runs/019b5d6e-0000-7000-8000-0000000000aa');
    expect(await screen.findByRole('heading', { name: 'Choose a namespace' })).toBeInTheDocument();
    await user.selectOptions(await screen.findByLabelText('Namespace'), 'production');
    expect(app.namespace.current()).toBe('production');
    expect(await screen.findByRole('heading', { name: 'One run' })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Choose a namespace' })).not.toBeInTheDocument();
  });

  it('show their page when a namespace is already chosen', async () => {
    await mount('/workflows', { chosen: 'production' });
    expect(await screen.findByRole('heading', { name: 'Workflow library' })).toBeInTheDocument();
  });

  it('mention creating one only to somebody who may', async () => {
    await mount('/workflows', { permissions: ['namespace:create'] });
    expect(await screen.findByText(/or to create a new one/)).toBeInTheDocument();
  });

  it('do not mention creating one to somebody who may not', async () => {
    await mount('/workflows', { permissions: ['definition:read'] });
    await screen.findByRole('heading', { name: 'Choose a namespace' });
    expect(screen.queryByText(/or to create a new one/)).not.toBeInTheDocument();
  });
});

describe('where the Studio opens', () => {
  it('is the list of namespaces on a first visit', async () => {
    await mount('/');
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Namespace list' })).toBeInTheDocument());
  });

  it('is the workflows of the namespace the person works in', async () => {
    await mount('/', { chosen: 'production' });
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Workflow library' })).toBeInTheDocument());
  });
});
