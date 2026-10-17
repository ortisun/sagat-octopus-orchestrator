import type { ApiClient } from '../../api/client';
import { path } from '../../api/client';
import type { components } from '../../api/schema';
type Schemas = components['schemas'];
export type Connection = Schemas['DatabaseConnection'];
export type ConnectionWrite = Schemas['PutDatabaseConnectionRequest'];
export function connectionsApi(api: ApiClient, namespace: string) {
  const base = path`/namespaces/${namespace}/connections`;
  return {
    list: (signal?: AbortSignal) => api.get<Schemas['DatabaseConnectionList']>(base, { signal }),
    put: (name: string, body: ConnectionWrite) => api.put<Connection>(path`/namespaces/${namespace}/connections/${name}`, { body }),
    delete: (connection: Connection, requestId: string) => api.delete<Connection>(path`/namespaces/${namespace}/connections/${connection.name}`, { body: { expectedRevision: connection.revision, requestId } }),
  };
}
