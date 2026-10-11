import { describe, expect, it, vi } from 'vitest';
import { ApiClient, path } from './client';
import { ApiError } from './errors';

type Seen = { url: string; init: RequestInit };

function client(respond: (seen: Seen) => Response | Promise<Response>, over: Partial<ConstructorParameters<typeof ApiClient>[0]> = {}) {
  const seen: Seen[] = [];
  const unauthenticated = vi.fn();
  const api = new ApiClient({
    baseUrl: '/v1',
    token: () => 'tok-123',
    namespace: () => 'payments',
    onUnauthenticated: unauthenticated,
    newKey: () => 'key-fixed-0001',
    fetch: ((url: string, init: RequestInit) => {
      const entry = { url, init };
      seen.push(entry);
      return Promise.resolve(respond(entry));
    }) as typeof fetch,
    ...over,
  });
  return { api, seen, unauthenticated };
}

const json = (body: unknown, status = 200, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', ...headers } });

describe('ApiClient', () => {
  it('sends the token and the namespace to the page origin, and nothing that would carry state', async () => {
    const { api, seen } = client(() => json({ ok: true }));
    await expect(api.get('/workflows', { query: { limit: 25, cursor: undefined } })).resolves.toEqual({ ok: true });
    const { url, init } = seen[0]!;
    expect(url).toBe('/v1/workflows?limit=25');
    const headers = init.headers as Record<string, string>;
    expect(headers['Authorization']).toBe('Bearer tok-123');
    expect(headers['X-Octopus-Namespace']).toBe('payments');
    expect(headers['Idempotency-Key']).toBeUndefined();
    expect(init).toMatchObject({ method: 'GET', credentials: 'omit', redirect: 'error', cache: 'no-store', referrerPolicy: 'no-referrer' });
  });

  it('sends no Authorization header before sign-in', async () => {
    const { api, seen } = client(() => json({}), { token: () => undefined, namespace: () => undefined });
    await api.get('/health');
    const headers = seen[0]!.init.headers as Record<string, string>;
    expect(headers['Authorization']).toBeUndefined();
    expect(headers['X-Octopus-Namespace']).toBeUndefined();
  });

  it('gives every change an idempotency key, and a retry of the same intent can keep its own', async () => {
    const { api, seen } = client(() => json({ id: 1 }, 201));
    await api.post('/workflows', { body: { name: 'a' } });
    await api.put('/workflows/1', { body: { draftRevision: 1, ir: {} }, key: 'my-key-00001' });
    await api.delete('/x');
    const keys = seen.map((s) => (s.init.headers as Record<string, string>)['Idempotency-Key']);
    expect(keys).toEqual(['key-fixed-0001', 'my-key-00001', 'key-fixed-0001']);
    expect(seen[0]!.init.body).toBe('{"name":"a"}');
    expect((seen[0]!.init.headers as Record<string, string>)['Content-Type']).toBe('application/json');
  });

  it('refuses anything that is not a path of the page origin, so the token cannot be sent elsewhere', async () => {
    const { api, seen } = client(() => json({}));
    for (const bad of ['https://evil.example/x', '//evil.example/x', 'workflows', '/a/../b', '/a\\b']) {
      await expect(api.get(bad)).rejects.toThrow(/absolute paths/);
    }
    expect(seen).toHaveLength(0);
  });

  it('encodes what comes from the URL so it cannot change the path', () => {
    expect(path`/workflows/${'a/b?c#d'}/versions/${3}`).toBe('/workflows/a%2Fb%3Fc%23d/versions/3');
  });

  it('turns the envelope into an ApiError, and ends the session on a 401', async () => {
    const { api, unauthenticated } = client(() => json({ code: 'UNAUTHENTICATED', message: 'expired', retryable: false, traceId: 'b'.repeat(32) }, 401));
    const error = await api.get('/auth/context').catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).code).toBe('UNAUTHENTICATED');
    expect((error as ApiError).traceId).toBe('b'.repeat(32));
    expect(unauthenticated).toHaveBeenCalledOnce();
  });

  it('keeps the revision conflict and the throttle apart from other failures', async () => {
    const conflict = client(() => json({ code: 'CONFLICT', message: 'stale', retryable: false, traceId: 'c'.repeat(32), details: { current: 4 } }, 409));
    const e1 = (await conflict.api.put('/workflows/1', { body: {} }).catch((e: unknown) => e)) as ApiError;
    expect([e1.code, e1.status, e1.details]).toEqual(['CONFLICT', 409, { current: 4 }]);
    const limited = client(() => json({ code: 'RATE_LIMITED', message: 'slow down' }, 429, { 'retry-after': '3' }));
    const e2 = (await limited.api.get('/x').catch((e: unknown) => e)) as ApiError;
    expect([e2.code, e2.retryable, e2.retryAfterMs]).toEqual(['RATE_LIMITED', true, 3000]);
  });

  it('does not call a 200 with a page in it a success', async () => {
    const { api } = client(() => new Response('<html>login</html>', { status: 200, headers: { 'content-type': 'text/html' } }));
    await expect(api.get('/x')).rejects.toMatchObject({ code: 'INTERNAL' });
  });

  it('reports a network failure as a transport error, and a timeout too', async () => {
    const down = client(() => {
      throw new TypeError('Failed to fetch');
    });
    await expect(down.api.get('/x')).rejects.toMatchObject({ code: 'TRANSPORT', retryable: true });

    const slow = new ApiClient({
      baseUrl: '/v1',
      token: () => undefined,
      namespace: () => undefined,
      onUnauthenticated: () => {},
      timeoutMs: 20,
      fetch: ((_url: string, init: RequestInit) =>
        new Promise((_resolve, reject) => {
          init.signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError')));
        })) as typeof fetch,
    });
    await expect(slow.get('/x')).rejects.toMatchObject({ code: 'TRANSPORT' });
  });

  it('lets the caller cancel without calling it a failure of the platform', async () => {
    const controller = new AbortController();
    const api = new ApiClient({
      baseUrl: '/v1',
      token: () => undefined,
      namespace: () => undefined,
      onUnauthenticated: () => {},
      fetch: ((_url: string, init: RequestInit) =>
        new Promise((_resolve, reject) => {
          init.signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError')));
        })) as typeof fetch,
    });
    const pending = api.get('/x', { signal: controller.signal });
    controller.abort();
    await expect(pending).rejects.toMatchObject({ name: 'AbortError' });
  });

  it('understands an empty answer', async () => {
    const { api } = client(() => new Response(null, { status: 204 }));
    await expect(api.delete('/x')).resolves.toBeUndefined();
  });
});
