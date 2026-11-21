import { useInfiniteQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { useApp, useCan } from '../../app/context';
import { Banner, Button, ColumnHeader, Empty, ErrorState, Link, Loading, Page, PageHeader, Row, RowHeader, Select, Stack, StatusBadge, Table, TableBody, TableCell, TableHead, TableRow } from '../../ui';
import { NotAllowed } from '../runs/NotAllowed';
import { JOB_STATES, isActive, jobState, progress } from './model';
import { notServed, useNamespaceId } from './queries';
import { shortId } from '../runs/display';

const PAGE = 50;
const REFRESH_MS = 5_000;

/** `/batches`: the batch jobs of the namespace, newest first, kept up to date while one is at work. */
export function BatchesPage() {
  const allowed = useCan('run:read');
  if (!allowed) return <NotAllowed what="Batch operations" permission="run:read" />;
  return <Jobs />;
}

function Jobs() {
  const { ep } = useApp();
  const namespace = useNamespaceId();
  const [state, setState] = useState('');
  const list = useInfiniteQuery({
    queryKey: ['batches', namespace.id, state],
    enabled: namespace.id !== undefined,
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) => ep.listBatches(namespace.id as string, { limit: PAGE, ...(pageParam ? { cursor: pageParam } : {}), ...(state ? { state } : {}) }, signal),
    getNextPageParam: (last) => last.nextCursor ?? undefined,
    // A job at work changes under the page; once none is, the list stays as it is.
    refetchInterval: (query) => (query.state.data?.pages.some((page) => page.items.some((job) => isActive(job.state))) ? REFRESH_MS : false),
  });
  const jobs = list.data?.pages.flatMap((page) => page.items) ?? [];
  return (
    <Page>
      <Stack as="section" gap={4} aria-labelledby="batches-title">
        <PageHeader title={<span id="batches-title">Batch operations</span>}>
          A batch job applies one action to many runs, one at a time. Start one from the <Link to="/runs">run list</Link>, with the filters you have set.
        </PageHeader>
        <Row gap={3} align="end">
          <Select
            label="State"
            width={240}
            value={state}
            placeholder="All states"
            onChange={setState}
            options={[{ value: '', label: 'All states' }, ...JOB_STATES.map((name) => ({ value: name, label: jobState(name).label }))]}
          />
        </Row>
        {namespace.pending || list.isPending ? <Loading>Loading the batch jobs…</Loading> : null}
        {namespace.error || (list.error && !notServed(list.error)) ? <ErrorState error={namespace.error ?? list.error} what="Listing batch jobs" onRetry={() => void list.refetch()} /> : null}
        {notServed(list.error) ? <Banner tone="info">Batch operations are not enabled on this deployment.</Banner> : null}
        {list.isSuccess && jobs.length === 0 ? <Empty title={`No batch job ${state ? 'in this state' : 'yet'}.`} headingLevel={2} /> : null}
        {jobs.length > 0 ? (
          <Table label="Batch jobs, newest first">
            <TableHead>
              <TableRow>
                <ColumnHeader>Job</ColumnHeader>
                <ColumnHeader>Action</ColumnHeader>
                <ColumnHeader>State</ColumnHeader>
                <ColumnHeader>Progress</ColumnHeader>
                <ColumnHeader>Started by</ColumnHeader>
                <ColumnHeader>Created</ColumnHeader>
              </TableRow>
            </TableHead>
            <TableBody>
              {jobs.map((job) => {
                const { done, total } = progress(job);
                return (
                  <TableRow key={job.batchId}>
                    <RowHeader>
                      <Link to={`/batches/${job.batchId}`}>{shortId(job.batchId)}</Link>
                    </RowHeader>
                    <TableCell>{job.action}</TableCell>
                    <TableCell>
                      <StatusBadge view={jobState(job.state)} />
                    </TableCell>
                    <TableCell>
                      {done} of {total}
                    </TableCell>
                    <TableCell>{job.createdBy}</TableCell>
                    <TableCell>
                      <time dateTime={job.createdAt}>{new Date(job.createdAt).toLocaleString('en')}</time>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        ) : null}
        {list.hasNextPage ? (
          <Row>
            <Button onClick={() => void list.fetchNextPage()} disabled={list.isFetchingNextPage}>
              {list.isFetchingNextPage ? 'Loading…' : 'Load more'}
            </Button>
          </Row>
        ) : null}
      </Stack>
    </Page>
  );
}
