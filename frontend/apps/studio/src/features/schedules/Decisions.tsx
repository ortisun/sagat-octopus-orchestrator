import { useInfiniteQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { useApp } from '../../app/context';
import { Button, ColumnHeader, Empty, ErrorState, Heading, Link, Loading, Row, Select, Stack, StatusBadge, Table, TableBody, TableCell, TableHead, TableRow, Text } from '../../ui';
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
    <Stack as="section" gap={3} aria-labelledby="decisions-title">
      <Heading level={2} id="decisions-title">
        Decisions
      </Heading>
      <Text>Every time the schedule reached has a decision, whether or not it started a run.</Text>
      <Row gap={3} align="end">
        <Select
          label="Outcome"
          width={280}
          value={outcome}
          placeholder="All outcomes"
          onChange={setOutcome}
          options={[{ value: '', label: 'All outcomes' }, ...OUTCOMES.map((name) => ({ value: name, label: outcomeView(name).label }))]}
        />
      </Row>
      {list.isPending ? <Loading>Loading the decisions…</Loading> : null}
      {list.error ? <ErrorState error={list.error} what="Listing the decisions" onRetry={() => void list.refetch()} /> : null}
      {list.isSuccess && rows.length === 0 ? <Empty title={outcome ? 'No decision with this outcome.' : 'The schedule has not decided anything yet.'} /> : null}
      {rows.length > 0 ? (
        <Table label="The decisions of the schedule, newest first">
          <TableHead>
            <TableRow>
              <ColumnHeader>Decided</ColumnHeader>
              <ColumnHeader>Time</ColumnHeader>
              <ColumnHeader>Kind</ColumnHeader>
              <ColumnHeader>Outcome</ColumnHeader>
              <ColumnHeader>Why</ColumnHeader>
              <ColumnHeader>Run</ColumnHeader>
            </TableRow>
          </TableHead>
          <TableBody>
            {rows.map((action) => (
              <TableRow key={`${action.decidedAt}|${action.origin ?? ''}|${action.nominalTime}`}>
                <TableCell>
                  <Time value={action.decidedAt} />
                </TableCell>
                <TableCell>
                  <Time value={action.nominalTime} />
                </TableCell>
                <TableCell>{action.kind === 'SCHEDULED' ? 'Scheduled' : action.kind === 'TRIGGERED' ? 'Run now' : 'Backfill'}</TableCell>
                <TableCell>
                  <StatusBadge view={outcomeView(action.outcome)} />
                  {action.summaryCount && action.summaryCount > 1 ? (
                    <Text as="span" muted>
                      {' '}
                      {action.summaryCount} times
                    </Text>
                  ) : null}
                </TableCell>
                <TableCell>{reasonText(action.reason)}</TableCell>
                <TableCell>{action.runId ? <Link to={`/runs/${action.runId}`}>{shortId(action.runId)}</Link> : null}</TableCell>
              </TableRow>
            ))}
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
  );
}
