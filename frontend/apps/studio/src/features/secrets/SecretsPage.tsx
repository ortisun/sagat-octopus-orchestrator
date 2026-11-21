import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useApp, useCan, useNamespace } from '../../app/context';
import { Button, ColumnHeader, Dialog, Empty, ErrorState, Loading, Page, PageHeader, Stack, Table, RowHeader, TableBody, TableCell, TableHead, TableRow, Text } from '../../ui';
import { secretsApi } from './api';
import type { Secret } from './api';
import { SecretForm } from './SecretForm';
import '../../styles/namespaces.css';

export function SecretsPage() {
  const { ep } = useApp(); const current = useNamespace(); const allowed = useCan('secret:read');
  const namespaces = useQuery({ queryKey: ['namespaces'], queryFn: ({ signal }) => ep.namespaces(signal) });
  const namespace = namespaces.data?.items.find(n => n.slug === current);
  if (!allowed) return <Page><PageHeader title="Secrets" /><Text>Viewing secret references requires secret:read permission.</Text></Page>;
  if (namespaces.error) return <Page><ErrorState error={namespaces.error} what="Loading namespaces" /></Page>;
  if (!namespace) return <Page><PageHeader title="Secrets" /><Text>Select a namespace to manage secrets.</Text></Page>;
  return <SecretList key={namespace.id} namespace={namespace.id} />;
}
function SecretList({ namespace }: { namespace: string }) {
  const { api } = useApp(); const canManage = useCan('secret:manage');
  const [editing, setEditing] = useState<Secret | 'new' | null>(null);
  const list = useQuery({ queryKey: ['secrets', namespace], queryFn: ({ signal }) => secretsApi(api, namespace).list(signal) });
  return (
    <Page>
      <Stack as="section" gap={4} aria-labelledby="secrets-title">
        <PageHeader title={<span id="secrets-title">Secrets</span>} actions={canManage ? <Button variant="primary" onClick={() => setEditing('new')}>New secret</Button> : null}>
          Create and rotate credentials in OpenBao. Workflows and connections use the name; stored values are never displayed.
        </PageHeader>
        {list.isPending ? <Loading>Loading secrets…</Loading> : null}
        {list.error ? <ErrorState error={list.error} what="Loading secrets" onRetry={() => { void list.refetch(); }} /> : null}
        {list.data?.items.length === 0 ? <Empty title="No secrets available in this namespace." headingLevel={2} /> : null}
        {list.data && list.data.items.length > 0 ? (
          // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- keyboard scrolling for a wide table
          <div className="namespaces-table" role="region" aria-label="Secret references" tabIndex={0}>
            <Table label="Secret metadata of this namespace">
              <TableHead><TableRow><ColumnHeader>Name</ColumnHeader><ColumnHeader>Version</ColumnHeader><ColumnHeader>Storage</ColumnHeader><ColumnHeader>Actions</ColumnHeader></TableRow></TableHead>
              <TableBody>{list.data.items.map(secret => (
                <TableRow key={secret.name}>
                  <RowHeader>{secret.name}</RowHeader>
                  <TableCell>{secret.version}</TableCell>
                  <TableCell>{secret.provider === 'openbao' ? 'OpenBao' : 'Local (legacy)'}</TableCell>
                  <TableCell>{canManage ? <Button aria-label={`${secret.provider === 'local' ? 'Migrate' : 'Rotate'} secret ${secret.name}`} onClick={() => setEditing(secret)}>{secret.provider === 'local' ? 'Migrate' : 'Rotate'}</Button> : 'Read only'}</TableCell>
                </TableRow>
              ))}</TableBody>
            </Table>
          </div>
        ) : null}
        <Dialog open={editing !== null} title={editing === 'new' ? 'New secret' : editing?.provider === 'local' ? 'Migrate secret' : 'Rotate secret'} onClose={() => setEditing(null)}>{editing !== null ? <SecretForm namespace={namespace} {...(editing === 'new' ? {} : { secret: editing })} onClose={() => setEditing(null)} /> : null}</Dialog>
      </Stack>
    </Page>
  );
}
