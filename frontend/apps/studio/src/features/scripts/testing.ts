/**
 * Helpers for the tests of this feature: fixtures of what the platform answers, a scripted scripts
 * API with the rules of the real one that matter here (revisions, versions, test runs), and a signed-in
 * app. Only tests import this file.
 */
import { vi } from 'vitest';
import { createApp } from '../../app/context';
import type { App } from '../../app/context';
import type { AuthContext } from '../../api/endpoints';
import { AUTH_CONTEXT, TOKEN, fakePlatform } from '../../../test/render';
import type { Script, ScriptPolicy, ScriptSummary, ScriptTestRun, ScriptTestRunSummary, ScriptVersion, ScriptVersionSummary } from './api';

export const NAMESPACE_ID = '11111111-1111-4111-8111-111111111111';
export const SCRIPT_ID = '22222222-2222-4222-8222-222222222222';
export const OTHER_SCRIPT_ID = '44444444-4444-4444-8444-444444444444';
export const RUN_ID = '33333333-3333-4333-8333-333333333333';
export const DIGEST = `sha256:${'ab12'.repeat(16)}`;

export const POLICY: ScriptPolicy = { memoryMib: 256, cpuMillis: 1000, timeoutMs: 30_000, processes: 64, outputBytes: 65_536, scratchMib: 32, egress: 'none' };

export const DEVELOPER = ['script:read', 'script:write', 'script:publish'];
export const VIEWER = ['script:read'];
export const NOBODY: string[] = [];

export function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

export function problem(status: number, code: string, message: string, details?: Record<string, unknown>): Response {
  return json(status, { code, message, traceId: 'a'.repeat(32), ...(details ? { details } : {}) });
}

export function aScript(overrides: Partial<Script> = {}): Script {
  return {
    id: SCRIPT_ID,
    namespaceId: NAMESPACE_ID,
    name: 'double-it',
    language: 'typescript',
    source: 'export default async function (input: { n?: number }) {\n  return { double: (input.n ?? 0) * 2 };\n}\n',
    policy: POLICY,
    retentionDays: 14,
    revision: 1,
    createdBy: 'alice',
    createdAt: '2026-10-10T09:00:00Z',
    updatedAt: '2026-10-10T09:00:00Z',
    ...overrides,
  };
}

export function aSummary(overrides: Partial<ScriptSummary> = {}): ScriptSummary {
  const { source, ...rest } = aScript();
  return { ...rest, sourceBytes: source.length, ...overrides };
}

export function aVersionSummary(number: number, status: ScriptVersionSummary['status'] = 'READY', overrides: Partial<ScriptVersionSummary> = {}): ScriptVersionSummary {
  return {
    id: `55555555-5555-4555-8555-${String(number).padStart(12, '0')}`,
    scriptId: SCRIPT_ID,
    number,
    language: 'typescript',
    status,
    policy: POLICY,
    retentionDays: 14,
    contentDigest: `sha256:${'cd'.repeat(32)}`,
    ...(status === 'READY' ? { artifactDigest: DIGEST, activity: `script.${DIGEST.replace(':', '-')}` } : {}),
    createdBy: 'alice',
    createdAt: '2026-10-10T10:00:00Z',
    ...overrides,
  };
}

export function aVersion(number: number, status: ScriptVersion['status'] = 'READY', overrides: Partial<ScriptVersion> = {}): ScriptVersion {
  return { ...aVersionSummary(number, status), source: 'export default () => 1;\n', ...overrides };
}

export function aRunSummary(overrides: Partial<ScriptTestRunSummary> = {}): ScriptTestRunSummary {
  return { id: RUN_ID, scriptId: SCRIPT_ID, language: 'typescript', status: 'SUCCEEDED', wallMs: 120, startMs: 30, createdBy: 'alice', createdAt: '2026-10-10T09:30:00Z', ...overrides };
}

export function aRun(overrides: Partial<ScriptTestRun> = {}): ScriptTestRun {
  return { ...aRunSummary(), log: '', logTruncated: false, denied: [], ...overrides };
}

/** The app of a signed-in person with `permissions`, working in the namespace `payments`. */
export async function signedIn(fetcher: typeof fetch, permissions: string[] = DEVELOPER): Promise<App> {
  const app = createApp({ fetch: fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, role: 'developer', permissions } as AuthContext));
  app.namespace.set('payments');
  return app;
}

export interface ServerOptions {
  script?: Script;
  versions?: ScriptVersionSummary[];
  runs?: ScriptTestRunSummary[];
  scripts?: ScriptSummary[];
}

/**
 * A scripts API in memory. It keeps the rules the Studio relies on: a change carries the revision
 * it was based on and a stale one is a `409` that says what the revision is now; a script with a
 * version cannot be deleted; a name is unique. Everything else is scripted by the test through
 * `override`.
 */
export function scriptsServer(options: ServerOptions = {}) {
  const state = {
    script: options.script ?? aScript(),
    scripts: options.scripts ?? [aSummary()],
    versions: options.versions ?? [],
    runs: options.runs ?? [],
    patches: [] as { body: Record<string, unknown>; key: string | undefined }[],
  };
  const overrides: Record<string, (init: RequestInit, url: string) => unknown> = {};
  const base = `/namespaces/${NAMESPACE_ID}/scripts`;
  const one = `${base}/${state.script.id}`;
  const headerOf = (init: RequestInit, name: string) => (init.headers as Record<string, string> | undefined)?.[name];

  const routes: Record<string, (init: RequestInit, url: string) => unknown> = {
    'GET /namespaces': () => ({ items: [{ id: NAMESPACE_ID, workspaceId: NAMESPACE_ID, slug: 'payments', environment: 'dev', createdAt: '2026-10-01T10:00:00Z' }] }),
    [`GET ${base}`]: () => ({ items: state.scripts }),
    [`POST ${base}`]: (init) => {
      const body = JSON.parse(init.body as string) as { name: string; language: string; source: string };
      if (state.scripts.some((s) => s.name === body.name)) return problem(409, 'CONFLICT', 'a script with this name already exists', { field: 'name' });
      if (body.language !== 'typescript' && body.language !== 'python') return problem(422, 'VALIDATION_FAILED', 'the language is not built yet: typescript and python are', { reason: 'LANGUAGE_NOT_BUILT' });
      const created = aScript({ id: OTHER_SCRIPT_ID, name: body.name, language: body.language, source: body.source });
      state.scripts = [...state.scripts, aSummary({ id: created.id, name: created.name, language: created.language })];
      return json(201, created);
    },
    [`GET ${one}`]: () => state.script,
    [`PATCH ${one}`]: (init) => {
      const body = JSON.parse(init.body as string) as Record<string, unknown> & { revision: number };
      state.patches.push({ body, key: headerOf(init, 'Idempotency-Key') });
      if (body.revision !== state.script.revision) return problem(409, 'CONFLICT', 'the draft was changed since it was read; fetch it again and retry', { reason: 'REVISION_CONFLICT', currentRevision: state.script.revision });
      const { revision: _revision, ...changes } = body;
      void _revision;
      const next = { ...state.script, ...changes, revision: state.script.revision + 1, updatedAt: new Date(Date.parse(state.script.updatedAt) + 60_000).toISOString() };
      for (const field of ['inputSchema', 'outputSchema'] as const) if (next[field] === null) delete next[field];
      state.script = next;
      return state.script;
    },
    [`DELETE ${one}`]: () => {
      if (state.versions.length > 0) return problem(409, 'CONFLICT', 'a script with a published version cannot be deleted', { reason: 'STATE_CONFLICT' });
      state.scripts = state.scripts.filter((s) => s.id !== state.script.id);
      return new Response(null, { status: 204 });
    },
    [`GET ${one}/versions`]: () => ({ items: state.versions }),
    [`GET ${one}/test-runs`]: () => ({ items: state.runs }),
  };
  const merged = new Proxy(routes, {
    get: (target, key: string) => overrides[key] ?? target[key],
    has: (target, key: string) => key in overrides || key in target,
  });
  const platform = fakePlatform(merged);
  // A request can be held: it is not answered until the test lets it go, so a test can look at the page
  // while the platform has not answered yet, which is a state a real network is often in.
  const holds = new Map<string, Promise<void>>();
  const fetcher = ((url: string, init?: RequestInit) => {
    const key = `${init?.method ?? 'GET'} ${url.replace(/^\/v1/, '').split('?')[0] ?? ''}`;
    const held = holds.get(key);
    return held ? held.then(() => platform.fetcher(url, init)) : platform.fetcher(url, init);
  }) as typeof fetch;
  return {
    state,
    calls: platform.calls,
    fetcher,
    /** Holds the answer to "METHOD /path" until the function it returns is called. */
    hold: (key: string) => {
      let release: () => void = () => undefined;
      holds.set(
        key,
        new Promise<void>((resolve) => {
          release = () => {
            holds.delete(key);
            resolve();
          };
        }),
      );
      return release;
    },
    paths: { base, one },
    /** Replaces the answer to "METHOD /path" (the path without `/v1`). */
    override: (key: string, handler: (init: RequestInit, url: string) => unknown) => {
      overrides[key] = handler;
    },
    callsTo: (method: string, pathAfterV1: string) => platform.calls.filter((c) => c.method === method && c.url.replace(/^\/v1/, '').split('?')[0] === pathAfterV1),
  };
}

/** A `localStorage` that works (the test environment's may not), emptied for each test that asks for it. */
export function stubStorage(): Storage {
  const data = new Map<string, string>();
  const storage = {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => void data.set(key, String(value)),
    removeItem: (key: string) => void data.delete(key),
    clear: () => data.clear(),
    key: (index: number) => [...data.keys()][index] ?? null,
    get length() {
      return data.size;
    },
  } as Storage;
  vi.stubGlobal('localStorage', storage);
  return storage;
}
