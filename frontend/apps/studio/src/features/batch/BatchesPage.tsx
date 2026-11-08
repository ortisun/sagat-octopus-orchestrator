import { useInfiniteQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Link } from 'react-router';
import { useApp, useCan } from '../../app/context';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import { StatusBadge } from '../../ui/StatusBadge';
import { NotAllowed } from '../runs/NotAllowed';
import { JOB_STATES, isActive, jobState, progress } from './model';
import { notServed, useNamespaceId } from './queries';

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
    <section aria-labelledby="batches-title">
      <h1 id="batches-title">Batch operations</h1>
      <p>
        A batch job applies one action to many runs, one at a time. Start one from the <Link to="/runs">run list</Link>, with the filters you have set.
      </p>
      <div className="toolbar">
        <Field label="State">
          <select value={state} onChange={(event) => setState(event.target.value)}>
            <option value="">All states</option>
            {JOB_STATES.map((name) => (
              <option key={name} value={name}>
                {jobState(name).label}
              </option>
            ))}
          </select>
        </Field>
      </div>
      {namespace.pending || list.isPending ? <p role="status">Loading the batch jobs…</p> : null}
      {namespace.error || (list.error && !notServed(list.error)) ? <ErrorState error={namespace.error ?? list.error} what="Listing batch jobs" onRetry={() => void list.refetch()} /> : null}
      {notServed(list.error) ? <Banner tone="info">Batch operations are not enabled on this deployment.</Banner> : null}
      {list.isSuccess && jobs.length === 0 ? <p>No batch job {state ? 'in this state' : 'yet'}.</p> : null}
      {jobs.length > 0 ? (
        <table>
          <caption className="visually-hidden">Batch jobs, newest first</caption>
          <thead>
            <tr>
              <th scope="col">Job</th>
              <th scope="col">Action</th>
              <th scope="col">State</th>
              <th scope="col">Progress</th>
              <th scope="col">Started by</th>
              <th scope="col">Created</th>
            </tr>
          </thead>
          <tbody>
            {jobs.map((job) => {
              const { done, total } = progress(job);
              return (
                <tr key={job.batchId}>
                  <th scope="row">
                    <Link to={`/batches/${job.batchId}`}>{job.batchId.slice(0, 8)}</Link>
                  </th>
                  <td>{job.action}</td>
                  <td>
                    <StatusBadge view={jobState(job.state)} />
                  </td>
                  <td>
                    {done} of {total}
                  </td>
                  <td>{job.createdBy}</td>
                  <td>
                    <time dateTime={job.createdAt}>{new Date(job.createdAt).toLocaleString('en')}</time>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      ) : null}
      {list.hasNextPage ? (
        <Button onClick={() => void list.fetchNextPage()} disabled={list.isFetchingNextPage}>
          {list.isFetchingNextPage ? 'Loading…' : 'Load more'}
        </Button>
      ) : null}
    </section>
  );
}
