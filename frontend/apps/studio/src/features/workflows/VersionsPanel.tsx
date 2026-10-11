import { useInfiniteQuery, useQuery } from '@tanstack/react-query';
import { useId, useState } from 'react';
import { useApp, useNamespace } from '../../app/context';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { clip } from './canvasModel';
import { DiffView } from './DiffView';
import { openDraft } from './documentIO';
import { NodesTable } from './NodesTable';

const NO_COUNTS: ReadonlyMap<string, number> = new Map();
const noop = () => undefined;

function shortDigest(digest: string): string {
  return clip(digest.replace(/^sha256:/, ''), 12);
}

function when(iso: string | undefined): string {
  if (!iso) return '—';
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? '—' : date.toLocaleString();
}

function VersionView({ workflowId, number, onClose }: { workflowId: string; number: number; onClose: () => void }) {
  const { ep } = useApp();
  const namespace = useNamespace();
  const query = useQuery({ queryKey: ['workflow-version', namespace, workflowId, number], queryFn: ({ signal }) => ep.version(workflowId, number, signal), staleTime: Infinity });
  return (
    <section className="wf-version-view" aria-label={`Version ${number}`}>
      <div className="toolbar">
        <h3>Version {number} (read only)</h3>
        <Button className="spacer" onClick={onClose}>
          Close version {number}
        </Button>
      </div>
      {query.isPending ? <p>Loading the version…</p> : null}
      {query.isError ? <ErrorState error={query.error} what="Loading the version" onRetry={() => void query.refetch()} /> : null}
      {query.data ? (
        <>
          {query.data.releaseNotes ? <p className="wf-notes-text">Release notes: {clip(query.data.releaseNotes, 2048)}</p> : <p className="hint">No release notes.</p>}
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
    </section>
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
  if (query.isPending) return <p role="status">Comparing…</p>;
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
  const fromId = useId();
  const toId = useId();

  const query = useInfiniteQuery({
    queryKey: ['workflow-versions', namespace, workflowId],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) => ep.versions(workflowId, { limit: 25, ...(pageParam ? { cursor: pageParam } : {}) }, signal),
    getNextPageParam: (last) => last.nextCursor ?? undefined,
  });
  const versions = query.data?.pages.flatMap((p) => p.items) ?? [];

  return (
    <section className="wf-versions" aria-label="Versions">
      <h2 className="wf-panel-title">Versions</h2>
      {query.isPending ? <p>Loading the versions…</p> : null}
      {query.isError ? <ErrorState error={query.error} what="Loading the versions" onRetry={() => void query.refetch()} /> : null}
      {query.isSuccess && versions.length === 0 ? <p className="empty">Nothing is published yet. Publishing the draft creates version 1.</p> : null}
      {versions.length > 0 ? (
        <table className="data wf-table">
          <caption className="visually-hidden">Published versions, newest first</caption>
          <thead>
            <tr>
              <th scope="col">Version</th>
              <th scope="col">Digest</th>
              <th scope="col">Published by</th>
              <th scope="col">Published</th>
              <th scope="col">Actions</th>
            </tr>
          </thead>
          <tbody>
            {versions.map((v) => (
              <tr key={v.versionId} aria-current={v.versionNumber === activeNumber ? 'true' : undefined}>
                <th scope="row">
                  v{v.versionNumber}
                  {v.versionNumber === activeNumber ? (
                    <span className="wf-badge" data-tone="ok">
                      <span aria-hidden="true">✓</span> Active
                    </span>
                  ) : null}
                </th>
                <td>
                  <code>{shortDigest(v.digest)}</code>
                </td>
                <td>{v.publishedBy ? clip(v.publishedBy, 60) : '—'}</td>
                <td>{when(v.publishedAt)}</td>
                <td>
                  <Button aria-label={`View version ${v.versionNumber}`} onClick={() => setViewing(v.versionNumber)}>
                    View
                  </Button>
                  <Button
                    aria-label={`Compare version ${v.versionNumber} with the draft`}
                    onClick={() => setCompare({ from: String(v.versionNumber), to: 'draft', nonce: (compare?.nonce ?? 0) + 1 })}
                  >
                    Compare with draft
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
      {query.hasNextPage ? (
        <Button onClick={() => void query.fetchNextPage()} disabled={query.isFetchingNextPage}>
          {query.isFetchingNextPage ? 'Loading…' : 'Load more versions'}
        </Button>
      ) : null}

      {versions.length > 0 ? (
        <form
          className="wf-compare"
          onSubmit={(event) => {
            event.preventDefault();
            if (from !== '') setCompare({ from, to, nonce: (compare?.nonce ?? 0) + 1 });
          }}
        >
          <label htmlFor={fromId}>Compare</label>
          <select id={fromId} value={from} onChange={(event) => setFrom(event.target.value)}>
            <option value="">(choose a version)</option>
            {versions.map((v) => (
              <option key={v.versionId} value={String(v.versionNumber)}>
                version {v.versionNumber}
              </option>
            ))}
          </select>
          <label htmlFor={toId}>with</label>
          <select id={toId} value={to} onChange={(event) => setTo(event.target.value)}>
            <option value="draft">the saved draft</option>
            {versions.map((v) => (
              <option key={v.versionId} value={String(v.versionNumber)}>
                version {v.versionNumber}
              </option>
            ))}
          </select>
          <Button type="submit" disabled={from === ''}>
            Show differences
          </Button>
        </form>
      ) : null}

      {compare ? (
        <section className="wf-comparison" aria-label="Comparison">
          {compare.to === 'draft' || compare.from === 'draft' ? (
            <p className="hint">
              The draft here is the one the platform has saved (revision {draftRevision}).{draftDirty ? ' Changes not saved yet are not in it.' : ''}
            </p>
          ) : null}
          <Comparison workflowId={workflowId} from={compare.from} to={compare.to} nonce={compare.nonce} />
        </section>
      ) : null}
      {viewing !== undefined ? <VersionView workflowId={workflowId} number={viewing} onClose={() => setViewing(undefined)} /> : null}
    </section>
  );
}
