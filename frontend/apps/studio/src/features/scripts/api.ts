import type { ApiClient } from '../../api/client';
import { path } from '../../api/client';
import type { components } from '../../api/schema';

type Schemas = components['schemas'];

export type Script = Schemas['Script'];
export type ScriptSummary = Schemas['ScriptSummary'];
export type ScriptPolicy = Schemas['ScriptPolicyView'];
type JsonObject = Record<string, unknown>;
/**
 * The requests as the contract describes them. The generated types are stricter than the wire in two
 * places (a schema is `Record<string, never>`, and a field with a default is required), so the
 * bodies are written out here.
 */
export type CreateScriptRequest = Omit<Schemas['CreateScriptRequest'], 'retentionDays' | 'inputSchema' | 'outputSchema'> & {
  retentionDays?: number;
  inputSchema?: JsonObject;
  outputSchema?: JsonObject;
};
export type UpdateScriptRequest = Omit<Schemas['UpdateScriptRequest'], 'inputSchema' | 'outputSchema'> & {
  /** An object sets the schema; `null` removes it; absent leaves it alone. */
  inputSchema?: JsonObject | null;
  outputSchema?: JsonObject | null;
};
export type ScriptVersion = Schemas['ScriptVersion'];
export type ScriptVersionSummary = Schemas['ScriptVersionSummary'];
export type PublishedScriptVersion = Schemas['PublishedScriptVersion'];
export type ScriptTestRun = Schemas['ScriptTestRun'];
export type ScriptTestRunSummary = Schemas['ScriptTestRunSummary'];

export interface ScriptPage {
  items: ScriptSummary[];
}

/**
 * The scripts API of one namespace (feature 005): drafts, versions and test runs. The namespace is
 * in the path as an *id* (the rest of the API takes a slug in a header). Every id that ends up in
 * a path goes through `path`, which encodes it, so an id from the address bar cannot change which
 * resource is meant; a change carries an idempotency key, which the caller keeps across retries of
 * the same intent.
 */
export function scriptsApi(client: ApiClient, namespaceId: string) {
  const all = () => path`/namespaces/${namespaceId}/scripts`;
  const one = (id: string) => path`/namespaces/${namespaceId}/scripts/${id}`;
  const versions = (id: string) => path`/namespaces/${namespaceId}/scripts/${id}/versions`;
  const version = (id: string, number: number) => path`/namespaces/${namespaceId}/scripts/${id}/versions/${number}`;
  const testRuns = (id: string) => path`/namespaces/${namespaceId}/scripts/${id}/test-runs`;
  const testRun = (id: string, testRunId: string) => path`/namespaces/${namespaceId}/scripts/${id}/test-runs/${testRunId}`;
  return {
    namespaceId,
    list: (query: { after?: string; limit?: number } = {}, signal?: AbortSignal) => client.get<ScriptPage>(all(), { query, signal }),
    create: (body: CreateScriptRequest, key?: string) => client.post<Script>(all(), { body, ...(key ? { key } : {}) }),
    get: (id: string, signal?: AbortSignal) => client.get<Script>(one(id), { signal }),
    update: (id: string, body: UpdateScriptRequest, key?: string) => client.patch<Script>(one(id), { body, ...(key ? { key } : {}) }),
    remove: (id: string, key?: string) => client.delete<undefined>(one(id), key ? { key } : {}),

    versions: (id: string, query: { before?: number; limit?: number } = {}, signal?: AbortSignal) =>
      client.get<{ items: ScriptVersionSummary[] }>(versions(id), { query, signal }),
    version: (id: string, number: number, signal?: AbortSignal) => client.get<ScriptVersion>(version(id, number), { signal }),
    publish: (id: string, key?: string) => client.post<PublishedScriptVersion>(versions(id), key ? { key } : {}),

    testRuns: (id: string, query: { limit?: number } = {}, signal?: AbortSignal) => client.get<{ items: ScriptTestRunSummary[] }>(testRuns(id), { query, signal }),
    startTestRun: (id: string, input: unknown, key?: string) => client.post<ScriptTestRun>(testRuns(id), { body: input === undefined ? {} : { input }, ...(key ? { key } : {}) }),
    testRun: (id: string, testRunId: string, signal?: AbortSignal) => client.get<ScriptTestRun>(testRun(id, testRunId), { signal }),
  };
}

export type ScriptsApi = ReturnType<typeof scriptsApi>;
