import { ConnectionProbe } from './ConnectionProbe';
import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useApp, useCan, useNamespace } from '../../app/context';
import { Button, ColumnHeader, Dialog, Empty, ErrorState, Hint, Link, Loading, Page, PageHeader, Row, Stack, Table, RowHeader, TableBody, TableCell, TableHead, TableRow, Text } from '../../ui';
import { ConnectionForm } from './ConnectionForm';
import { DeleteConnection } from './DeleteConnection';
import { connectionsApi } from './api';
import type { Connection } from './api';
import '../../styles/namespaces.css';
import '../../styles/connections.css';

export function ConnectionsPage() {
  const { ep } = useApp(); const current = useNamespace(); const allowed = useCan('namespace:read');
  const namespaces = useQuery({ queryKey: ['namespaces'], queryFn: ({ signal }) => ep.namespaces(signal) });
  const namespace = namespaces.data?.items.find(n => n.slug === current);
  if (!allowed) return <Page><PageHeader title="Connections" /><Text>Viewing connections requires namespace:read permission.</Text></Page>;
  if (namespaces.error) return <Page><ErrorState error={namespaces.error} what="Loading namespaces" /></Page>;
  if (!namespace) return <Page><PageHeader title="Connections" /><Text>Select a namespace to manage database connections.</Text></Page>;
  return <ConnectionList key={namespace.id} namespace={namespace.id} development={namespace.environment === 'dev'} />;
}
function ConnectionList({ namespace, development }: { namespace: string; development: boolean }) {
  const { api } = useApp(); const canManage = useCan('database:manage'); const canAdHoc = useCan('database:adhoc'); const canReadRuns = useCan('run:read');
  const [editing, setEditing] = useState<Connection | 'new' | null>(null);
  const [probing, setProbing] = useState<Connection | null>(null);
  const [deleting, setDeleting] = useState<Connection | null>(null);
  const list = useQuery({ queryKey: ['connections', namespace], queryFn: ({ signal }) => connectionsApi(api, namespace).list(signal) });
  return (
    <Page>
      <Stack as="section" gap={4} aria-labelledby="connections-title">
        <PageHeader title={<span id="connections-title">Connections</span>} actions={canManage ? <Button variant="primary" onClick={() => setEditing('new')}>New connection</Button> : null}>
          Configure PostgreSQL databases and their credential references for this namespace.
        </PageHeader>
        {canAdHoc ? <Text><Link to="/database-executions">Run ad hoc database work</Link></Text> : null}
        <Hint>Each edit creates a revision. Active and paused describe access; saving settings does not confirm connectivity.</Hint>
        {list.isPending ? <Loading>Loading connections…</Loading> : null}
        {list.error ? <ErrorState error={list.error} what="Loading connections" onRetry={() => { void list.refetch(); }} /> : null}
        {list.data?.items.length === 0 ? <Empty title="No database connections yet." headingLevel={2} /> : null}
        {list.data && list.data.items.length > 0 ? (
          // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- keyboard scrolling for a wide table
          <div className="namespaces-table" role="region" aria-label="Database connections" tabIndex={0}>
            <Table label="Database connections of this namespace">
              <TableHead><TableRow><ColumnHeader>Name</ColumnHeader><ColumnHeader>Database</ColumnHeader><ColumnHeader>Revision</ColumnHeader><ColumnHeader>Status</ColumnHeader><ColumnHeader>Credentials</ColumnHeader><ColumnHeader>Actions</ColumnHeader></TableRow></TableHead>
              <TableBody>{list.data.items.map(item => (
                <TableRow key={item.id}>
                  <RowHeader>{item.name}</RowHeader>
                  <TableCell>{item.config.host}:{item.config.port} / {item.config.database}</TableCell>
                  <TableCell>{item.revision}</TableCell>
                  <TableCell>{item.status === 'ACTIVE' ? 'Active' : 'Paused'}</TableCell>
                  <TableCell>{item.config.secretName}</TableCell>
                  <TableCell>
                    <Row>
                      {canManage ? <><Button aria-label={`Edit connection ${item.name}`} onClick={() => setEditing(item)}>Edit</Button><Button aria-label={`Delete connection ${item.name}`} onClick={() => setDeleting(item)}>Delete</Button></> : 'Read only'}
                      {canReadRuns ? <Button aria-label={`Probe connection ${item.name}`} onClick={() => setProbing(item)}>Probe / status</Button> : null}
                    </Row>
                  </TableCell>
                </TableRow>
              ))}</TableBody>
            </Table>
          </div>
        ) : null}
        <Dialog open={editing !== null} title={editing === 'new' ? 'New connection' : 'Edit connection'} onClose={() => setEditing(null)}>{editing !== null ? <ConnectionForm namespace={namespace} development={development} {...(editing === 'new' ? {} : { connection: editing })} onClose={() => setEditing(null)} /> : null}</Dialog>
        <Dialog open={probing !== null} title="Connection probe" onClose={() => setProbing(null)}>{probing ? <ConnectionProbe key={`${namespace}:${probing.name}:${probing.revision}`} namespace={namespace} connection={probing} /> : null}</Dialog>
        <Dialog open={deleting !== null} title="Delete connection" onClose={() => setDeleting(null)}>{deleting ? <DeleteConnection namespace={namespace} connection={deleting} onClose={() => setDeleting(null)} /> : null}</Dialog>
      </Stack>
    </Page>
  );
}
