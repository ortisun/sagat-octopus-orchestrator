import { describe, expect, it, vi } from 'vitest';
import { loadSettings } from './settings';

describe('runtime Studio settings', () => {
  it('reads the trace URL from the no-store public config on each page load', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(
        JSON.stringify({
          oidc: null,
          traceUrl: 'https://traces.example.test/trace/{traceId}',
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      ),
    );

    await expect(loadSettings(fetcher)).resolves.toEqual({
      oidc: null,
      traceUrl: 'https://traces.example.test/trace/{traceId}',
    });
    expect(fetcher).toHaveBeenCalledWith('/studio/config.json', {
      credentials: 'omit',
      cache: 'no-store',
    });
  });

  it('fails closed when config is absent, unavailable or has a non-string trace URL', async () => {
    const absent = vi.fn<typeof fetch>().mockResolvedValue(new Response(JSON.stringify({ oidc: null }), { status: 200 }));
    await expect(loadSettings(absent)).resolves.toEqual({ oidc: null, traceUrl: null });

    const malformed = vi.fn<typeof fetch>().mockResolvedValue(new Response(JSON.stringify({ oidc: null, traceUrl: { href: 'https://bad.test' } }), { status: 200 }));
    await expect(loadSettings(malformed)).resolves.toEqual({ oidc: null, traceUrl: null });

    const unavailable = vi.fn<typeof fetch>().mockResolvedValue(new Response(null, { status: 503 }));
    await expect(loadSettings(unavailable)).resolves.toEqual({ oidc: null, traceUrl: null });
  });
});
