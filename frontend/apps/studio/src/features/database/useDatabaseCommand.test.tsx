import { act, renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { AUTH_CONTEXT, TOKEN } from '../../../test/render';
import { createApp } from '../../app/context';
import { ApiError } from '../../api/errors';
import { useDatabaseCommand } from './useDatabaseCommand';

describe('database command uncertainty', () => {
  it.each([403, 404, 429])('keeps the frozen intent after a lost acknowledgement followed by %s', async status => {
    const execute = vi.fn().mockRejectedValueOnce(new Error('lost acknowledgement')).mockRejectedValueOnce(new ApiError({ code: 'FORBIDDEN', status, message: 'Refused' })).mockResolvedValue({ runId: 'recovered' });
    const app = createApp();
    const { result } = renderHook(() => useDatabaseCommand<{ sql: string }>(execute, app.session, 'test'));
    const body = { sql: 'INSERT INTO effects VALUES (1)' };
    await act(() => result.current.send(body)); body.sql = 'CHANGED';
    await act(() => result.current.send());
    expect(result.current.state).toMatchObject({ phase: 'failed', refused: false });
    act(() => result.current.reset()); expect(result.current.locked).toBe(true);
    await act(() => result.current.send({ sql: 'NEW WRITE' }));
    expect(execute.mock.calls.map(c => c[0] as unknown)).toEqual([execute.mock.calls[0]?.[0], execute.mock.calls[0]?.[0], execute.mock.calls[0]?.[0]]);
    expect(execute.mock.calls[2]?.[0]).toMatchObject({ body: { sql: 'INSERT INTO effects VALUES (1)' } });
  });
  it('survives navigation and treats an empty success body as an uncertain acknowledgement', async () => {
    const app = createApp();
    const execute = vi.fn().mockResolvedValueOnce(undefined).mockResolvedValue({ runId: 'receipt' });
    const first = renderHook(() => useDatabaseCommand(execute, app.session, 'namespace'));
    await act(() => first.result.current.send({ statement: 'INSERT INTO effects VALUES (2)' }));
    expect(first.result.current.state).toMatchObject({ phase: 'failed', refused: false });
    first.unmount();
    const second = renderHook(() => useDatabaseCommand(execute, app.session, 'namespace'));
    expect(second.result.current.body).toEqual({ statement: 'INSERT INTO effects VALUES (2)' });
    await act(() => second.result.current.send());
    expect(execute.mock.calls[0]?.[0]).toEqual(execute.mock.calls[1]?.[0]);
    expect(second.result.current.state).toMatchObject({ phase: 'accepted' });
  });
  it('clears pending SQL when the session ends', async () => {
    const app = createApp();
    await app.session.signIn(TOKEN, () => Promise.resolve(AUTH_CONTEXT as never));
    const execute = vi.fn().mockRejectedValue(new Error('unknown'));
    const first = renderHook(() => useDatabaseCommand(execute, app.session, 'namespace'));
    await act(() => first.result.current.send({ statement: 'private SQL' })); first.unmount();
    app.session.signOut('signed-out');
    const second = renderHook(() => useDatabaseCommand(execute, app.session, 'namespace'));
    expect(second.result.current.body).toBeUndefined(); expect(second.result.current.state.phase).toBe('idle');
  });
  it('allows correction after a definite first-attempt refusal', async () => {
    const execute = vi.fn().mockRejectedValue(new ApiError({ code: 'VALIDATION_FAILED', status: 422, message: 'Invalid definition' }));
    const app = createApp();
    const { result } = renderHook(() => useDatabaseCommand(execute, app.session, 'test'));
    await act(() => result.current.send({})); expect(result.current.state).toMatchObject({ refused: true });
    act(() => result.current.reset()); expect(result.current.locked).toBe(false);
  });
});
