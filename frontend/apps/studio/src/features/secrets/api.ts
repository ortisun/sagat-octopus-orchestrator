import type { ApiClient } from '../../api/client';
import { path } from '../../api/client';
import type { components } from '../../api/schema';
export type Secret = components['schemas']['Secret'];
export function secretsApi(api: ApiClient, namespace: string) {
  const base = path`/namespaces/${namespace}/secrets`;
  return {
    list: (signal?: AbortSignal) => api.get<components['schemas']['SecretList']>(base, { signal }),
    put: (name: string, value: string, expectedVersion: number, requestId: string) => api.put<Secret>(path`/namespaces/${namespace}/secrets/${name}`, { body: { value, expectedVersion, requestId, requiredProvider: 'openbao' } }),
    migrate: (secret: Secret, requestId: string) => api.post<Secret>(path`/namespaces/${namespace}/secrets/${secret.name}/migrate`, { body: { expectedVersion: secret.version, requestId } }),
  };
}
