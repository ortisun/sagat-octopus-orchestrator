import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AuthContext } from '../api/endpoints';
import { SessionStore, expiryOf } from './session';

const b64 = (value: unknown) => btoa(JSON.stringify(value)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
const token = (claims: Record<string, unknown>) => `${b64({ alg: 'RS256' })}.${b64(claims)}.sig`;

const context: AuthContext = { tenant: 'acme', subject: 'alice', role: 'operator', permissions: ['run:read', 'run:operate'], traceId: 'a'.repeat(32) };

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('SessionStore', () => {
  it('keeps the token only after the platform says whose it is, and exposes the permissions', async () => {
    const store = new SessionStore();
    const seen = vi.fn();
    store.subscribe(seen);
    const t = token({ exp: Math.floor(Date.now() / 1000) + 3600 });
    await store.signIn(`  ${t}\n`, () => Promise.resolve(context));
    expect(store.token()).toBe(t);
    expect(store.snapshot().info).toMatchObject({ tenant: 'acme', subject: 'alice', role: 'operator' });
    expect(store.can('run:operate')).toBe(true);
    expect(store.can('saga:override')).toBe(false);
    expect(seen).toHaveBeenCalled();
  });

  it('does not keep a token the platform refused, nor send one that is not a token', async () => {
    const store = new SessionStore();
    const whoAmI = vi.fn().mockRejectedValue(new Error('401'));
    await expect(store.signIn(token({}), whoAmI)).rejects.toThrow('401');
    expect(store.token()).toBeUndefined();
    for (const bad of ['', 'abc', 'a.b', 'a b.c.d', 'a.b.c.d', '<script>.x.y', 'x'.repeat(20_000)]) {
      const ask = vi.fn();
      await expect(store.signIn(bad, ask)).rejects.toThrow(/not a token/);
      expect(ask).not.toHaveBeenCalled();
    }
  });

  it('ends at the expiry of the token, and says why', async () => {
    const store = new SessionStore();
    const exp = Math.floor(Date.now() / 1000) + 60;
    await store.signIn(token({ exp }), () => Promise.resolve(context));
    await vi.advanceTimersByTimeAsync(59_000);
    expect(store.token()).toBeDefined();
    await vi.advanceTimersByTimeAsync(2000);
    expect(store.token()).toBeUndefined();
    expect(store.snapshot()).toMatchObject({ info: undefined, ended: 'expired' });
  });

  it('forgets everything on sign-out and on rejection', async () => {
    const store = new SessionStore();
    await store.signIn(token({}), () => Promise.resolve(context));
    store.signOut('rejected');
    expect(store.token()).toBeUndefined();
    expect(store.snapshot()).toEqual({ info: undefined, ended: 'rejected', expiresAt: undefined });
    expect(store.can('run:read')).toBe(false);
    await store.signIn(token({}), () => Promise.resolve(context));
    expect(store.snapshot().ended).toBeUndefined();
  });

  it('never writes the token anywhere', async () => {
    const setItem = vi.spyOn(Storage.prototype, 'setItem');
    const cookie = vi.spyOn(document, 'cookie', 'set');
    const store = new SessionStore();
    await store.signIn(token({ exp: 1 }), () => Promise.resolve(context));
    expect(setItem).not.toHaveBeenCalled();
    expect(cookie).not.toHaveBeenCalled();
  });
});

describe('expiryOf', () => {
  it('reads exp when there is one and says nothing otherwise', () => {
    expect(expiryOf(token({ exp: 1_900_000_000 }))).toBe(1_900_000_000);
    expect(expiryOf(token({ exp: 'soon' }))).toBeUndefined();
    expect(expiryOf('not.a.token')).toBeUndefined();
    expect(expiryOf('')).toBeUndefined();
  });
});
