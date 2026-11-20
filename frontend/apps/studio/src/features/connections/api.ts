import type { ApiClient } from '../../api/client';
import { path } from '../../api/client';
import type { components } from '../../api/schema';
type Schemas = components['schemas'];
export type Connection = Schemas['DatabaseConnection'];
export type ConnectionWrite = Schemas['PutDatabaseConnectionRequest'];
export type PostgresConfig = Extract<Connection['config'], { driver: 'postgresql' }>;
export type IcebergConfig = Extract<Connection['config'], { driver: 'iceberg' }>;
/** The Studio edits and probes PostgreSQL connections. The other drivers are listed by name until their forms exist. */
export function postgresOf(connection: Connection): PostgresConfig | undefined { return connection.config.driver === 'postgresql' ? connection.config : undefined; }
/** An Iceberg connection of any catalog and storage type. */
export function icebergOf(connection: Connection): IcebergConfig | undefined { return connection.config.driver === 'iceberg' ? connection.config : undefined; }
/** The shape the Studio form edits: a REST catalog over S3-compatible storage. The Glue, Hive, JDBC, GCS and ADLS shapes are listed and deleted here until their forms exist (T035-078). */
export function editableIcebergOf(connection: Connection): IcebergConfig | undefined {
  const config = icebergOf(connection);
  return config && config.catalog.type === 'rest' && config.catalog.rest && config.storage.type === 's3' && config.storage.s3 ? config : undefined;
}
export function connectionsApi(api: ApiClient, namespace: string) {
  const base = path`/namespaces/${namespace}/connections`;
  return {
    list: (signal?: AbortSignal) => api.get<Schemas['DatabaseConnectionList']>(base, { signal }),
    put: (name: string, body: ConnectionWrite) => api.put<Connection>(path`/namespaces/${namespace}/connections/${name}`, { body }),
    delete: (connection: Connection, requestId: string) => api.delete<Connection>(path`/namespaces/${namespace}/connections/${connection.name}`, { body: { expectedRevision: connection.revision, requestId } }),
    /** The drivers this deployment offers; the Studio never hard-codes the list (contracts/connections.md). */
    drivers: (signal?: AbortSignal) => api.get<Schemas['DatabaseDriverList']>(path`/namespaces/${namespace}/database-drivers`, { signal }),
  };
}
