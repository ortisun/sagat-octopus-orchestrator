import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useApp, useCan, useNamespace } from './context';
import { CreateNamespaceDialog } from './CreateNamespaceDialog';
import { Button } from '../ui/Button';
import { ErrorState } from '../ui/ErrorState';
import { useAnnounce } from '../ui/Announcer';
import '../styles/namespaces.css';

/** Tenant namespaces have their own destination; the shell selector only changes context. */
export function NamespacesPage() {
  const { ep, namespace } = useApp();
  const current = useNamespace();
  const canCreate = useCan('namespace:create');
  const announce = useAnnounce();
  const [creating, setCreating] = useState(false);
  const list = useQuery({ queryKey: ['namespaces'], queryFn: ({ signal }) => ep.namespaces(signal) });
  return <section aria-labelledby="namespaces-title">
    <div className="namespaces-heading">
      <h1 id="namespaces-title">Namespaces</h1>
      {canCreate ? <Button variant="primary" onClick={() => setCreating(true)}>New namespace</Button> : null}
    </div>
    <p>Workflows, runs, operations, scripts and integrations belong to a namespace. Use one to open its menu.</p>
    {list.isPending ? <p role="status">Loading namespaces…</p> : null}
    {list.error ? <ErrorState error={list.error} what="Loading namespaces" onRetry={() => { void list.refetch(); }} /> : null}
    {list.isSuccess && list.data.items.length === 0 ? <p className="empty">No namespaces yet.{canCreate ? ' Create one with New namespace.' : ''}</p> : null}
    {list.data && list.data.items.length > 0 ? (
      // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- keyboard users must be able to scroll the table (WCAG 2.1.1)
      <div className="namespaces-table" role="region" aria-label="Namespaces" tabIndex={0}>
      <table className="data">
        <caption className="visually-hidden">Namespaces of your tenant</caption>
        <thead><tr><th scope="col">Name</th><th scope="col">Environment</th><th scope="col">Selection</th></tr></thead>
        <tbody>{list.data.items.map((item) => <tr key={item.id}>
          <th scope="row">{item.slug}</th>
          <td>{{ dev: 'Development', staging: 'Staging', prod: 'Production' }[item.environment]}</td>
          <td>{current === item.slug ? <span className="badge" data-tone="ok">Current</span> : <Button aria-label={`Use namespace ${item.slug}`} onClick={() => { namespace.set(item.slug); announce(`Now working in ${item.slug}.`); }}>Use namespace</Button>}</td>
        </tr>)}</tbody>
      </table>
    </div>) : null}
    {canCreate ? <CreateNamespaceDialog open={creating} onClose={() => setCreating(false)} /> : null}
  </section>;
}
