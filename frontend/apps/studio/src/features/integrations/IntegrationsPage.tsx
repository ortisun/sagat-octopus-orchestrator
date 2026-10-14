import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useApp, useCan, useNamespace } from '../../app/context';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { Dialog } from '../../ui/Dialog';
import { integrationsApi } from './api';
import type { Connector } from './api';
import { NetworkAccess } from './NetworkAccess';
import { IntegrationForm } from './IntegrationForm';
import '../../styles/namespaces.css';

export function IntegrationsPage() {
  const { ep } = useApp();
  const current = useNamespace();
  const allowed = useCan('event:read');
  const namespaces = useQuery({ queryKey: ['namespaces'], queryFn: ({ signal }) => ep.namespaces(signal) });
  const namespace = namespaces.data?.items.find((item) => item.slug === current);
  if (!allowed) return <section><h1>Integrations</h1><p>Viewing integrations requires event:read permission.</p></section>;
  if (namespaces.error) return <ErrorState error={namespaces.error} what="Loading namespaces" />;
  if (!namespace) return <section><h1>Integrations</h1><p>Select a namespace to manage its integrations.</p></section>;
  return <IntegrationList key={namespace.id} namespace={namespace.id} />;
}

function IntegrationList({ namespace }: { namespace: string }) {
  const { api } = useApp();
  const canManage = useCan('event:manage');
  const canNetwork = useCan('egress:manage');
  const service = integrationsApi(api, namespace);
  const list = useQuery({ queryKey: ['integrations', namespace], queryFn: ({ signal }) => service.list(signal), refetchInterval: 15_000 });
  const [editing, setEditing] = useState<Connector | 'new' | null>(null);
  return <section aria-labelledby="integrations-title">
    <div className="namespaces-heading"><h1 id="integrations-title">Integrations</h1>{canManage ? <Button variant="primary" onClick={() => setEditing('new')}>New integration</Button> : null}</div>
    <p>Receive events from Kafka, NATS JetStream or SQS in this namespace. Configure the connection and its credentials here.</p>
    <p className="hint">Active means consumption is enabled. It does not confirm that the broker is reachable. The platform’s internal NATS is configured by the deployment.</p>
    {canNetwork ? <NetworkAccess namespace={namespace} /> : <p className="hint">A namespace administrator must allow the broker hosts and ports in its network access policy.</p>}
    {list.isPending ? <p role="status">Loading integrations…</p> : null}
    {list.error ? <ErrorState error={list.error} what="Loading integrations" onRetry={() => { void list.refetch(); }} /> : null}
    {list.data?.items.length === 0 ? <p className="empty">No integrations configured yet.</p> : null}
    {list.data && list.data.items.length > 0 ? (
      // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- keyboard users must be able to scroll the table
      <div className="namespaces-table" role="region" aria-label="Integrations" tabIndex={0}>
        <table className="data"><caption className="visually-hidden">Broker integrations of this namespace</caption>
          <thead><tr><th scope="col">Name</th><th scope="col">Type</th><th scope="col">Status</th><th scope="col">Credentials</th><th scope="col">Runtime</th><th scope="col">Actions</th></tr></thead>
          <tbody>{list.data.items.map((item) => <tr key={item.id}><th scope="row">{item.slug}</th><td>{item.kind}</td><td>{item.status === 'ACTIVE' ? 'Active' : 'Paused'}</td><td>{item.secretName ? 'Configured' : 'None'}</td><td>{item.health.lastError ?? (item.health.lastMessageAt ? 'Messages received' : 'No messages received yet')}</td><td>{canManage ? <Button aria-label={`Edit integration ${item.slug}`} onClick={() => setEditing(item)}>Edit</Button> : 'Read only'}</td></tr>)}</tbody>
        </table>
      </div>
    ) : null}
    <Dialog open={editing !== null} title={editing === 'new' ? 'New integration' : 'Edit integration'} onClose={() => setEditing(null)}>
      {editing !== null ? <IntegrationForm namespace={namespace} connector={editing === 'new' ? undefined : editing} onClose={() => setEditing(null)} /> : null}
    </Dialog>
  </section>;
}
