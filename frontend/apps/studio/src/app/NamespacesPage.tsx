import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useApp, useCan, useNamespace } from './context';
import { CreateNamespaceDialog } from './CreateNamespaceDialog';
import { Badge, Button, ColumnHeader, Empty, ErrorState, Loading, Page, PageHeader, Stack, Table, RowHeader, TableBody, TableCell, TableHead, TableRow, useAnnounce } from '../ui';
import '../styles/namespaces.css';

const ENVIRONMENT = { dev: 'Development', staging: 'Staging', prod: 'Production' } as const;

/** Tenant namespaces have their own destination; the shell selector only changes context. */
export function NamespacesPage() {
  const { ep, namespace } = useApp();
  const current = useNamespace();
  const canCreate = useCan('namespace:create');
  const announce = useAnnounce();
  const [creating, setCreating] = useState(false);
  const list = useQuery({ queryKey: ['namespaces'], queryFn: ({ signal }) => ep.namespaces(signal) });
  return (
    <Page>
      <Stack as="section" gap={4} aria-labelledby="namespaces-title">
        <PageHeader title={<span id="namespaces-title">Namespaces</span>} actions={canCreate ? <Button variant="primary" onClick={() => setCreating(true)}>New namespace</Button> : null}>
          Workflows, runs, operations, scripts and integrations belong to a namespace. Use one to open its menu.
        </PageHeader>
        {list.isPending ? <Loading>Loading namespaces…</Loading> : null}
        {list.error ? <ErrorState error={list.error} what="Loading namespaces" onRetry={() => { void list.refetch(); }} /> : null}
        {list.isSuccess && list.data.items.length === 0 ? <Empty title={`No namespaces yet.${canCreate ? ' Create one with New namespace.' : ''}`} headingLevel={2} /> : null}
        {list.data && list.data.items.length > 0 ? (
          // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- keyboard users must be able to scroll the table (WCAG 2.1.1)
          <div className="namespaces-table" role="region" aria-label="Namespaces" tabIndex={0}>
            <Table label="Namespaces of your tenant">
              <TableHead><TableRow><ColumnHeader>Name</ColumnHeader><ColumnHeader>Environment</ColumnHeader><ColumnHeader>Selection</ColumnHeader></TableRow></TableHead>
              <TableBody>{list.data.items.map((item) => (
                <TableRow key={item.id}>
                  <RowHeader>{item.slug}</RowHeader>
                  <TableCell>{ENVIRONMENT[item.environment]}</TableCell>
                  <TableCell>{current === item.slug ? <Badge tone="ok">Current</Badge> : <Button aria-label={`Use namespace ${item.slug}`} onClick={() => { namespace.set(item.slug); announce(`Now working in ${item.slug}.`); }}>Use namespace</Button>}</TableCell>
                </TableRow>
              ))}</TableBody>
            </Table>
          </div>
        ) : null}
        {canCreate ? <CreateNamespaceDialog open={creating} onClose={() => setCreating(false)} /> : null}
      </Stack>
    </Page>
  );
}
