import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Route, Routes } from 'react-router';
import { beforeEach, describe, expect, it } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import { installStorage } from '../../../test/storage';
import { NS_ID, apiError, bodyOf, json, keyOf } from '../../../test/workflowsHarness';
import { createApp } from '../../app/context';
import type { Routes as FakeRoutes } from '../../../test/render';
import { WorkflowsPage } from './WorkflowsPage';

const draft = (n: number, extra: Record<string, unknown> = {}) => ({
  id: `00000000-0000-4000-8000-00000000000${n}`,
  name: `flow-${n}`,
  draftRevision: n,
  ir: {},
  updatedAt: '2026-03-01T10:00:00Z',
  ...extra,
});
const namespaces = { 'GET /namespaces': () => ({ items: [{ id: NS_ID, workspaceId: NS_ID, slug: 'dev', environment: 'dev', createdAt: '2026-01-01T00:00:00Z' }] }) };

async function mount(routes: FakeRoutes, permissions = ['definition:read', 'definition:write']) {
  installStorage();
  const platform = fakePlatform({ ...namespaces, ...routes });
  const app = createApp({ fetch: platform.fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions } as never));
  app.namespace.set('dev');
  const user = userEvent.setup();
  renderApp(
    <Routes>
      <Route path="/workflows" element={<WorkflowsPage />} />
      <Route path="/workflows/:workflowId" element={<p>the editor of a workflow</p>} />
    </Routes>,
    { app, path: '/workflows' },
  );
  return { platform, user, app };
}

beforeEach(() => installStorage());

describe('WorkflowsPage', () => {
  it('lists the workflows of the namespace in a table with the name, the active version and the last update', async () => {
    const { platform } = await mount({
      'GET /workflows': () => ({
        items: [draft(1, { activeVersion: { workflowId: 'w', versionId: 'v', versionNumber: 3, digest: 'sha256:x' } }), draft(2)],
        nextCursor: null,
      }),
    });
    const table = await screen.findByRole('table', { name: /Workflows of the namespace dev/ });
    const rows = within(table).getAllByRole('row');
    expect(rows).toHaveLength(3);
    expect(within(rows[1]!).getByRole('link', { name: 'flow-1' })).toHaveAttribute('href', '/workflows/00000000-0000-4000-8000-000000000001');
    expect(within(rows[1]!).getByText('Version 3')).toBeInTheDocument();
    expect(within(rows[2]!).getByText('None (not published)')).toBeInTheDocument();
    expect(
      within(table)
        .getAllByRole('columnheader')
        .map((h) => h.textContent),
    ).toEqual(['Name', 'Active version', 'Draft revision', 'Last updated']);
    // The namespace the user works in is sent with the request.
    const sent = platform.calls.find((c) => c.url.startsWith('/v1/workflows'));
    expect((sent?.init.headers as Record<string, string>)['X-Octopus-Namespace']).toBe('dev');
  });

  it('says so when there are no workflows yet, and points at the way to make one', async () => {
    await mount({ 'GET /workflows': () => ({ items: [], nextCursor: null }) });
    expect(await screen.findByText(/No workflows in/)).toHaveTextContent('Create the first one');
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
  });

  it('says it is loading, then shows a failure with its trace id and lets the user try again', async () => {
    let attempts = 0;
    await mount({
      'GET /workflows': () => (++attempts === 1 ? apiError(503, 'UNAVAILABLE', 'The platform is busy.', 'd'.repeat(32)) : { items: [draft(1)], nextCursor: null }),
    });
    expect(await screen.findByRole('alert')).toHaveTextContent('The platform is busy.');
    expect(screen.getByRole('alert')).toHaveTextContent('d'.repeat(32));
    await userEvent.click(screen.getByRole('button', { name: 'Try again' }));
    expect(await screen.findByRole('link', { name: 'flow-1' })).toBeInTheDocument();
  });

  it('loads more with the cursor the platform gave, and keeps what it already has', async () => {
    const { user, platform } = await mount({
      'GET /workflows': (_init, url) => (url.includes('cursor=next-1') ? { items: [draft(3)], nextCursor: null } : { items: [draft(1), draft(2)], nextCursor: 'next-1' }),
    });
    await screen.findByRole('link', { name: 'flow-1' });
    await user.click(screen.getByRole('button', { name: 'Load more' }));
    expect(await screen.findByRole('link', { name: 'flow-3' })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'flow-1' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Load more' })).not.toBeInTheDocument();
    expect(platform.calls.filter((c) => c.url.startsWith('/v1/workflows?')).map((c) => new URL(c.url, 'http://x').searchParams.get('cursor'))).toEqual([null, 'next-1']);
  });

  it('shows a viewer the list, and a New workflow button that says why it does nothing', async () => {
    const { user, platform } = await mount({ 'GET /workflows': () => ({ items: [draft(1)], nextCursor: null }) }, ['definition:read']);
    await screen.findByRole('link', { name: 'flow-1' });
    const button = screen.getByRole('button', { name: 'New workflow' });
    expect(button).toHaveAttribute('aria-disabled', 'true');
    expect(button).toHaveAccessibleDescription(/needs the definition:write permission/);
    await user.click(button);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(platform.calls.some((c) => c.method === 'POST')).toBe(false);
  });

  it('asks the platform for nothing when the user may not read workflows', async () => {
    const { platform } = await mount({}, ['run:read']);
    expect(await screen.findByText(/needs the definition:read permission/)).toBeInTheDocument();
    await waitFor(() => expect(platform.calls.filter((c) => c.url.startsWith('/v1/workflows'))).toHaveLength(0));
  });
});

describe('creating a workflow', () => {
  const list = { 'GET /workflows': () => ({ items: [], nextCursor: null }) };

  it('checks the name while it is typed: the button stays off until it is a valid slug', async () => {
    const { user } = await mount(list);
    await user.click(await screen.findByRole('button', { name: 'New workflow' }));
    const dialog = await screen.findByRole('dialog', { name: 'New workflow' });
    const name = within(dialog).getByRole('textbox', { name: 'Name' });
    const create = within(dialog).getByRole('button', { name: 'Create workflow' });
    expect(create).toBeDisabled();
    await user.type(name, 'Order');
    expect(within(dialog).getByRole('alert')).toHaveTextContent('lowercase letter');
    expect(name).toHaveAttribute('aria-invalid', 'true');
    expect(create).toBeDisabled();
    await user.clear(name);
    await user.type(name, 'ab');
    expect(within(dialog).getByRole('alert')).toHaveTextContent('at least 3');
    await user.clear(name);
    await user.type(name, 'order_flow');
    expect(within(dialog).getByRole('alert')).toHaveTextContent('lowercase letters, digits and dashes');
    await user.clear(name);
    await user.type(name, 'order-flow');
    expect(within(dialog).queryByRole('alert')).not.toBeInTheDocument();
    expect(create).toBeEnabled();
  });

  it('refuses a name longer than the platform allows', async () => {
    const { user } = await mount(list);
    await user.click(await screen.findByRole('button', { name: 'New workflow' }));
    const dialog = await screen.findByRole('dialog');
    await user.type(within(dialog).getByRole('textbox', { name: 'Name' }), 'a'.repeat(65));
    expect(within(dialog).getByRole('alert')).toHaveTextContent('at most 64');
    expect(within(dialog).getByRole('button', { name: 'Create workflow' })).toBeDisabled();
  });

  it('creates an empty draft named like the workflow and opens it in the editor', async () => {
    const created = draft(5, { name: 'order-flow' });
    const { user, platform } = await mount({ ...list, 'POST /workflows': () => json(created, 201) });
    await user.click(await screen.findByRole('button', { name: 'New workflow' }));
    await user.type(await screen.findByRole('textbox', { name: 'Name' }), 'order-flow');
    await user.click(screen.getByRole('button', { name: 'Create workflow' }));
    expect(await screen.findByText('the editor of a workflow')).toBeInTheDocument();
    const post = platform.calls.find((c) => c.method === 'POST');
    const body = bodyOf<{ name: string; ir: { metadata: { name: string }; nodes: unknown[]; entrypoint: string; triggers: { kind: string }[] } }>(post?.init);
    expect(body.name).toBe('order-flow');
    expect(body.ir.metadata.name).toBe('order-flow');
    expect(body.ir.nodes).toEqual([]);
    expect(body.ir.triggers).toEqual([{ kind: 'manual' }]);
    expect(keyOf(post)).toMatch(/\S/);
  });

  it('says the name is taken when the platform says so, and keeps the dialog open', async () => {
    const { user } = await mount({ ...list, 'POST /workflows': () => apiError(409, 'CONFLICT', 'exists') });
    await user.click(await screen.findByRole('button', { name: 'New workflow' }));
    await user.type(await screen.findByRole('textbox', { name: 'Name' }), 'order-flow');
    await user.click(screen.getByRole('button', { name: 'Create workflow' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('"order-flow" already exists');
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Create workflow' })).toBeDisabled();
  });

  it('shows the reason the platform gives for refusing, with its trace id', async () => {
    const { user } = await mount({ ...list, 'POST /workflows': () => apiError(403, 'FORBIDDEN', 'You are not allowed to create workflows here.', 'e'.repeat(32)) });
    await user.click(await screen.findByRole('button', { name: 'New workflow' }));
    await user.type(await screen.findByRole('textbox', { name: 'Name' }), 'order-flow');
    await user.click(screen.getByRole('button', { name: 'Create workflow' }));
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('You are not allowed to create workflows here.');
    expect(alert).toHaveTextContent('e'.repeat(32));
  });

  it('gives focus back to the button that opened the dialog when it is cancelled', async () => {
    const { user } = await mount(list);
    const button = await screen.findByRole('button', { name: 'New workflow' });
    await user.click(button);
    await user.click(await screen.findByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('textbox', { name: 'Name' })).not.toBeInTheDocument());
    expect(button).toHaveFocus();
  });
});
