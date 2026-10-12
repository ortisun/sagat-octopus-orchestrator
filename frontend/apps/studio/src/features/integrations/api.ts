import type { ApiClient } from '../../api/client';
import { path } from '../../api/client';
import type { components } from '../../api/schema';

type Schemas = components['schemas'];
export type Connector = Schemas['Connector'];
export type ConnectorInput = Omit<Schemas['CreateConnectorRequest'], 'settings'> & { settings: Connector['settings'] };
export function integrationsApi(api: ApiClient, namespace: string) {
  const base = path`/namespaces/${namespace}/connectors`;
  return {
    network: (signal?: AbortSignal) => api.get<Schemas['EgressPolicy']>(path`/namespaces/${namespace}/egress-policy`, { signal }),
    saveNetwork: (body: Schemas['SetEgressPolicyRequest']) => api.put<Schemas['EgressPolicy']>(path`/namespaces/${namespace}/egress-policy`, { body }),
    list: (signal?: AbortSignal) => api.get<Schemas['ConnectorList']>(base, { signal }),
    save: (body: ConnectorInput, id?: string) => {
      if (!id) return api.post<Connector>(base, { body });
      const { kind, settings, secretName, defaultEventType, ratePerSecond, status } = body;
      const update = { kind, settings, secretName, defaultEventType, ratePerSecond, status };
      return api.put<Connector>(path`/namespaces/${namespace}/connectors/${id}`, { body: update });
    },
    credentials: (name: string, value: string) => api.put<Schemas['Secret']>(path`/namespaces/${namespace}/secrets/${name}`, { body: { value } }),
  };
}
