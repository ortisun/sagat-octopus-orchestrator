import { describe, expect, it, vi } from 'vitest';
import { ApiClient } from './client';
import { endpoints } from './endpoints';

type Seen = { url: string; init: RequestInit };

function client(respond: (seen: Seen, index: number) => Response | Promise<Response>) {
  const seen: Seen[] = [];
  const api = new ApiClient({
    baseUrl: '/v1',
    token: () => 'tok-123',
    namespace: () => 'payments',
    onUnauthenticated: () => undefined,
    fetch: ((url: string, init: RequestInit) => {
      const entry = { url, init };
      seen.push(entry);
      return Promise.resolve(respond(entry, seen.length - 1));
    }) as typeof fetch,
  });
  return { ep: endpoints(api), seen };
}

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

const namespace = (id: number) => ({
  id: `00000000-0000-4000-8000-${String(id).padStart(12, '0')}`,
  workspaceId: '00000000-0000-4000-8000-000000000001',
  slug: `namespace-${id}`,
  environment: 'dev' as const,
  createdAt: '2026-10-12T12:00:00Z',
});

describe('endpoints.namespaces', () => {
  it('follows every cursor and returns one complete page to callers', async () => {
    const first = Array.from({ length: 200 }, (_, index) => namespace(index + 1));
    const last = namespace(201);
    const { ep, seen } = client((_request, index) =>
      index === 0 ? json({ items: first, nextCursor: 'after-200' }) : json({ items: [last], nextCursor: null }),
    );

    await expect(ep.namespaces()).resolves.toEqual({ items: [...first, last], nextCursor: null });
    expect(seen.map(({ url }) => url)).toEqual([
      '/v1/namespaces?limit=200',
      '/v1/namespaces?limit=200&cursor=after-200',
    ]);
  });

  it('rejects the whole load when a later page fails', async () => {
    const { ep, seen } = client((_request, index) =>
      index === 0
        ? json({ items: [namespace(1)], nextCursor: 'next' })
        : json({ code: 'UNAVAILABLE', message: 'Try again later.', retryable: true, traceId: 'a'.repeat(32) }, 503),
    );

    await expect(ep.namespaces()).rejects.toMatchObject({ code: 'UNAVAILABLE', status: 503 });
    expect(seen).toHaveLength(2);
  });

  it('cancels a later page with the caller signal', async () => {
    const controller = new AbortController();
    const { ep, seen } = client((request, index) => {
      if (index === 0) return json({ items: [namespace(1)], nextCursor: 'next' });
      return new Promise<Response>((_resolve, reject) => {
        request.init.signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError')), { once: true });
      });
    });

    const pending = ep.namespaces(controller.signal);
    await vi.waitFor(() => expect(seen).toHaveLength(2));
    controller.abort();

    await expect(pending).rejects.toMatchObject({ name: 'AbortError' });
    expect(seen[1]!.init.signal?.aborted).toBe(true);
  });
});
