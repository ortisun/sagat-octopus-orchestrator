import { useInfiniteQuery, useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { useApp, useNamespace } from '../../app/context';
import { Badge, Button, Code, ColumnHeader, Empty, ErrorState, Heading, Hint, Loading, Row, Select, Stack, Table, TableBody, TableCell, TableHead, TableRow, Text } from '../../ui';
import { Moment } from '../runs/Now';
import { clip } from './canvasModel';
import { DiffView } from './DiffView';
import { openDraft } from './documentIO';
import { NodesTable } from './NodesTable';

const NO_COUNTS: ReadonlyMap<string, number> = new Map();
const noop = () => undefined;

function shortDigest(digest: string): string {
  return clip(digest.replace(/^sha256:/, ''), 12);
}

function VersionView({ workflowId, number, onClose }: { workflowId: string; number: number; onClose: () => void }) {
  const { ep } = useApp();
  const namespace = useNamespace();
  const query = useQuery({ queryKey: ['workflow-version', namespace, workflowId, number], queryFn: ({ signal }) => ep.version(workflowId, number, signal), staleTime: Infinity });
  return (
    <Stack as="section" gap={3} aria-label={`Version ${number}`}>
      <Row justify="between">
        <Heading level={3}>Version {number} (read only)</Heading>
        <Button onClick={onClose}>Close version {number}</Button>
      </Row>
      {query.isPending ? <Text>Loading the version…</Text> : null}
      {query.isError ? <ErrorState error={query.error} what="Loading the version" onRetry={() => void query.refetch()} /> : null}
      {query.data ? (
        <>
          {query.data.releaseNotes ? <p className="wf-notes-text">Release notes: {clip(query.data.releaseNotes, 2048)}</p> : <Hint>No release notes.</Hint>}
          <NodesTable
            doc={openDraft(query.data.document).doc}
            selectedId={undefined}
            problems={NO_COUNTS}
            advice={NO_COUNTS}
            editable={false}
            inspectable={false}
            caption={`Nodes of version ${number}`}
            onEdit={noop}
            onDelete={noop}
            onSetFirst={noop}
            onConnect={noop}
          />
        </>
      ) : null}
    </Stack>
  );
}

function Comparison({ workflowId, from, to, nonce }: { workflowId: string; from: string; to: string; nonce: number }) {
  const { ep } = useApp();
  const query = useQuery({
    queryKey: ['workflow-diff', workflowId, from, to, nonce],
    queryFn: ({ signal }) => ep.diff(workflowId, from, to, signal),
    staleTime: Infinity,
    gcTime: 0,
  });
  if (query.isPending) return <Loading>Comparing…</Loading>;
  if (query.isError) return <ErrorState error={query.error} what="Comparing the versions" onRetry={() => void query.refetch()} />;
  return <DiffView diff={query.data} classified />;
}

/**
 * The published versions of the workflow (they never change), a read-only view of any one of them,
 * and a comparison of two versions or of a version with the saved draft.
 */
export function VersionsPanel({
  workflowId,
  activeNumber,
  draftDirty,
  draftRevision,
}: {
  workflowId: string;
  activeNumber: number | undefined;
  draftDirty: boolean;
  draftRevision: number;
}) {
  const { ep } = useApp();
  const namespace = useNamespace();
  const [viewing, setViewing] = useState<number | undefined>();
  const [compare, setCompare] = useState<{ from: string; to: string; nonce: number } | undefined>();
  const [from, setFrom] = useState('');
  const [to, setTo] = useState('draft');

  const query = useInfiniteQuery({
    queryKey: ['workflow-versions', namespace, workflowId],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) => ep.versions(workflowId, { limit: 25, ...(pageParam ? { cursor: pageParam } : {}) }, signal),
    getNextPageParam: (last) => last.nextCursor ?? undefined,
  });
  const versions = query.data?.pages.flatMap((p) => p.items) ?? [];
  const versionOptions = versions.map((v) => ({ value: String(v.versionNumber), label: `version ${v.versionNumber}` }));

  return (
    <Stack as="section" gap={3} aria-label="Versions">
      <Heading level={2} look={4}>
        Versions
      </Heading>
      {query.isPending ? <Text>Loading the versions…</Text> : null}
      {query.isError ? <ErrorState error={query.error} what="Loading the versions" onRetry={() => void query.refetch()} /> : null}
      {query.isSuccess && versions.length === 0 ? <Empty title="Nothing is published yet. Publishing the draft creates version 1." /> : null}
      {versions.length > 0 ? (
        <Table label="Published versions, newest first">
          <TableHead>
            <TableRow>
              <ColumnHeader>Version</ColumnHeader>
              <ColumnHeader>Digest</ColumnHeader>
              <ColumnHeader>Published by</ColumnHeader>
              <ColumnHeader>Published</ColumnHeader>
              <ColumnHeader>Actions</ColumnHeader>
            </TableRow>
          </TableHead>
          <TableBody>
            {versions.map((v) => (
              <TableRow key={v.versionId} aria-current={v.versionNumber === activeNumber ? 'true' : undefined}>
                <ColumnHeader scope="row">
                  <span className="wf-cell-id">
                    v{v.versionNumber}
                    {v.versionNumber === activeNumber ? (
                      <Badge tone="ok">
                        <span aria-hidden="true">✓</span> Active
                      </Badge>
                    ) : null}
                  </span>
                </ColumnHeader>
                <TableCell>
                  <Code>{shortDigest(v.digest)}</Code>
                </TableCell>
                <TableCell>{v.publishedBy ? clip(v.publishedBy, 60) : '—'}</TableCell>
                <TableCell>
                  <Moment at={v.publishedAt} empty="—" />
                </TableCell>
                <TableCell>
                  <div className="wf-cell-actions">
                    <Button size="sm" aria-label={`View version ${v.versionNumber}`} onClick={() => setViewing(v.versionNumber)}>
                      View
                    </Button>
                    <Button
                      size="sm"
                      aria-label={`Compare version ${v.versionNumber} with the draft`}
                      onClick={() => setCompare({ from: String(v.versionNumber), to: 'draft', nonce: (compare?.nonce ?? 0) + 1 })}
                    >
                      Compare with draft
                    </Button>
                  </div>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      ) : null}
      {query.hasNextPage ? (
        <div>
          <Button onClick={() => void query.fetchNextPage()} disabled={query.isFetchingNextPage}>
            {query.isFetchingNextPage ? 'Loading…' : 'Load more versions'}
          </Button>
        </div>
      ) : null}

      {versions.length > 0 ? (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (from !== '') setCompare({ from, to, nonce: (compare?.nonce ?? 0) + 1 });
          }}
        >
          <Row align="end">
            <Select label="Compare" width="14rem" value={from} placeholder="(choose a version)" options={versionOptions} onChange={setFrom} />
            <Select label="with" width="14rem" value={to} options={[{ value: 'draft', label: 'the saved draft' }, ...versionOptions]} onChange={setTo} />
            <Button type="submit" disabled={from === ''}>
              Show differences
            </Button>
          </Row>
        </form>
      ) : null}

      {compare ? (
        <Stack as="section" gap={3} aria-label="Comparison">
          {compare.to === 'draft' || compare.from === 'draft' ? (
            <Hint>
              The draft here is the one the platform has saved (revision {draftRevision}).{draftDirty ? ' Changes not saved yet are not in it.' : ''}
            </Hint>
          ) : null}
          <Comparison workflowId={workflowId} from={compare.from} to={compare.to} nonce={compare.nonce} />
        </Stack>
      ) : null}
      {viewing !== undefined ? <VersionView workflowId={workflowId} number={viewing} onClose={() => setViewing(undefined)} /> : null}
    </Stack>
  );
}
