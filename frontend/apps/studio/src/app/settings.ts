import type { StudioSettings } from './context';

const EMPTY: StudioSettings = { oidc: null, traceUrl: null };

/** Read deployment settings at page load, independently of the compiled Studio assets. */
export async function loadSettings(fetcher: typeof globalThis.fetch = globalThis.fetch): Promise<StudioSettings> {
  try {
    const response = await fetcher('/studio/config.json', { credentials: 'omit', cache: 'no-store' });
    if (!response.ok) return EMPTY;
    const body = (await response.json()) as { oidc?: StudioSettings['oidc']; traceUrl?: unknown };
    return {
      oidc: body.oidc ?? null,
      traceUrl: typeof body.traceUrl === 'string' ? body.traceUrl : null,
    };
  } catch {
    return EMPTY;
  }
}
