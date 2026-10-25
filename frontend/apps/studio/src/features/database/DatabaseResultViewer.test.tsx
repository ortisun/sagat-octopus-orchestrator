import { act, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import { createApp } from '../../app/context';
import { DatabaseResultViewer } from './DatabaseResultViewer';
import { MAX_RESULT_BYTES, readDatabaseResult } from './readResult';

const canary = '<img src=x onerror=alert(1)>private-result';
const result = { columns: [{ name: 'same', type: 'integer' }, { name: 'same', type: 'text' }], rows: [[{ type: 'integer', value: '9223372036854775807' }, { type: 'text', value: canary }]], affectedRows: '1', truncated: false };
async function setup(allowed = true) {
  const platform = fakePlatform({ 'GET /namespaces': () => ({ items: [{ id: 'ns', slug: 'payments' }, { id: 'other', slug: 'other' }] }), 'GET *': () => result });
  const app = createApp({ fetch: platform.fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions: ['namespace:read', 'run:read', ...(allowed ? ['database:result:read'] : [])] } as never));
  app.namespace.set('payments'); const mounted = renderApp(<DatabaseResultViewer runId="run" nodeId="query" />, { app });
  return { platform, user: userEvent.setup(), ...mounted };
}
describe('database result disclosure', () => {
  it('requires explicit permission and never reads rows automatically', async () => {
    const { platform } = await setup(false);
    expect(screen.queryByRole('button', { name: 'Read database result' })).not.toBeInTheDocument();
    expect(platform.calls).toHaveLength(0);
  });
  it('preserves exact numbers and duplicate columns, escapes markup, avoids cache and clears on close/scope/session changes', async () => {
    const { user, app, platform } = await setup();
    await user.click(await screen.findByRole('button', { name: 'Read database result' }));
    await screen.findByText(canary); expect(screen.getByText('9223372036854775807')).toBeInTheDocument();
    expect(document.querySelector('img')).toBeNull(); expect(screen.getAllByRole('columnheader')).toHaveLength(2);
    expect(JSON.stringify(app.queries.getQueryCache().getAll().map(q => q.state.data))).not.toContain(canary);
    const request = platform.calls.find(c => c.url.includes('database-results')); expect(request?.init.cache).toBe('no-store');
    await user.click(screen.getByRole('button', { name: 'Close database result' })); expect(screen.queryByText(canary)).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Read database result' })); await screen.findByText(canary);
    act(() => app.namespace.set('other')); expect(screen.queryByText(canary)).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Read database result' })); await screen.findByText(canary);
    act(() => app.session.signOut('signed-out')); expect(screen.queryByText(canary)).not.toBeInTheDocument();
  });
  it('retains unsafe JSON numbers as numeric text without rounding them', async () => {
    const raw = '{"columns":[{"name":"data","type":"json"}],"rows":[[{"type":"json","value":{"id":9007199254740993,"amount":1.1234567890123456789}}]],"affectedRows":"1","truncated":false}';
    const platform = fakePlatform({ 'GET /namespaces': () => ({ items: [{ id: 'ns', slug: 'payments' }] }), 'GET *': () => new Response(raw, { headers: { 'Content-Type': 'application/json' } }) });
    const app = createApp({ fetch: platform.fetcher });
    await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions: ['namespace:read', 'run:read', 'database:result:read'] } as never));
    app.namespace.set('payments'); renderApp(<DatabaseResultViewer runId="run" nodeId="query" />, { app });
    await userEvent.setup().click(await screen.findByRole('button', { name: 'Read database result' }));
    await screen.findByText('{"id":9007199254740993,"amount":1.1234567890123456789}');
  });
  it('bounds chunked error envelopes and still ends an unauthenticated session', async () => {
    let chunks = 0; let cancelled = false;
    const stream = new ReadableStream<Uint8Array>({ pull(controller) { chunks++; controller.enqueue(new Uint8Array(16_384)); }, cancel() { cancelled = true; } });
    const platform = fakePlatform({ 'GET *': () => new Response(stream, { status: 502 }) });
    const app = createApp({ fetch: platform.fetcher });
    await expect(readDatabaseResult(app.api, 'ns', 'run', 'query', new AbortController().signal)).rejects.toMatchObject({ code: 'INTERNAL' });
    expect(cancelled).toBe(true); expect(chunks).toBeLessThanOrEqual(6);
    const unauthenticated = createApp({ fetch: fakePlatform({ 'GET *': () => new Response('not trusted', { status: 401 }) }).fetcher });
    await unauthenticated.session.signIn(TOKEN, () => Promise.resolve(AUTH_CONTEXT as never));
    await expect(readDatabaseResult(unauthenticated.api, 'ns', 'run', 'query', new AbortController().signal)).rejects.toMatchObject({ code: 'UNAUTHENTICATED' });
    expect(unauthenticated.session.snapshot().info).toBeUndefined();
  });
  it('rejects oversized and incomplete responses without a partial result', async () => {
    const controller = new AbortController();
    for (const body of ['x'.repeat(MAX_RESULT_BYTES + 1), JSON.stringify(result).slice(0, -1)]) {
      const platform = fakePlatform({ 'GET *': () => new Response(body, { headers: { 'Content-Type': 'application/json' } }) });
      const app = createApp({ fetch: platform.fetcher });
      await expect(readDatabaseResult(app.api, 'ns', 'run', 'query', controller.signal)).rejects.toMatchObject({ code: 'INTERNAL' });
    }
  });
});
