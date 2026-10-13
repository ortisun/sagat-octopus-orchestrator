import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import { createApp } from '../../app/context';
import { SendSignal } from './SendSignal';

async function mount({ permission = true, status = 'WAITING', fail = false } = {}) {
  const platform = fakePlatform({ 'POST /runs/run-1/signals/order.approved': () => fail
    ? new Response(JSON.stringify({ code: 'UNAVAILABLE', message: 'Try again.', retryable: true }), { status: 503, headers: { 'content-type': 'application/json' } })
    : { deliveredTo: ['approval'] } });
  const app = createApp({ fetch: platform.fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions: permission ? ['run:signal'] : [] } as never));
  app.namespace.set('payments');
  const refresh = vi.fn();
  renderApp(<SendSignal runId="run-1" status={status} onSent={refresh} />, { app });
  return { app, platform, refresh, user: userEvent.setup() };
}
async function fill(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole('button', { name: 'Send signal' }));
  await user.type(screen.getByLabelText('Signal name'), 'order.approved');
  await user.clear(screen.getByLabelText('Payload (JSON object)'));
  await user.click(screen.getByLabelText('Payload (JSON object)'));
  await user.paste('{"approved":true}');
}
describe('Send signal', () => {
  it('is unavailable without permission', async () => {
    await mount({ permission: false });
    expect(screen.queryByRole('button', { name: 'Send signal' })).not.toBeInTheDocument();
  });
  it('is unavailable for a finished run', async () => {
    await mount({ status: 'SUCCEEDED' });
    expect(screen.queryByRole('button', { name: 'Send signal' })).not.toBeInTheDocument();
  });
  it('explains consequences and sends only after confirmation', async () => {
    const { user, platform, refresh } = await mount(); await fill(user);
    expect(screen.getByText(/every waiting step/)).toBeVisible();
    expect(platform.calls).toHaveLength(0);
    await user.click(screen.getByRole('button', { name: 'Confirm signal' }));
    await waitFor(() => expect(refresh).toHaveBeenCalledOnce());
    expect(JSON.parse(platform.calls[0]!.init.body as string)).toEqual({ approved: true });
    expect(screen.queryByLabelText('Payload (JSON object)')).not.toBeInTheDocument();
  });
  it('refuses malformed JSON and non-object payloads', async () => {
    const { user, platform } = await mount(); await fill(user);
    for (const payload of ['{', '[]', 'null', '123']) {
      await user.clear(screen.getByLabelText('Payload (JSON object)')); await user.paste(payload);
      await user.click(screen.getByRole('button', { name: 'Confirm signal' }));
      expect(screen.getByRole('alert')).toHaveTextContent(/JSON object/);
    }
    expect(platform.calls).toHaveLength(0);
  });
  it('rejects invalid names and oversized UTF-8 payloads before sending', async () => {
    const { user, platform } = await mount(); await fill(user);
    fireEvent.change(screen.getByLabelText('Signal name'), { target: { value: 'bad/name' } });
    await user.click(screen.getByRole('button', { name: 'Confirm signal' }));
    expect(screen.getByRole('alert')).toHaveTextContent('signal name');
    fireEvent.change(screen.getByLabelText('Signal name'), { target: { value: 'order.approved' } });
    fireEvent.change(screen.getByLabelText('Payload (JSON object)'), { target: { value: JSON.stringify({ value: 'é'.repeat(140000) }) } });
    await user.click(screen.getByRole('button', { name: 'Confirm signal' }));
    expect(screen.getByRole('alert')).toHaveTextContent('256 KiB');
    expect(platform.calls).toHaveLength(0);
  });
  it('keeps an identical retry idempotent and changes the key when input changes', async () => {
    const { user, platform } = await mount({ fail: true }); await fill(user);
    await user.click(screen.getByRole('button', { name: 'Confirm signal' }));
    await screen.findByRole('alert');
    await user.click(screen.getByRole('button', { name: 'Confirm signal' }));
    await waitFor(() => expect(platform.calls).toHaveLength(2));
    const key = (index: number) => new Headers(platform.calls[index]!.init.headers).get('Idempotency-Key');
    expect(key(0)).toBeTruthy(); expect(key(1)).toBe(key(0));
    await user.clear(screen.getByLabelText('Payload (JSON object)')); await user.paste('{}');
    await user.click(screen.getByRole('button', { name: 'Confirm signal' }));
    await waitFor(() => expect(platform.calls).toHaveLength(3)); expect(key(2)).not.toBe(key(0));
  });
  it('ignores a committed response arriving after the namespace changed', async () => {
    let complete!: (value: { deliveredTo: string[] }) => void;
    const platform = fakePlatform({ 'POST /runs/run-1/signals/order.approved': () => new Promise((resolve) => { complete = resolve; }) });
    const app = createApp({ fetch: platform.fetcher });
    await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions: ['run:signal'] } as never));
    app.namespace.set('payments');
    const refresh = vi.fn();
    renderApp(<SendSignal runId="run-1" status="WAITING" onSent={refresh} />, { app });
    const user = userEvent.setup(); await fill(user);
    await user.click(screen.getByRole('button', { name: 'Confirm signal' }));
    await waitFor(() => expect(platform.calls).toHaveLength(1));
    act(() => app.namespace.set('other'));
    await act(async () => { complete({ deliveredTo: ['approval'] }); await Promise.resolve(); });
    expect(refresh).not.toHaveBeenCalled();
    expect(screen.queryByLabelText('Payload (JSON object)')).not.toBeInTheDocument();
  });
  it('discards the form when the same subject signs in again', async () => {
    const { user, app } = await mount(); await fill(user);
    await act(async () => app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions: ['run:signal'] } as never)));
    expect(screen.queryByLabelText('Payload (JSON object)')).not.toBeInTheDocument();
  });
  it('discards payload and confirmation when changing namespace', async () => {
    const { user, app, platform } = await mount(); await fill(user);
    act(() => app.namespace.set('other'));
    expect(screen.queryByLabelText('Payload (JSON object)')).not.toBeInTheDocument();
    expect(platform.calls).toHaveLength(0);
  });
});
