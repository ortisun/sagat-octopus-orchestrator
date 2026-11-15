import { useInfiniteQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Link } from 'react-router';
import { useApp } from '../../app/context';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import { StatusBadge } from '../../ui/StatusBadge';
import { OUTCOMES, outcomeView, reasonText } from './model';
import { Time } from './shared';
import { shortId } from '../runs/display';

const PAGE = 50;

/** Every time the schedule acted on, or chose not to, newest decision first, in pages. */
export function Decisions({ namespaceId, scheduleId }: { namespaceId: string; scheduleId: string }) {
  const { ep } = useApp();
  const [outcome, setOutcome] = useState('');
  const list = useInfiniteQuery({
    queryKey: ['schedule-actions', namespaceId, scheduleId, outcome],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) => ep.scheduleActions(namespaceId, scheduleId, { limit: PAGE, ...(pageParam ? { cursor: pageParam } : {}), ...(outcome ? { outcome } : {}) }, signal),
    getNextPageParam: (last) => last.nextCursor ?? undefined,
  });
  const rows = list.data?.pages.flatMap((page) => page.items) ?? [];
  return (
    <section aria-labelledby="decisions-title">
      <h2 id="decisions-title">Decisions</h2>
      <p>Every time the schedule reached has a decision, whether or not it started a run.</p>
      <div className="toolbar">
        <Field label="Outcome">
          <select value={outcome} onChange={(event) => setOutcome(event.target.value)}>
            <option value="">All outcomes</option>
            {OUTCOMES.map((name) => (
              <option key={name} value={name}>
                {outcomeView(name).label}
              </option>
            ))}
          </select>
        </Field>
      </div>
      {list.isPending ? <p role="status">Loading the decisions…</p> : null}
      {list.error ? <ErrorState error={list.error} what="Listing the decisions" onRetry={() => void list.refetch()} /> : null}
      {list.isSuccess && rows.length === 0 ? <p>{outcome ? 'No decision with this outcome.' : 'The schedule has not decided anything yet.'}</p> : null}
      {rows.length > 0 ? (
        <table>
          <caption className="visually-hidden">The decisions of the schedule, newest first</caption>
          <thead>
            <tr>
              <th scope="col">Decided</th>
              <th scope="col">Time</th>
              <th scope="col">Kind</th>
              <th scope="col">Outcome</th>
              <th scope="col">Why</th>
              <th scope="col">Run</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((action) => (
              <tr key={`${action.decidedAt}|${action.origin ?? ''}|${action.nominalTime}`}>
                <td>
                  <Time value={action.decidedAt} />
                </td>
                <td>
                  <Time value={action.nominalTime} />
                </td>
                <td>{action.kind === 'SCHEDULED' ? 'Scheduled' : action.kind === 'TRIGGERED' ? 'Run now' : 'Backfill'}</td>
                <td>
                  <StatusBadge view={outcomeView(action.outcome)} />
                  {action.summaryCount && action.summaryCount > 1 ? <span className="hint"> {action.summaryCount} times</span> : null}
                </td>
                <td>{reasonText(action.reason)}</td>
                <td>{action.runId ? <Link to={`/runs/${action.runId}`}>{shortId(action.runId)}</Link> : null}</td>
              </tr>
            ))}
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
