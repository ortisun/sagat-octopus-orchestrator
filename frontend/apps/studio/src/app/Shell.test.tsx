import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Route, Routes } from 'react-router';
import { describe, expect, it } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../test/render';
import { createApp } from './context';
import { Shell } from './Shell';

const namespace = { id: '019b5d6e-0000-7000-8000-000000000001', workspaceId: '019b5d6e-0000-7000-8000-000000000002', slug: 'production', environment: 'prod', createdAt: '2026-10-01T00:00:00Z' };
const other = { ...namespace, id: '019b5d6e-0000-7000-8000-000000000003', slug: 'staging', environment: 'staging' };

/** `chosen` is the namespace the person had picked before (a remembered choice); `null` is a first visit. */
async function mount(path = '/workflows', { chosen = 'production', items = [namespace, other] }: { chosen?: string | null; items?: unknown[] } = {}) {
  const platform = fakePlatform({ 'GET /namespaces': () => ({ items }) });
  const app = createApp({ fetch: platform.fetcher });
  app.namespace.set(chosen ?? undefined);
  await app.session.signIn(TOKEN, () => Promise.resolve(AUTH_CONTEXT as never));
  const result = renderApp(
    <Routes>
      <Route element={<Shell />}>
        <Route path="namespaces" element={<h1>Namespace list</h1>} />
        <Route path="workflows" element={<h1>Workflow library</h1>} />
        <Route path="runs" element={<h1>Run history</h1>} />
        <Route path="operations" element={<h1>Operations center</h1>} />
        <Route path="scripts" element={<h1>Script library</h1>} />
      </Route>
    </Routes>,
    { app, path },
  );
  await screen.findByRole('option', { name: 'production' });
  return { ...result, app, platform };
}

const sidebar = () => screen.getByRole('complementary', { name: 'Studio navigation' });
const links = () => within(within(sidebar()).getByRole('navigation', { name: 'Main' })).getAllByRole('link').map((link) => link.textContent);

describe('Shell', () => {
  it('keeps the brand and, in order, the tenant, the chosen namespace and what lives in it', async () => {
    await mount('/runs');
    expect(within(sidebar()).getByText('sagat-octopus')).toBeInTheDocument();
    const navigation = within(sidebar()).getByRole('navigation', { name: 'Main' });
    expect(within(sidebar()).getByLabelText('Namespace')).toHaveValue('production');
    expect(links()).toEqual(['Namespaces', 'Workflows', 'Runs', 'Operations', 'Scripts', 'Ad hoc database', 'Connections', 'Secrets', 'Integrations']);
    // What lives in the namespace is under it, in a list that says whose it is.
    const inside = within(navigation).getByRole('list', { name: 'In production' });
    expect(within(inside).getAllByRole('link').map((link) => link.textContent)).toEqual(['Workflows', 'Runs', 'Operations', 'Scripts', 'Ad hoc database', 'Connections', 'Secrets', 'Integrations']);
    expect(within(navigation).getByRole('link', { name: 'Namespaces' }).closest('ul')).not.toBe(inside);
    expect(within(navigation).getByRole('link', { name: 'Runs' })).toHaveAttribute('aria-current', 'page');
  });

  it('shows only the namespaces until one is chosen, and does not choose one for the person', async () => {
    const user = userEvent.setup();
    const { app } = await mount('/namespaces', { chosen: null, items: [namespace] });
    // One namespace only: it is still the person's to choose.
    expect(app.namespace.current()).toBeUndefined();
    expect(links()).toEqual(['Namespaces']);
    const select = within(sidebar()).getByLabelText('Namespace');
    expect(select).toHaveValue('');
    expect(within(select).getByRole('option', { name: 'Choose a namespace' })).toBeDisabled();
    expect(within(sidebar()).getByText(/Choose a namespace to open its workflows/)).toBeInTheDocument();
    expect(within(sidebar()).queryByRole('link', { name: 'Workflows' })).not.toBeInTheDocument();

    await user.selectOptions(select, 'production');
    expect(app.namespace.current()).toBe('production');
    expect(links()).toEqual(['Namespaces', 'Workflows', 'Runs', 'Operations', 'Scripts', 'Ad hoc database', 'Connections', 'Secrets', 'Integrations']);
    expect(within(sidebar()).queryByText(/Choose a namespace to open/)).not.toBeInTheDocument();
    expect(within(select).queryByRole('option', { name: 'Choose a namespace' })).not.toBeInTheDocument();
  });

  it('says so when the tenant has no namespace yet, and offers only the namespaces', async () => {
    const platform = fakePlatform({ 'GET /namespaces': () => ({ items: [] }) });
    const app = createApp({ fetch: platform.fetcher });
    app.namespace.set(undefined);
    await app.session.signIn(TOKEN, () => Promise.resolve(AUTH_CONTEXT as never));
    renderApp(<Routes><Route element={<Shell />}><Route path="namespaces" element={<h1>Namespace list</h1>} /></Route></Routes>, { app, path: '/namespaces' });
    expect(await screen.findByRole('option', { name: 'No namespaces yet' })).toBeInTheDocument();
    expect(within(sidebar()).getByLabelText('Namespace')).toBeDisabled();
    expect(links()).toEqual(['Namespaces']);
  });

  it('forgets a remembered namespace the tenant does not list, instead of picking another', async () => {
    const { app } = await mount('/namespaces', { chosen: 'retired' });
    await waitFor(() => expect(app.namespace.current()).toBeUndefined());
    expect(links()).toEqual(['Namespaces']);
    expect(within(sidebar()).getByLabelText('Namespace')).toHaveValue('');
  });

  it('keeps a remembered namespace the tenant lists', async () => {
    const { app } = await mount('/workflows', { chosen: 'staging' });
    expect(app.namespace.current()).toBe('staging');
    expect(within(sidebar()).getByLabelText('Namespace')).toHaveValue('staging');
    expect(within(sidebar()).getByRole('list', { name: 'In staging' })).toBeInTheDocument();
  });

  it('keeps the top bar grounded in the current page and real session', async () => {
    await mount('/scripts');
    const topbar = screen.getByRole('banner');
    expect(topbar).toHaveTextContent('production');
    expect(topbar).toHaveTextContent('Scripts');
    expect(within(topbar).getByLabelText('Signed in as')).toHaveTextContent('alice admin');
    expect(within(topbar).getByRole('button', { name: 'Sign out' })).toBeInTheDocument();
  });

  it('does not put the tenant\'s own page under a namespace in the top bar', async () => {
    await mount('/namespaces');
    const location = within(screen.getByRole('banner')).getByLabelText('Current location');
    expect(location).toHaveTextContent(/^Namespaces$/);
  });

  it('keeps the skip link and moves focus to the page after navigation', async () => {
    const user = userEvent.setup();
    await mount();
    expect(screen.getByRole('link', { name: 'Skip to the content' })).toHaveAttribute('href', '#main');
    await user.click(screen.getByRole('link', { name: 'Runs' }));
    await screen.findByRole('heading', { name: 'Run history' });
    await waitFor(() => expect(screen.getByRole('main')).toHaveFocus());
  });

  it('names the Operations destination in the top bar', async () => {
    await mount('/operations');
    expect(screen.getByRole('banner')).toHaveTextContent('Operations');
    expect(screen.getByRole('link', { name: 'Operations' })).toHaveAttribute('aria-current', 'page');
  });

  it('provides a namespace menu instead of a sidebar creation button', async () => {
    await mount();
    expect(screen.getByRole('link', { name: 'Namespaces' })).toHaveAttribute('href', '/namespaces');
    expect(screen.queryByRole('button', { name: 'New namespace' })).not.toBeInTheDocument();
  });
});
