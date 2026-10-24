import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useApp, useCan, useNamespace } from '../../app/context';
import { Button } from '../../ui/Button';
import { Dialog } from '../../ui/Dialog';
import { ErrorState } from '../../ui/ErrorState';
import { secretsApi } from './api';
import type { Secret } from './api';
import { SecretForm } from './SecretForm';
import '../../styles/namespaces.css';
import '../../styles/connections.css';

export function SecretsPage() {
  const { ep } = useApp(); const current = useNamespace(); const allowed = useCan('secret:read');
  const namespaces = useQuery({ queryKey: ['namespaces'], queryFn: ({ signal }) => ep.namespaces(signal) });
  const namespace = namespaces.data?.items.find(n => n.slug === current);
  if (!allowed) return <section><h1>Secrets</h1><p>Viewing secret references requires secret:read permission.</p></section>;
  if (namespaces.error) return <ErrorState error={namespaces.error} what="Loading namespaces" />;
  if (!namespace) return <section><h1>Secrets</h1><p>Select a namespace to manage secrets.</p></section>;
  return <SecretList key={namespace.id} namespace={namespace.id} />;
}
function SecretList({ namespace }: { namespace: string }) {
  const { api } = useApp(); const canManage = useCan('secret:manage');
  const [editing, setEditing] = useState<Secret | 'new' | null>(null);
  const list = useQuery({ queryKey: ['secrets', namespace], queryFn: ({ signal }) => secretsApi(api, namespace).list(signal) });
  return <section aria-labelledby="secrets-title">
    <div className="namespaces-heading"><h1 id="secrets-title">Secrets</h1>{canManage ? <Button variant="primary" onClick={() => setEditing('new')}>New secret</Button> : null}</div>
    <p>Create and rotate credentials in OpenBao. Workflows and connections use the name; stored values are never displayed.</p>
    {list.isPending ? <p role="status">Loading secrets…</p> : null}
    {list.error ? <ErrorState error={list.error} what="Loading secrets" onRetry={() => { void list.refetch(); }} /> : null}
    {list.data?.items.length === 0 ? <p className="empty">No secrets available in this namespace.</p> : null}
    {list.data && list.data.items.length > 0 ? (
      // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- keyboard scrolling for a wide table
      <div className="namespaces-table" role="region" aria-label="Secret references" tabIndex={0}>
        <table className="data"><caption className="visually-hidden">Secret metadata of this namespace</caption><thead><tr><th scope="col">Name</th><th scope="col">Version</th><th scope="col">Storage</th><th scope="col">Actions</th></tr></thead><tbody>{list.data.items.map(secret => <tr key={secret.name}><th scope="row">{secret.name}</th><td>{secret.version}</td><td>{secret.provider === 'openbao' ? 'OpenBao' : 'Local (legacy)'}</td><td>{canManage ? <Button aria-label={`${secret.provider === 'local' ? 'Migrate' : 'Rotate'} secret ${secret.name}`} onClick={() => setEditing(secret)}>{secret.provider === 'local' ? 'Migrate' : 'Rotate'}</Button> : 'Read only'}</td></tr>)}</tbody></table>
      </div>
    ) : null}
    <Dialog open={editing !== null} title={editing === 'new' ? 'New secret' : editing?.provider === 'local' ? 'Migrate secret' : 'Rotate secret'} onClose={() => setEditing(null)}>{editing !== null ? <SecretForm namespace={namespace} {...(editing === 'new' ? {} : { secret: editing })} onClose={() => setEditing(null)} /> : null}</Dialog>
  </section>;
}
