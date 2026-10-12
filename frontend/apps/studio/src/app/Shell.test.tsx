import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Route, Routes } from 'react-router';
import { describe, expect, it } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../test/render';
import { createApp } from './context';
import { Shell } from './Shell';

const namespace = { id: '019b5d6e-0000-7000-8000-000000000001', workspaceId: '019b5d6e-0000-7000-8000-000000000002', slug: 'production', environment: 'prod', createdAt: '2026-10-01T00:00:00Z' };

async function mount(path = '/workflows') {
  const platform = fakePlatform({ 'GET /namespaces': () => ({ items: [namespace] }) });
  const app = createApp({ fetch: platform.fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve(AUTH_CONTEXT as never));
  const result = renderApp(
    <Routes>
      <Route element={<Shell />}>
        <Route path="workflows" element={<h1>Workflow library</h1>} />
        <Route path="runs" element={<h1>Run history</h1>} />
        <Route path="scripts" element={<h1>Script library</h1>} />
      </Route>
    </Routes>,
    { app, path },
  );
  await screen.findByRole('option', { name: 'production' });
  return { ...result, app };
}

describe('Shell', () => {
  it('keeps the brand, namespace and only real destinations in the sidebar', async () => {
    await mount('/runs');
    const sidebar = screen.getByRole('complementary', { name: 'Studio navigation' });
    expect(within(sidebar).getByText('sagat-octopus')).toBeInTheDocument();
    expect(within(sidebar).getByLabelText('Namespace')).toHaveValue('production');
    const navigation = within(sidebar).getByRole('navigation', { name: 'Main' });
    expect(within(navigation).getAllByRole('link').map((link) => link.textContent)).toEqual(['Workflows', 'Runs', 'Scripts', 'Integrations', 'Namespaces']);
    expect(within(navigation).getByRole('link', { name: 'Runs' })).toHaveAttribute('aria-current', 'page');
  });

  it('keeps the top bar grounded in the current page and real session', async () => {
    await mount('/scripts');
    const topbar = screen.getByRole('banner');
    expect(topbar).toHaveTextContent('production');
    expect(topbar).toHaveTextContent('Scripts');
    expect(within(topbar).getByLabelText('Signed in as')).toHaveTextContent('alice admin');
    expect(within(topbar).getByRole('button', { name: 'Sign out' })).toBeInTheDocument();
  });

  it('keeps the skip link and moves focus to the page after navigation', async () => {
    const user = userEvent.setup();
    await mount();
    expect(screen.getByRole('link', { name: 'Skip to the content' })).toHaveAttribute('href', '#main');
    await user.click(screen.getByRole('link', { name: 'Runs' }));
    await screen.findByRole('heading', { name: 'Run history' });
    await waitFor(() => expect(screen.getByRole('main')).toHaveFocus());
  });
});

 it('provides a namespace menu instead of a sidebar creation button', async () => {
    await mount();
    expect(screen.getByRole('link', { name: 'Namespaces' })).toHaveAttribute('href', '/namespaces');
    expect(screen.queryByRole('button', { name: 'New namespace' })).not.toBeInTheDocument();
 });
