import { useInfiniteQuery } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { useApp, useCan } from '../../app/context';
import { Button, ColumnHeader, Empty, ErrorState, Link, Loading, Page, PageHeader, Row, RowHeader, Select, Stack, StatusBadge, Table, TableBody, TableCell, TableHead, TableRow, Text, TextInput } from '../../ui';
import { NotAllowed } from '../runs/NotAllowed';
import { useWorkflowIndex } from '../runs/queries';
import { OVERLAP_TEXT, describeSpec, scheduleState } from './model';
import { Time, useNamespaceId } from './shared';
import { shortId } from '../runs/display';

const PAGE = 50;
/** How long the name has to rest before it is asked for, so a word being typed is one request. */
const TYPING_MS = 300;

/** `/schedules`: the schedules of the namespace, with the state of each and the next time it acts. */
export function SchedulesPage() {
  const allowed = useCan('schedule:read');
  if (!allowed) return <NotAllowed what="Schedules" permission="schedule:read" />;
  return <Schedules />;
}

function Schedules() {
  const { ep } = useApp();
  const namespace = useNamespaceId();
  const workflows = useWorkflowIndex();
  const mayWrite = useCan('schedule:write');
  const mayStart = useCan('run:start');
  const canCreate = mayWrite && mayStart;
  const [state, setState] = useState<'' | 'ACTIVE' | 'PAUSED'>('');
  const [workflowId, setWorkflowId] = useState('');
  const [typed, setTyped] = useState('');
  const [name, setName] = useState('');
  useEffect(() => {
    const timer = setTimeout(() => setName(typed.trim()), TYPING_MS);
    return () => clearTimeout(timer);
  }, [typed]);
  const filtered = state !== '' || workflowId !== '' || name !== '';
  const list = useInfiniteQuery({
    queryKey: ['schedules', namespace.id, state, workflowId, name],
    enabled: namespace.id !== undefined,
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) =>
      ep.listSchedules(
        namespace.id as string,
        { limit: PAGE, ...(pageParam ? { cursor: pageParam } : {}), ...(state ? { state } : {}), ...(workflowId ? { workflowId } : {}), ...(name ? { name } : {}) },
        signal,
      ),
    getNextPageParam: (last) => last.nextCursor ?? undefined,
  });
  const schedules = list.data?.pages.flatMap((page) => page.items) ?? [];
  return (
    <Page>
      <Stack as="section" gap={4} aria-labelledby="schedules-title">
        <PageHeader title={<span id="schedules-title">Schedules</span>} actions={canCreate ? <Link to="/schedules/new">New schedule</Link> : null}>
          A schedule starts a run of a workflow at the times its rule gives. The platform decides each time on its own clock; this page only shows what it decided.
        </PageHeader>
        <Row gap={3} align="end">
          <Select
            label="State"
            width={200}
            value={state}
            placeholder="All states"
            onChange={(value) => setState(value)}
            options={[
              { value: '', label: 'All states' },
              { value: 'ACTIVE', label: 'Active' },
              { value: 'PAUSED', label: 'Paused' },
            ]}
          />
          <Select
            label="Workflow"
            width={240}
            value={workflowId}
            placeholder="All workflows"
            onChange={(value) => setWorkflowId(value)}
            options={[{ value: '', label: 'All workflows' }, ...(workflows.data?.list ?? []).map((workflow) => ({ value: workflow.id, label: workflow.name }))]}
          />
          <TextInput label="Name starts with" width={240} type="search" autoComplete="off" maxLength={128} value={typed} onChange={setTyped} />
        </Row>
        {namespace.pending || list.isPending ? <Loading>Loading the schedules…</Loading> : null}
        {namespace.error || list.error ? <ErrorState error={namespace.error ?? list.error} what="Listing schedules" onRetry={() => void list.refetch()} /> : null}
        {list.isSuccess && schedules.length === 0 ? <Empty title={filtered ? 'No schedule matches these filters.' : 'No schedule yet.'} headingLevel={2} /> : null}
        {schedules.length > 0 ? (
          <Table label="Schedules">
            <TableHead>
              <TableRow>
                <ColumnHeader>Name</ColumnHeader>
                <ColumnHeader>State</ColumnHeader>
                <ColumnHeader>Workflow</ColumnHeader>
                <ColumnHeader>When</ColumnHeader>
                <ColumnHeader>Next time</ColumnHeader>
                <ColumnHeader>When a run is open</ColumnHeader>
              </TableRow>
            </TableHead>
            <TableBody>
              {schedules.map((schedule) => {
                const workflowId = schedule.action?.workflowId;
                const workflow = workflowId ? workflows.data?.byId.get(workflowId) : undefined;
                const overlap = schedule.policies?.overlap ?? 'SKIP';
                return (
                  <TableRow key={schedule.scheduleId}>
                    <RowHeader>
                      <Link to={`/schedules/${schedule.scheduleId}`}>{schedule.name}</Link>
                      {schedule.origin === 'TRIGGER' ? (
                        <Text as="span" muted>
                          {' '}
                          · cron trigger
                        </Text>
                      ) : null}
                    </RowHeader>
                    <TableCell>
                      <StatusBadge view={scheduleState(schedule)} />
                    </TableCell>
                    <TableCell>{workflow ? workflow.name : workflowId ? shortId(workflowId) : '—'}</TableCell>
                    <TableCell>{describeSpec(schedule)}</TableCell>
                    <TableCell>{schedule.state === 'PAUSED' || schedule.exhausted ? '—' : <Time value={schedule.nextActionAt} />}</TableCell>
                    <TableCell>{OVERLAP_TEXT[overlap].label}</TableCell>
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
