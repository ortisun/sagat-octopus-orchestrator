import { useState } from 'react';
import { useInfiniteQuery } from '@tanstack/react-query';
import { Badge, Button, Code, Empty, ErrorState, Facts, Link, Loading, Panel, Select, Stack, Text } from '../../ui';
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
    <Panel
      title="Alerts"
      labelledBy="operations-alerts-title"
      hint="Conditions recorded by the platform monitor for this namespace."
      actions={
        <Select
          label="Alert state"
          value={filter}
          onChange={setFilter}
          width="10rem"
          options={[{ value: 'ALL' as const, label: 'All' }, { value: 'FIRING' as const, label: 'Firing' }, { value: 'RESOLVED' as const, label: 'Resolved' }]}
        />
      }
    >
      {list.isPending ? <Loading>Loading alerts…</Loading> : null}
      {list.isError ? <ErrorState error={list.error} what="Loading alerts" onRetry={() => void list.refetch()} /> : null}
      {list.isSuccess && alerts.length === 0 ? <Empty title={`No ${filter === 'ALL' ? '' : `${filter.toLowerCase()} `}alerts are recorded for this namespace.`} /> : null}
      {alerts.length > 0 ? (
        <Stack gap={3}>
          {alerts.map((item) => {
            const kind = alertKind(item.kind);
            const href = safeRunbookHref(item.runbook);
            return (
              <article className="operations-item" key={item.id}>
                <Panel
                  title={kind}
                  level={3}
                  actions={
                    <>
                      <Badge tone={item.state === 'FIRING' ? item.severity === 'critical' ? 'bad' : 'waiting' : 'ok'}>{item.state === 'FIRING' ? 'Firing' : 'Resolved'}</Badge>
                      <Badge tone={item.severity === 'critical' ? 'bad' : 'waiting'}>{item.severity}</Badge>
                    </>
                  }
                >
                  <Text>{item.summary}</Text>
                  <Facts
                    items={[
                      ...(item.subject ? [{ term: 'Subject', value: <Code>{item.subject}</Code> }] : []),
                      { term: 'Items', value: item.items },
                      { term: 'Oldest', value: `${item.oldestSeconds} seconds` },
                      { term: 'Last seen', value: <Moment at={item.lastSeenAt} empty="Not recorded" /> },
                      { term: 'Firings', value: item.firedCount },
                    ]}
                  />
                  <Text>
                    Runbook:{' '}
                    {/* A runbook on another site opens in a new tab; one of this deployment opens here. Anything else stays text. */}
                    {href ? <Link to={href} external={new URL(href).origin !== window.location.origin} aria-label={`Open runbook for ${kind}`}>{item.runbook}</Link> : <Code>{item.runbook}</Code>}
                  </Text>
                </Panel>
              </article>
            );
          })}
        </Stack>
      ) : null}
      {list.isFetchNextPageError ? <ErrorState error={list.error} what="Loading more alerts" onRetry={() => void list.fetchNextPage()} /> : null}
      {list.hasNextPage ? <div><Button onClick={() => void list.fetchNextPage()} disabled={list.isFetchingNextPage}>{list.isFetchingNextPage ? 'Loading…' : 'Load more alerts'}</Button></div> : null}
    </Panel>
  );
}
