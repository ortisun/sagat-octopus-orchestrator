import { useState } from 'react';
import { useInfiniteQuery } from '@tanstack/react-query';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import type { AlertState, OperationsApi } from './api';
import { Moment } from '../runs/Now';
import { alertKind, safeRunbookHref } from './model';

type Filter = AlertState | 'ALL';

export function AlertsPanel({ api, namespaceId, sessionKey }: { api: OperationsApi; namespaceId: string; sessionKey: string }) {
  const [filter, setFilter] = useState<Filter>('ALL');
  const list = useInfiniteQuery({
    queryKey: ['operations', sessionKey, namespaceId, 'alerts', filter],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) => api.alerts({ limit: 50, ...(filter === 'ALL' ? {} : { state: filter }), ...(pageParam ? { cursor: pageParam } : {}) }, signal),
    getNextPageParam: (last) => last.nextCursor ?? undefined,
  });
  const alerts = list.data?.pages.flatMap((page) => page.items) ?? [];

  return (
    <section className="operations-panel" aria-labelledby="operations-alerts-title">
      <div className="operations-panel-heading">
        <div>
          <h2 id="operations-alerts-title">Alerts</h2>
          <p>Conditions recorded by the platform monitor for this namespace.</p>
        </div>
        <label className="operations-filter">
          <span>Alert state</span>
          <select value={filter} onChange={(event) => setFilter(event.target.value as Filter)}>
            <option value="ALL">All</option>
            <option value="FIRING">Firing</option>
            <option value="RESOLVED">Resolved</option>
          </select>
        </label>
      </div>

      {list.isPending ? <p role="status">Loading alerts…</p> : null}
      {list.isError ? <ErrorState error={list.error} what="Loading alerts" onRetry={() => void list.refetch()} /> : null}
      {list.isSuccess && alerts.length === 0 ? <p className="empty">No {filter === 'ALL' ? '' : `${filter.toLowerCase()} `}alerts are recorded for this namespace.</p> : null}
      {alerts.length > 0 ? (
        <div className="operations-alert-list">
          {alerts.map((item) => {
            const kind = alertKind(item.kind);
            const href = safeRunbookHref(item.runbook);
            return (
              <article className="operations-alert" key={item.id}>
                <header>
                  <h3>{kind}</h3>
                  <span className="badge" data-tone={item.state === 'FIRING' ? item.severity === 'critical' ? 'bad' : 'waiting' : 'ok'}>{item.state === 'FIRING' ? 'Firing' : 'Resolved'}</span>
                  <span className="badge" data-tone={item.severity === 'critical' ? 'bad' : 'waiting'}>{item.severity}</span>
                </header>
                <p>{item.summary}</p>
                <dl className="operations-facts">
                  {item.subject ? <><dt>Subject</dt><dd><code>{item.subject}</code></dd></> : null}
                  <dt>Items</dt><dd>{item.items}</dd>
                  <dt>Oldest</dt><dd>{item.oldestSeconds} seconds</dd>
                  <dt>Last seen</dt><dd><Moment at={item.lastSeenAt} empty="Not recorded" /></dd>
                  <dt>Firings</dt><dd>{item.firedCount}</dd>
                </dl>
                <p className="operations-runbook">
                  Runbook:{' '}
                  {href ? <a href={href} target={new URL(href).origin === window.location.origin ? undefined : '_blank'} rel="noreferrer" aria-label={`Open runbook for ${kind}`}>{item.runbook}</a> : <code>{item.runbook}</code>}
                </p>
              </article>
            );
          })}
        </div>
      ) : null}
      {list.isFetchNextPageError ? <ErrorState error={list.error} what="Loading more alerts" onRetry={() => void list.fetchNextPage()} /> : null}
      {list.hasNextPage ? <Button onClick={() => void list.fetchNextPage()} disabled={list.isFetchingNextPage}>{list.isFetchingNextPage ? 'Loading…' : 'Load more alerts'}</Button> : null}
    </section>
  );
}
