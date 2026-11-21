import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useApp, useCan, useNamespace } from '../../app/context';
import { Button, ColumnHeader, Dialog, Empty, ErrorState, Hint, Loading, Page, PageHeader, Stack, Table, RowHeader, TableBody, TableCell, TableHead, TableRow, Text } from '../../ui';
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
  if (!allowed) return <Page><PageHeader title="Integrations" /><Text>Viewing integrations requires event:read permission.</Text></Page>;
  if (namespaces.error) return <Page><ErrorState error={namespaces.error} what="Loading namespaces" /></Page>;
  if (!namespace) return <Page><PageHeader title="Integrations" /><Text>Select a namespace to manage its integrations.</Text></Page>;
  return <IntegrationList key={namespace.id} namespace={namespace.id} />;
}

function IntegrationList({ namespace }: { namespace: string }) {
  const { api } = useApp();
  const canManage = useCan('event:manage');
  const canNetwork = useCan('egress:manage');
  const service = integrationsApi(api, namespace);
  const list = useQuery({ queryKey: ['integrations', namespace], queryFn: ({ signal }) => service.list(signal), refetchInterval: 15_000 });
  const [editing, setEditing] = useState<Connector | 'new' | null>(null);
  return (
    <Page>
      <Stack as="section" gap={4} aria-labelledby="integrations-title">
        <PageHeader title={<span id="integrations-title">Integrations</span>} actions={canManage ? <Button variant="primary" onClick={() => setEditing('new')}>New integration</Button> : null}>
          Receive events from Kafka, NATS JetStream or SQS in this namespace. Configure the connection and its credentials here.
        </PageHeader>
        <Hint>Active means consumption is enabled. It does not confirm that the broker is reachable. The platform’s internal NATS is configured by the deployment.</Hint>
        {canNetwork ? <NetworkAccess namespace={namespace} /> : <Hint>A namespace administrator must allow the broker hosts and ports in its network access policy.</Hint>}
        {list.isPending ? <Loading>Loading integrations…</Loading> : null}
        {list.error ? <ErrorState error={list.error} what="Loading integrations" onRetry={() => { void list.refetch(); }} /> : null}
        {list.data?.items.length === 0 ? <Empty title="No integrations configured yet." headingLevel={2} /> : null}
        {list.data && list.data.items.length > 0 ? (
          // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- keyboard users must be able to scroll the table
          <div className="namespaces-table" role="region" aria-label="Integrations" tabIndex={0}>
            <Table label="Broker integrations of this namespace">
              <TableHead><TableRow><ColumnHeader>Name</ColumnHeader><ColumnHeader>Type</ColumnHeader><ColumnHeader>Status</ColumnHeader><ColumnHeader>Credentials</ColumnHeader><ColumnHeader>Runtime</ColumnHeader><ColumnHeader>Actions</ColumnHeader></TableRow></TableHead>
              <TableBody>{list.data.items.map((item) => (
                <TableRow key={item.id}>
                  <RowHeader>{item.slug}</RowHeader>
                  <TableCell>{item.kind}</TableCell>
                  <TableCell>{item.status === 'ACTIVE' ? 'Active' : 'Paused'}</TableCell>
                  <TableCell>{item.secretName ? 'Configured' : 'None'}</TableCell>
                  <TableCell>{item.health.lastError ?? (item.health.lastMessageAt ? 'Messages received' : 'No messages received yet')}</TableCell>
                  <TableCell>{canManage ? <Button aria-label={`Edit integration ${item.slug}`} onClick={() => setEditing(item)}>Edit</Button> : 'Read only'}</TableCell>
                </TableRow>
              ))}</TableBody>
            </Table>
          </div>
        ) : null}
        <Dialog open={editing !== null} title={editing === 'new' ? 'New integration' : 'Edit integration'} onClose={() => setEditing(null)}>
          {editing !== null ? <IntegrationForm namespace={namespace} connector={editing === 'new' ? undefined : editing} onClose={() => setEditing(null)} /> : null}
        </Dialog>
      </Stack>
    </Page>
  );
}
