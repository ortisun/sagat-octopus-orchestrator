import { useQuery } from '@tanstack/react-query';
import { useApp, useCan, useNamespace, useSession } from '../../app/context';
import { ErrorState } from '../../ui/ErrorState';
import { AlertsPanel } from './AlertsPanel';
import { operationsApi } from './api';
import { WorkersPanel } from './WorkersPanel';
import './operations.css';

/** Alerts and worker controls backed by feature 008's namespace-scoped operational read models. */
export function OperationsPage() {
  const { api, ep } = useApp();
  const current = useNamespace();
  const session = useSession();
  const canRead = useCan('ops:read');
  const canManage = useCan('worker:manage');
  const namespaces = useQuery({
    queryKey: ['namespaces'],
    queryFn: ({ signal }) => ep.namespaces(signal),
    enabled: canRead,
  });
  const namespace = namespaces.data?.items.find((item) => item.slug === current);

  if (!canRead) {
    return <section><h1>Operations</h1><p>Viewing operational alerts and worker identities requires ops:read permission.</p></section>;
  }
  if (namespaces.isPending) {
    return <section><h1>Operations</h1><p role="status">Loading the operations context…</p></section>;
  }
  if (namespaces.isError) {
    return <section><h1>Operations</h1><ErrorState error={namespaces.error} what="Loading namespaces" onRetry={() => void namespaces.refetch()} /></section>;
  }
  if (!namespace) {
    return <section><h1>Operations</h1><p>Select a namespace to inspect its alerts and workers.</p></section>;
  }

  const sessionKey = `${session.info?.tenant ?? 'signed-out'}:${session.info?.subject ?? 'signed-out'}:${session.expiresAt ?? 'session'}`;
  const service = operationsApi(api, namespace.id);
  return (
    <section className="operations-page" aria-labelledby="operations-title" key={`${sessionKey}:${namespace.id}`}>
      <header className="operations-heading">
        <h1 id="operations-title">Operations</h1>
        <p>Review recorded alert conditions and control which worker identities can claim new attempts in <strong>{namespace.slug}</strong>.</p>
      </header>
      <div className="operations-layout">
        <AlertsPanel api={service} namespaceId={namespace.id} sessionKey={sessionKey} />
        <WorkersPanel api={service} canManage={canManage} namespace={namespace.slug} namespaceId={namespace.id} sessionKey={sessionKey} />
      </div>
    </section>
  );
}
