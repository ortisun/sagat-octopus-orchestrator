import { ConnectionProbe } from './ConnectionProbe';
import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useApp, useCan, useNamespace } from '../../app/context';
import { Button, ColumnHeader, Dialog, Empty, ErrorState, Hint, Link, Loading, Page, PageHeader, Row, Stack, Table, RowHeader, TableBody, TableCell, TableHead, TableRow, Text } from '../../ui';
import { ConnectionEditor } from './ConnectionEditor';
import { DeleteConnection } from './DeleteConnection';
import { connectionsApi, editableIcebergOf, icebergOf, postgresOf } from './api';
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
  // The deployment lists Iceberg only when it runs it (`lake.enabled`); a probe of an Iceberg connection needs that, and the Studio never guesses.
  const drivers = useQuery({ queryKey: ['database-drivers', namespace], retry: false, queryFn: ({ signal }) => connectionsApi(api, namespace).drivers(signal) });
  const lakeOffered = drivers.data?.items.some(driver => driver.id === 'iceberg') ?? false;
  return (
    <Page>
      <Stack as="section" gap={4} aria-labelledby="connections-title">
        <PageHeader title={<span id="connections-title">Connections</span>} actions={canManage ? <Button variant="primary" onClick={() => setEditing('new')}>New connection</Button> : null}>
          Configure PostgreSQL databases and lake connections, and the credential references they use, for this namespace.
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
              <TableBody>{list.data.items.map(item => {
                const postgres = postgresOf(item); const iceberg = icebergOf(item); const editable = !!postgres || !!editableIcebergOf(item);
                return (
                  <TableRow key={item.id}>
                    <RowHeader>{item.name}</RowHeader>
                    <TableCell>{postgres ? `${postgres.host}:${postgres.port} / ${postgres.database}` : item.config.driver}</TableCell>
                    <TableCell>{item.revision}</TableCell>
                    <TableCell>{item.status === 'ACTIVE' ? 'Active' : 'Paused'}</TableCell>
                    <TableCell>{postgres ? postgres.secretName : iceberg?.catalogSecret ?? '—'}</TableCell>
                    <TableCell>
                      <Row>
                        {canManage ? <>{editable ? <Button aria-label={`Edit connection ${item.name}`} onClick={() => setEditing(item)}>Edit</Button> : null}<Button aria-label={`Delete connection ${item.name}`} onClick={() => setDeleting(item)}>Delete</Button></> : 'Read only'}
                        {canReadRuns && (postgres || (lakeOffered && editableIcebergOf(item))) ? <Button aria-label={`Probe connection ${item.name}`} onClick={() => setProbing(item)}>Probe / status</Button> : null}
                      </Row>
                    </TableCell>
                  </TableRow>
                );
              })}</TableBody>
            </Table>
          </div>
        ) : null}
        <Dialog open={editing !== null} title={editing === 'new' ? 'New connection' : 'Edit connection'} onClose={() => setEditing(null)}>{editing !== null ? <ConnectionEditor namespace={namespace} development={development} {...(editing === 'new' ? {} : { connection: editing })} onClose={() => setEditing(null)} /> : null}</Dialog>
        <Dialog open={probing !== null} title="Connection probe" onClose={() => setProbing(null)}>{probing ? <ConnectionProbe key={`${namespace}:${probing.name}:${probing.revision}`} namespace={namespace} connection={probing} /> : null}</Dialog>
        <Dialog open={deleting !== null} title="Delete connection" onClose={() => setDeleting(null)}>{deleting ? <DeleteConnection namespace={namespace} connection={deleting} onClose={() => setDeleting(null)} /> : null}</Dialog>
      </Stack>
    </Page>
  );
}
