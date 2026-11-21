import { useQuery } from '@tanstack/react-query';
import { useApp, useCan, useNamespace, useSession } from '../../app/context';
import { ErrorState, Loading, Page, PageHeader, Stack, Text } from '../../ui';
import { AlertsPanel } from './AlertsPanel';
import { operationsApi } from './api';
import { WorkersPanel } from './WorkersPanel';
import './operations.css';

/** Alerts and worker controls backed by feature 008's namespace-scoped operational read models. */
export function OperationsPage() {
  const { api, ep, session } = useApp();
  const current = useNamespace();
  useSession();
  const canRead = useCan('ops:read');
  const canManage = useCan('worker:manage');
  const namespaces = useQuery({
    queryKey: ['namespaces'],
    queryFn: ({ signal }) => ep.namespaces(signal),
    enabled: canRead,
  });
  const namespace = namespaces.data?.items.find((item) => item.slug === current);

  if (!canRead) {
    return <Page><PageHeader title="Operations" /><Text>Viewing operational alerts and worker identities requires ops:read permission.</Text></Page>;
  }
  if (namespaces.isPending) {
    return <Page><PageHeader title="Operations" /><Loading>Loading the operations context…</Loading></Page>;
  }
  if (namespaces.isError) {
    return <Page><PageHeader title="Operations" /><ErrorState error={namespaces.error} what="Loading namespaces" onRetry={() => void namespaces.refetch()} /></Page>;
  }
  if (!namespace) {
    return <Page><PageHeader title="Operations" /><Text>Select a namespace to inspect its alerts and workers.</Text></Page>;
  }

  const sessionKey = String(session.revision());
  const service = operationsApi(api, namespace.id);
  return (
    <Page key={`${sessionKey}:${namespace.id}`}>
      <Stack as="section" gap={4} aria-labelledby="operations-title">
        <PageHeader title={<span id="operations-title">Operations</span>}>
          Review recorded alert conditions and control which worker identities can claim new attempts in <strong>{namespace.slug}</strong>.
        </PageHeader>
        <div className="operations-layout">
          <AlertsPanel api={service} namespaceId={namespace.id} sessionKey={sessionKey} />
          <WorkersPanel api={service} canManage={canManage} namespace={namespace.slug} namespaceId={namespace.id} sessionKey={sessionKey} />
        </div>
      </Stack>
    </Page>
  );
}
