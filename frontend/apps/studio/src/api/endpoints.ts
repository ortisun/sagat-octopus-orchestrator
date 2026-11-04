import type { ApiClient } from './client';
import { path } from './client';
import type { components } from './schema';
import type { DefinitionDiff, GraphChanges, JournalEvent, OperationRequest, OperationResult, Page, Run, RunTelemetry, RuntimeGraph, Validation, VersionSummary, Workflow, WorkflowVersion } from './types';

type Schemas = components['schemas'];

export type AuthContext = Schemas['AuthContext'];
export type NamespaceResource = Schemas['NamespaceResource'];
export type VersionDocument = Schemas['WorkflowVersionDocument'];
export type BatchJob = Schemas['BatchJob'];
export type BatchItem = Schemas['BatchItem'];
export type BatchPreview = Schemas['BatchPreview'];
export type BatchSelector = Schemas['BatchSelector'];
/**
 * What `createBatchOperation` takes. The generated type makes the fields that have a default
 * required and the payload of a signal an empty object; the contract has them optional and free.
 */
export type BatchCreateRequest = Omit<Schemas['BatchCreateRequest'], 'businessId' | 'maxItems' | 'itemsPerSecond' | 'signal'> & {
  businessId?: Schemas['BatchCreateRequest']['businessId'];
  maxItems?: number;
  itemsPerSecond?: number;
  signal?: { name: string; payload: Record<string, unknown> };
};

async function collectPages<T>(load: (cursor: string | undefined) => Promise<Page<T>>): Promise<T[]> {
  const items: T[] = [];
  let cursor: string | undefined;
  do {
    const page = await load(cursor);
    items.push(...page.items);
    cursor = page.nextCursor ?? undefined;
  } while (cursor);
  return items;
}

/** The operations of the API that the Studio uses, with the shapes of the contract. */
export function endpoints(api: ApiClient) {
  return {
    authContext: (signal?: AbortSignal) => api.get<AuthContext>('/auth/context', { signal }),
    workspaces: (signal?: AbortSignal) => collectPages((cursor) => api.get<Schemas['WorkspacePage']>('/workspaces', { query: { limit: 200, cursor }, signal })),
    createNamespace: (workspace: string, body: Schemas['CreateNamespaceRequest']) => api.post<NamespaceResource>(path`/workspaces/${workspace}/namespaces`, { body }),
    namespaces: async (signal?: AbortSignal): Promise<Page<NamespaceResource>> => ({
      items: await collectPages((cursor) => api.get<Page<NamespaceResource>>('/namespaces', { query: { limit: 200, cursor }, signal })),
      nextCursor: null,
    }),

    listWorkflows: (query: { cursor?: string; limit?: number } = {}, signal?: AbortSignal) => api.get<Page<Workflow>>('/workflows', { query, signal }),
    getWorkflow: (id: string, signal?: AbortSignal) => api.get<Workflow>(path`/workflows/${id}`, { signal, safeNumbers: true }),
    createWorkflow: (name: string, ir: Record<string, unknown>, key?: string) => api.post<Workflow>('/workflows', { body: { name, ir }, ...(key ? { key } : {}) }),
    updateDraft: (id: string, draftRevision: number, ir: Record<string, unknown>, key?: string) => api.put<Workflow>(path`/workflows/${id}`, { body: { draftRevision, ir }, ...(key ? { key } : {}) }),
    validate: (id: string, ir: Record<string, unknown> | undefined, signal?: AbortSignal) => api.post<Validation>(path`/workflows/${id}/validate`, { body: ir ? { ir } : {}, signal }),
    diff: (id: string, from?: string, to?: string, signal?: AbortSignal) => api.get<DefinitionDiff>(path`/workflows/${id}/diff`, { query: { from, to }, signal }),
    publish: (id: string, draftRevision: number, releaseNotes?: string, key?: string) =>
      api.post<WorkflowVersion>(path`/workflows/${id}/publish`, { body: { draftRevision, ...(releaseNotes ? { releaseNotes } : {}) }, ...(key ? { key } : {}) }),
    versions: (id: string, query: { cursor?: string; limit?: number } = {}, signal?: AbortSignal) => api.get<Page<VersionSummary>>(path`/workflows/${id}/versions`, { query, signal }),
    version: (id: string, number: number, signal?: AbortSignal) => api.get<VersionDocument>(path`/workflows/${id}/versions/${number}`, { signal, safeNumbers: true }),

    listRuns: (query: { cursor?: string; limit?: number; workflowId?: string; status?: string; origin?: string } = {}, signal?: AbortSignal) => api.get<Page<Run>>('/runs', { query, signal }),
    startRun: (workflowId: string, input: unknown) => api.post<Run>('/runs', { body: { workflowId, input } }),
    getRun: (id: string, signal?: AbortSignal) => api.get<Run>(path`/runs/${id}`, { signal }),
    graph: (id: string, signal?: AbortSignal) => api.get<RuntimeGraph>(path`/runs/${id}/graph`, { signal }),
    changes: (id: string, since: number, signal?: AbortSignal) => api.get<GraphChanges>(path`/runs/${id}/graph/changes`, { query: { since }, signal }),
    journal: (id: string, query: { cursor?: string; limit?: number } = {}, signal?: AbortSignal) => api.get<Page<JournalEvent>>(path`/runs/${id}/events`, { query, signal }),
    telemetry: (id: string, signal?: AbortSignal) => api.get<RunTelemetry>(path`/runs/${id}/telemetry`, { signal }),
    operate: (id: string, request: OperationRequest, key?: string) => api.post<OperationResult>(path`/runs/${id}/operations`, { body: request, ...(key ? { key } : {}) }),

    // Batch operations (feature 018): the routes carry the namespace id, not its slug.
    previewBatch: (namespaceId: string, body: { selector: BatchSelector; maxItems?: number }, signal?: AbortSignal) =>
      api.post<BatchPreview>(path`/namespaces/${namespaceId}/batch-operations/preview`, { body, signal }),
    createBatch: (namespaceId: string, body: BatchCreateRequest, key: string, signal?: AbortSignal) =>
      api.post<BatchJob>(path`/namespaces/${namespaceId}/batch-operations`, { body, key, signal }),
    listBatches: (namespaceId: string, query: { cursor?: string; limit?: number; state?: string } = {}, signal?: AbortSignal) =>
      api.get<Page<BatchJob>>(path`/namespaces/${namespaceId}/batch-operations`, { query, signal }),
    getBatch: (namespaceId: string, id: string, signal?: AbortSignal) => api.get<BatchJob>(path`/namespaces/${namespaceId}/batch-operations/${id}`, { signal }),
    batchItems: (namespaceId: string, id: string, query: { cursor?: string; limit?: number; state?: string } = {}, signal?: AbortSignal) =>
      api.get<Page<BatchItem>>(path`/namespaces/${namespaceId}/batch-operations/${id}/items`, { query, signal }),
    cancelBatch: (namespaceId: string, id: string, reason: string, key: string, signal?: AbortSignal) =>
      api.post<BatchJob>(path`/namespaces/${namespaceId}/batch-operations/${id}/cancel`, { body: { reason }, key, signal }),
  };
}

export type Endpoints = ReturnType<typeof endpoints>;
