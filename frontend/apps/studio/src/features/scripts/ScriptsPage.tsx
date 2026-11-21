import { useInfiniteQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { useCan } from '../../app/context';
import { Actions, Banner, Button, Code, ColumnHeader, Dialog, Empty, ErrorState, Heading, Hint, Link, Loading, Page, Row, Stack, Table, TableBody, TableCell, TableHead, TableRow, Text, VisuallyHidden, useAnnounce } from '../../ui';
import '../../styles/scripts.css';
import type { ScriptSummary, ScriptsApi } from './api';
import { CreateScriptDialog } from './CreateScriptDialog';
import { languageInfo } from './languages';
import { clipLabel, formatBytes, formatTime } from './text';
import { useScriptsApi } from './useScriptsApi';

const PAGE = 50;

/** The scripts of the namespace: what exists, what has been published, and how to make or remove one. */
export function ScriptsPage() {
  const context = useScriptsApi();
  const canRead = useCan('script:read');
  return (
    <section className="scripts-page" aria-labelledby="scripts-title">
      <Page>
        <Stack gap={1}>
          <Heading level={1} id="scripts-title">
            Scripts
          </Heading>
          <Hint>Code that runs in the platform&apos;s sandbox as a step of a workflow. A script is a draft you edit and try; publishing it builds an immutable version whose digest a workflow pins.</Hint>
        </Stack>
      {!canRead ? (
        <Banner tone="warn" title="Your role cannot see scripts.">
          Reading scripts needs the permission <Code>script:read</Code>.
        </Banner>
      ) : context.state === 'loading' ? (
        <Loading>Loading…</Loading>
      ) : context.state === 'failed' ? (
        <ErrorState error={context.error} what="Reading your namespaces" onRetry={context.retry} />
      ) : context.state === 'none' ? (
        <Banner tone="info">There is no namespace to hold scripts yet.</Banner>
      ) : (
        <ScriptList api={context.api} namespace={context.namespace} />
      )}
      </Page>
    </section>
  );
}

function ScriptList({ api, namespace }: { api: ScriptsApi; namespace: string }) {
  const canWrite = useCan('script:write');
  const queries = useQueryClient();
  const announce = useAnnounce();
  const [creating, setCreating] = useState(false);
  const [deleting, setDeleting] = useState<ScriptSummary | undefined>(undefined);
  const key = ['scripts', api.namespaceId, 'list'];
  const list = useInfiniteQuery({
    queryKey: key,
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) => api.list({ ...(pageParam === undefined ? {} : { after: pageParam }), limit: PAGE }, signal),
    getNextPageParam: (last) => (last.items.length >= PAGE ? last.items[last.items.length - 1]?.name : undefined),
  });
  const items = list.data?.pages.flatMap((page) => page.items) ?? [];

  return (
    <>
      <Row justify="between">
        <Hint>
          Namespace <strong>{clipLabel(namespace, 63)}</strong>
        </Hint>
        {canWrite ? (
          <Button variant="primary" onClick={() => setCreating(true)}>
            New script
          </Button>
        ) : (
          <Hint>Writing scripts needs the permission script:write.</Hint>
        )}
      </Row>

      {list.isError ? <ErrorState error={list.error} what="Reading the scripts" onRetry={() => void list.refetch()} /> : null}
      {list.isPending && !list.isError ? <Loading>Loading the scripts…</Loading> : null}
      {list.isSuccess && items.length === 0 ? (
        <Empty title={`No scripts in this namespace yet.${canWrite ? ' Make one with “New script”.' : ''}`} />
      ) : null}
      {items.length > 0 ? (
        // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- keyboard users must be able to scroll the table (WCAG 2.1.1)
        <div className="scripts-table-scroll" role="region" aria-label="Scripts table" tabIndex={0}>
          <Table label="Scripts in the namespace, by name" className="scripts-table">
            <TableHead>
              <TableRow>
                <ColumnHeader scope="col">Name</ColumnHeader>
                <ColumnHeader scope="col">Language</ColumnHeader>
                <ColumnHeader scope="col">Latest version</ColumnHeader>
                <ColumnHeader scope="col">Source</ColumnHeader>
                <ColumnHeader scope="col">Updated</ColumnHeader>
                <ColumnHeader scope="col">
                  <VisuallyHidden>Actions</VisuallyHidden>
                </ColumnHeader>
              </TableRow>
            </TableHead>
            <TableBody>
              {items.map((script) => (
                <TableRow key={script.id}>
                  <ColumnHeader scope="row">
                    <Link to={`/scripts/${encodeURIComponent(script.id)}`}>{clipLabel(script.name)}</Link>
                  </ColumnHeader>
                  <TableCell>{languageInfo(script.language).label}</TableCell>
                  <TableCell>
                    {script.latestVersion === undefined ? (
                      <Text as="span" muted>
                        not published
                      </Text>
                    ) : (
                      `Version ${script.latestVersion}`
                    )}
                  </TableCell>
                  <TableCell>{formatBytes(script.sourceBytes)}</TableCell>
                  <TableCell>
                    <time dateTime={script.updatedAt}>{formatTime(script.updatedAt)}</time>
                  </TableCell>
                  <TableCell>
                    {canWrite ? (
                      script.latestVersion === undefined ? (
                        <Button variant="danger" onClick={() => setDeleting(script)} aria-label={`Delete ${clipLabel(script.name)}`}>
                          Delete
                        </Button>
                      ) : (
                        <Button disabled aria-label={`Delete ${clipLabel(script.name)}: it has versions, which a workflow may pin`} title="A script with a published version is kept: a workflow may pin its artifact.">
                          Delete
                        </Button>
                      )
                    ) : null}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      ) : null}
      {list.hasNextPage ? (
        <Row>
          <Button onClick={() => void list.fetchNextPage()} disabled={list.isFetchingNextPage}>
            {list.isFetchingNextPage ? 'Loading…' : 'Load more scripts'}
          </Button>
        </Row>
      ) : null}

      <CreateScriptDialog open={creating} api={api} onClose={() => setCreating(false)} />
      <DeleteDialog
        script={deleting}
        api={api}
        onClose={() => setDeleting(undefined)}
        onDeleted={(name) => {
          setDeleting(undefined);
          announce(`Script ${name} deleted.`);
          void queries.invalidateQueries({ queryKey: ['scripts', api.namespaceId] });
        }}
      />
    </>
  );
}

function DeleteDialog({ script, api, onClose, onDeleted }: { script: ScriptSummary | undefined; api: ScriptsApi; onClose: () => void; onDeleted: (name: string) => void }) {
  return (
    <Dialog open={script !== undefined} title="Delete script" onClose={onClose}>
      {script ? <DeleteBody script={script} api={api} onClose={onClose} onDeleted={onDeleted} /> : null}
    </Dialog>
  );
}

function DeleteBody({ script, api, onClose, onDeleted }: { script: ScriptSummary; api: ScriptsApi; onClose: () => void; onDeleted: (name: string) => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(undefined);
  const remove = () => {
    setBusy(true);
    setError(undefined);
    api.remove(script.id).then(
      () => onDeleted(script.name),
      (failure: unknown) => {
        setBusy(false);
        setError(failure);
      },
    );
  };
  return (
    <>
      <Text>
        Delete <strong>{clipLabel(script.name)}</strong> and its draft? This cannot be undone. Only a script that was never published can be deleted.
      </Text>
      {error !== undefined ? <ErrorState error={error} what="Deleting the script" /> : null}
      <Actions>
        <Button onClick={onClose}>Keep it</Button>
        <Button variant="danger" onClick={remove} disabled={busy}>
          {busy ? 'Deleting…' : 'Delete script'}
        </Button>
      </Actions>
    </>
  );
}
