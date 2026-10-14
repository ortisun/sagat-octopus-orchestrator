import type { ApiClient } from '../../api/client';
import { path } from '../../api/client';
import type { components, operations } from '../../api/schema';

type Schemas = components['schemas'];

export type Alert = Schemas['Alert'];
export type AlertPage = Schemas['AlertPage'];
export type Worker = Schemas['Worker'];
export type Drain = Schemas['Drain'];
export type WorkerList = Schemas['WorkerList'];
export type AlertState = NonNullable<NonNullable<operations['listAlerts']['parameters']['query']>['state']>;

export function operationsApi(client: ApiClient, namespaceId: string) {
  const base = path`/namespaces/${namespaceId}`;
  return {
    alerts: (query: { state?: AlertState; cursor?: string; limit?: number } = {}, signal?: AbortSignal) =>
      client.get<AlertPage>(`${base}/alerts`, { query, signal }),
    workers: (signal?: AbortSignal) => client.get<WorkerList>(`${base}/workers`, { signal }),
    drain: (subject: string, reason: string, key: string, signal?: AbortSignal) =>
      client.post<Drain>(`${base}${path`/workers/${subject}/drain`}`, { body: { reason }, key, signal }),
    resume: (subject: string, reason: string, key: string, signal?: AbortSignal) =>
      client.post<Schemas['Undrain']>(`${base}${path`/workers/${subject}/undrain`}`, { body: { reason }, key, signal }),
  };
}

export type OperationsApi = ReturnType<typeof operationsApi>;
