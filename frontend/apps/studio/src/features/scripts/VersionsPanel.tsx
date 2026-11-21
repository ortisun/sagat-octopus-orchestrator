import { Button, Code, ColumnHeader, Empty, ErrorState, Hint, Loading, Row, Stack, StatusBadge, Table, TableBody, TableCell, TableHead, TableRow, Text, VisuallyHidden } from '../../ui';
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
    <Stack gap={3}>
      <Text>
        A version is the draft frozen and built in isolation into an artifact. It never changes. A workflow pins a version by its <strong>artifact digest</strong>: copy it from here into a Script node.
      </Text>
      <Row>
        <Button onClick={() => void query.refetch()} disabled={query.isFetching}>
          {query.isFetching ? 'Refreshing…' : 'Refresh'}
        </Button>
        {building ? <Hint>A version is building; this list refreshes by itself for a while.</Hint> : null}
      </Row>
      {query.isError ? <ErrorState error={query.error} what="Reading the versions" onRetry={() => void query.refetch()} /> : null}
      {query.isPending && !query.isError ? <Loading>Loading the versions…</Loading> : null}
      {query.isSuccess && items.length === 0 ? <Empty title="Nothing published yet." /> : null}
      {items.length > 0 ? (
        <Table label="Versions of this script, newest first">
          <TableHead>
            <TableRow>
              <ColumnHeader scope="col">Version</ColumnHeader>
              <ColumnHeader scope="col">State</ColumnHeader>
              <ColumnHeader scope="col">Artifact digest</ColumnHeader>
              <ColumnHeader scope="col">Language</ColumnHeader>
              <ColumnHeader scope="col">Published by</ColumnHeader>
              <ColumnHeader scope="col">
                <VisuallyHidden>Actions</VisuallyHidden>
              </ColumnHeader>
            </TableRow>
          </TableHead>
          <TableBody>
            {items.map((version) => (
              <VersionRow key={version.id} version={version} onOpen={onOpen} />
            ))}
          </TableBody>
        </Table>
      ) : null}
      {query.hasNextPage ? (
        <Row>
          <Button onClick={() => void query.fetchNextPage()} disabled={query.isFetchingNextPage}>
            {query.isFetchingNextPage ? 'Loading…' : 'Load older versions'}
          </Button>
        </Row>
      ) : null}
    </Stack>
  );
}

function VersionRow({ version, onOpen }: { version: ScriptVersionSummary; onOpen: (number: number) => void }) {
  return (
    <TableRow>
      <ColumnHeader scope="row">{version.number}</ColumnHeader>
      <TableCell>
        <StatusBadge view={versionStatus(version.status)} />
        {version.status === 'FAILED' && (version.failureCode || version.failure) ? (
          <div title={buildFailureHelp(version.failureCode ?? version.failure)}>
            <Text as="span" muted>
              {clipLabel(version.failureCode || version.failure || '', 120)}
            </Text>
          </div>
        ) : null}
      </TableCell>
      <TableCell>
        {version.artifactDigest ? (
          <Row>
            <span title={version.artifactDigest}>
              <Code>{shortDigest(version.artifactDigest)}</Code>
            </span>
            <CopyButton text={version.artifactDigest} label="Copy digest" what={`the digest of version ${version.number}`} />
          </Row>
        ) : (
          <Text as="span" muted>
            {version.status === 'BUILDING' ? 'not built yet' : 'none'}
          </Text>
        )}
      </TableCell>
      <TableCell>{languageInfo(version.language).label}</TableCell>
      <TableCell>
        {clipLabel(version.createdBy)}
        <div>
          <Text as="span" muted>
            <time dateTime={version.createdAt}>{formatTime(version.createdAt)}</time>
          </Text>
        </div>
      </TableCell>
      <TableCell>
        <Button onClick={() => onOpen(version.number)} aria-label={`Open version ${version.number} read-only`}>
          Open
        </Button>
      </TableCell>
    </TableRow>
  );
}
