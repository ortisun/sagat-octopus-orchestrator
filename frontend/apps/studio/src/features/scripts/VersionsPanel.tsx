import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { StatusBadge } from '../../ui/StatusBadge';
import type { ScriptVersionSummary } from './api';
import { CopyButton } from './CopyButton';
import { languageInfo } from './languages';
import type { useVersionList } from './queries';
import { buildFailureHelp, versionStatus } from './status';
import { clipLabel, formatTime, shortDigest } from './text';

/** Published versions: what each was built into, and a way to read one as it was published. */
export function VersionsPanel({ versions, onOpen }: { versions: ReturnType<typeof useVersionList>; onOpen: (number: number) => void }) {
  const { query, items, building } = versions;
  return (
    <div className="versions-panel">
      <p>
        A version is the draft frozen and built in isolation into an artifact. It never changes. A workflow pins a version by its <strong>artifact digest</strong>: copy it from here into a Script node.
      </p>
      <div className="toolbar">
        <Button onClick={() => void query.refetch()} disabled={query.isFetching}>
          {query.isFetching ? 'Refreshing…' : 'Refresh'}
        </Button>
        {building ? <span className="hint">A version is building; this list refreshes by itself for a while.</span> : null}
      </div>
      {query.isError ? <ErrorState error={query.error} what="Reading the versions" onRetry={() => void query.refetch()} /> : null}
      {query.isPending && !query.isError ? <p role="status">Loading the versions…</p> : null}
      {query.isSuccess && items.length === 0 ? <p className="empty">Nothing published yet.</p> : null}
      {items.length > 0 ? (
        <table className="data">
          <caption className="visually-hidden">Versions of this script, newest first</caption>
          <thead>
            <tr>
              <th scope="col">Version</th>
              <th scope="col">State</th>
              <th scope="col">Artifact digest</th>
              <th scope="col">Language</th>
              <th scope="col">Published by</th>
              <th scope="col">
                <span className="visually-hidden">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {items.map((version) => (
              <VersionRow key={version.id} version={version} onOpen={onOpen} />
            ))}
          </tbody>
        </table>
      ) : null}
      {query.hasNextPage ? (
        <div className="toolbar">
          <Button onClick={() => void query.fetchNextPage()} disabled={query.isFetchingNextPage}>
            {query.isFetchingNextPage ? 'Loading…' : 'Load older versions'}
          </Button>
        </div>
      ) : null}
    </div>
  );
}

function VersionRow({ version, onOpen }: { version: ScriptVersionSummary; onOpen: (number: number) => void }) {
  return (
    <tr>
      <th scope="row">{version.number}</th>
      <td>
        <StatusBadge view={versionStatus(version.status)} />
        {version.status === 'FAILED' && (version.failureCode || version.failure) ? (
          <div className="hint" title={buildFailureHelp(version.failureCode ?? version.failure)}>
            {clipLabel(version.failureCode || version.failure || '', 120)}
          </div>
        ) : null}
      </td>
      <td>
        {version.artifactDigest ? (
          <>
            <code title={version.artifactDigest}>{shortDigest(version.artifactDigest)}</code>{' '}
            <CopyButton text={version.artifactDigest} label="Copy digest" what={`the digest of version ${version.number}`} />
          </>
        ) : (
          <span className="hint">{version.status === 'BUILDING' ? 'not built yet' : 'none'}</span>
        )}
      </td>
      <td>{languageInfo(version.language).label}</td>
      <td>
        {clipLabel(version.createdBy)}
        <div className="hint">
          <time dateTime={version.createdAt}>{formatTime(version.createdAt)}</time>
        </div>
      </td>
      <td>
        <Button onClick={() => onOpen(version.number)} aria-label={`Open version ${version.number} read-only`}>
          Open
        </Button>
      </td>
    </tr>
  );
}
