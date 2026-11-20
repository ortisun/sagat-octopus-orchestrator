import { Link } from 'react-router';
import { ConnectionProbe } from './ConnectionProbe';
import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useApp, useCan, useNamespace } from '../../app/context';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { Dialog } from '../../ui/Dialog';
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
  if (!allowed) return <section><h1>Connections</h1><p>Viewing connections requires namespace:read permission.</p></section>;
  if (namespaces.error) return <ErrorState error={namespaces.error} what="Loading namespaces" />;
  if (!namespace) return <section><h1>Connections</h1><p>Select a namespace to manage database connections.</p></section>;
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
  return <section aria-labelledby="connections-title">
    <div className="namespaces-heading"><h1 id="connections-title">Connections</h1>{canManage ? <Button variant="primary" onClick={() => setEditing('new')}>New connection</Button> : null}</div>
    {canAdHoc ? <p><Link className="btn" to="/database-executions">Run ad hoc database work</Link></p> : null}
    <p>Configure PostgreSQL databases and lake connections, and the credential references they use, for this namespace.</p>
    <p className="hint">Each edit creates a revision. Active and paused describe access; saving settings does not confirm connectivity.</p>
    {list.isPending ? <p role="status">Loading connections…</p> : null}
    {list.error ? <ErrorState error={list.error} what="Loading connections" onRetry={() => { void list.refetch(); }} /> : null}
    {list.data?.items.length === 0 ? <p className="empty">No database connections yet.</p> : null}
    {list.data && list.data.items.length > 0 ? (
      // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- keyboard scrolling for a wide table
      <div className="namespaces-table" role="region" aria-label="Database connections" tabIndex={0}>
        <table className="data"><caption className="visually-hidden">Database connections of this namespace</caption><thead><tr><th scope="col">Name</th><th scope="col">Database</th><th scope="col">Revision</th><th scope="col">Status</th><th scope="col">Credentials</th><th scope="col">Actions</th></tr></thead>
          <tbody>{list.data.items.map(item => { const postgres = postgresOf(item); const iceberg = icebergOf(item); const editable = !!postgres || !!editableIcebergOf(item); return <tr key={item.id}><th scope="row">{item.name}</th><td>{postgres ? `${postgres.host}:${postgres.port} / ${postgres.database}` : item.config.driver}</td><td>{item.revision}</td><td>{item.status === 'ACTIVE' ? 'Active' : 'Paused'}</td><td>{postgres ? postgres.secretName : iceberg?.catalogSecret ?? '—'}</td><td>{canManage ? <>{editable ? <Button aria-label={`Edit connection ${item.name}`} onClick={() => setEditing(item)}>Edit</Button> : null}<Button aria-label={`Delete connection ${item.name}`} onClick={() => setDeleting(item)}>Delete</Button></> : 'Read only'}{canReadRuns && (postgres || (lakeOffered && editableIcebergOf(item))) ? <Button aria-label={`Probe connection ${item.name}`} onClick={() => setProbing(item)}>Probe / status</Button> : null}</td></tr>; })}</tbody>
        </table>
      </div>
    ) : null}
    <Dialog open={editing !== null} title={editing === 'new' ? 'New connection' : 'Edit connection'} onClose={() => setEditing(null)}>{editing !== null ? <ConnectionEditor namespace={namespace} development={development} {...(editing === 'new' ? {} : { connection: editing })} onClose={() => setEditing(null)} /> : null}</Dialog>
    <Dialog open={probing !== null} title="Connection probe" onClose={() => setProbing(null)}>{probing ? <ConnectionProbe key={`${namespace}:${probing.name}:${probing.revision}`} namespace={namespace} connection={probing} /> : null}</Dialog>
    <Dialog open={deleting !== null} title="Delete connection" onClose={() => setDeleting(null)}>{deleting ? <DeleteConnection namespace={namespace} connection={deleting} onClose={() => setDeleting(null)} /> : null}</Dialog>
  </section>;
}
