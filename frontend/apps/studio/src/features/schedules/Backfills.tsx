import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import type { Overlap, ScheduleBackfill } from '../../api/endpoints';
import { useApp, useCan } from '../../app/context';
import { Actions, Banner, Button, ColumnHeader, Dialog, Empty, ErrorState, Heading, Loading, Row, RowHeader, Select, Stack, StatusBadge, Table, TableBody, TableCell, TableHead, TableRow, Text, TextInput, VisuallyHidden, useAnnounce } from '../../ui';
import type { StatusView } from '../../model/status';
import { OVERLAPS, OVERLAP_TEXT, backfillOf, outcomeView } from './model';
import { Time, useCommand } from './shared';

const REFRESH_MS = 3_000;

const STATE: Record<ScheduleBackfill['state'], StatusView> = {
  PENDING: { label: 'Waiting to start', symbol: '◔', tone: 'idle', help: 'Another backfill of the schedule is at work, or this one has not begun.' },
  RUNNING: { label: 'Running', symbol: '▶', tone: 'running', help: 'It is acting on the times of its range.' },
  COMPLETED: { label: 'Completed', symbol: '✓', tone: 'ok', help: 'Every time of the range has a decision.' },
  CANCELLED: { label: 'Cancelled', symbol: '⊘', tone: 'skip', help: 'It was stopped; what it had done stays.' },
};
const open = (backfill: ScheduleBackfill) => backfill.state === 'PENDING' || backfill.state === 'RUNNING';

/** The backfills of a schedule: the times of a past range acted on after the fact. */
export function Backfills({ namespaceId, scheduleId }: { namespaceId: string; scheduleId: string }) {
  const { ep } = useApp();
  const operate = useCan('schedule:operate');
  const start = useCan('run:start');
  const list = useQuery({
    queryKey: ['schedule-backfills', namespaceId, scheduleId],
    queryFn: ({ signal }) => ep.scheduleBackfills(namespaceId, scheduleId, signal),
    // One at work changes under the page; once none is, the list stays as it is.
    refetchInterval: (query) => (query.state.data?.items.some(open) ? REFRESH_MS : false),
  });
  const items = list.data?.items ?? [];
  // A backfill makes decisions as it goes, and the page reads those once: when what the backfills say changes (one has gone on, ended or
  // begun), the decisions and the counters of the schedule are read again, so the history is not older than the backfill above it.
  const queries = useQueryClient();
  const says = items.map((backfill) => `${backfill.backfillId}:${backfill.state}:${Object.values(backfill.counts).reduce((total, count) => total + count, 0)}`).join('|');
  const before = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (!list.isSuccess) return;
    const last = before.current;
    before.current = says;
    if (last === undefined || last === says) return;
    void queries.invalidateQueries({ queryKey: ['schedule-actions', namespaceId, scheduleId] });
    void queries.invalidateQueries({ queryKey: ['schedule', namespaceId, scheduleId], exact: true });
  }, [says, list.isSuccess, queries, namespaceId, scheduleId]);
  return (
    <Stack as="section" gap={3} aria-labelledby="backfills-title">
      <Heading level={2} id="backfills-title">
        Backfills
      </Heading>
      <Text>A backfill acts on the times of a range that has passed, for example after the schedule was created late. A time that already has a decision is never given a second run.</Text>
      {operate && start ? <NewBackfill namespaceId={namespaceId} scheduleId={scheduleId} /> : null}
      {list.isPending ? <Loading>Loading the backfills…</Loading> : null}
      {list.error ? <ErrorState error={list.error} what="Listing the backfills" onRetry={() => void list.refetch()} /> : null}
      {list.isSuccess && items.length === 0 ? <Empty title="No backfill yet." /> : null}
      {items.length > 0 ? (
        <Table label="The latest backfills of the schedule">
          <TableHead>
            <TableRow>
              <ColumnHeader>Range</ColumnHeader>
              <ColumnHeader>State</ColumnHeader>
              <ColumnHeader>When a run is open</ColumnHeader>
              <ColumnHeader>So far</ColumnHeader>
              <ColumnHeader>
                <VisuallyHidden>Actions</VisuallyHidden>
              </ColumnHeader>
            </TableRow>
          </TableHead>
          <TableBody>
            {items.map((backfill) => (
              <TableRow key={backfill.backfillId}>
                <RowHeader>
                  <Time value={backfill.start} /> to <Time value={backfill.end} />
                </RowHeader>
                <TableCell>
                  <StatusBadge view={STATE[backfill.state]} />
                </TableCell>
                <TableCell>{OVERLAP_TEXT[backfill.overlap].label}</TableCell>
                <TableCell>
                  {Object.entries(backfill.counts).length === 0
                    ? '—'
                    : Object.entries(backfill.counts)
                        .map(([outcome, count]) => `${count} ${outcomeView(outcome).label.toLowerCase()}`)
                        .join(', ')}
                </TableCell>
                <TableCell>{operate && open(backfill) ? <Cancel namespaceId={namespaceId} scheduleId={scheduleId} backfill={backfill} /> : null}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      ) : null}
    </Stack>
  );
}

function Cancel({ namespaceId, scheduleId, backfill }: { namespaceId: string; scheduleId: string; backfill: ScheduleBackfill }) {
  const { ep } = useApp();
  const queries = useQueryClient();
  const announce = useAnnounce();
  const command = useCommand(`cancel:${backfill.backfillId}`);
  return (
    <>
      <Button
        disabled={command.sending}
        onClick={() =>
          command.send(
            (key, signal) => ep.cancelBackfill(namespaceId, scheduleId, backfill.backfillId, key, signal),
            async () => {
              announce('The backfill was cancelled.');
              await queries.invalidateQueries({ queryKey: ['schedule-backfills', namespaceId, scheduleId] });
            },
          )
        }
      >
        {command.sending ? 'Cancelling…' : 'Cancel'}
      </Button>
      {command.error ? <ErrorState error={command.error} what="Cancelling the backfill" /> : null}
    </>
  );
}

function NewBackfill({ namespaceId, scheduleId }: { namespaceId: string; scheduleId: string }) {
  const [shown, setShown] = useState(false);
  return (
    <>
      <Row>
        <Button onClick={() => setShown(true)}>New backfill</Button>
      </Row>
      <Dialog open={shown} title="Backfill a range" onClose={() => setShown(false)}>
        {shown ? <BackfillForm namespaceId={namespaceId} scheduleId={scheduleId} onClose={() => setShown(false)} /> : null}
      </Dialog>
    </>
  );
}

function BackfillForm({ namespaceId, scheduleId, onClose }: { namespaceId: string; scheduleId: string; onClose: () => void }) {
  const { ep } = useApp();
  const queries = useQueryClient();
  const announce = useAnnounce();
  const [start, setStart] = useState('');
  const [end, setEnd] = useState('');
  const [overlap, setOverlap] = useState<Overlap | ''>('');
  const [problem, setProblem] = useState<string>();
  const request = backfillOf(start, end, overlap);
  const command = useCommand(JSON.stringify('body' in request ? request.body : null));
  const submit = (event: FormEvent) => {
    event.preventDefault();
    // The end must not be after now, and now is read when the person confirms, not when the form opened.
    const checked = backfillOf(start, end, overlap, new Date());
    if ('problem' in checked) {
      setProblem(checked.problem);
      return;
    }
    setProblem(undefined);
    command.send(
      (key, signal) => ep.createBackfill(namespaceId, scheduleId, checked.body, key, signal),
      async () => {
        announce('The backfill was accepted.');
        await queries.invalidateQueries({ queryKey: ['schedule-backfills', namespaceId, scheduleId] });
        onClose();
      },
    );
  };
  return (
    <form onSubmit={submit} noValidate>
      <Stack gap={3}>
        <Text>It acts on every time of the schedule from the start up to, not including, the end, at most 1,000 of them. The end cannot be in the future.</Text>
        {/* The moments are typed as text and reach the model as typed: `backfillOf` reads them and sends them in UTC. */}
        <TextInput label="From" hint="Such as 2026-11-01T00:00:00Z." autoComplete="off" spellCheck={false} mono value={start} disabled={command.sending} onChange={setStart} />
        <TextInput label="Up to, not including" hint="Such as 2026-11-02T00:00:00Z." autoComplete="off" spellCheck={false} mono value={end} disabled={command.sending} onChange={setEnd} />
        <Select<Overlap | ''>
          label="If a run is still open"
          hint={overlap === '' ? 'Each time waits for room, so none is dropped.' : OVERLAP_TEXT[overlap].help}
          value={overlap}
          placeholder="Wait for room (recommended)"
          disabled={command.sending}
          onChange={setOverlap}
          options={[{ value: '', label: 'Wait for room (recommended)' }, ...OVERLAPS.map((name) => ({ value: name, label: OVERLAP_TEXT[name].label }))]}
        />
        {problem ? <Banner tone="error">{problem}</Banner> : null}
        {command.error ? <ErrorState error={command.error} what="Starting the backfill" /> : null}
        <Actions>
          <Button onClick={onClose}>Close</Button>
          <Button type="submit" variant="primary" disabled={command.sending}>
            {command.sending ? 'Starting…' : 'Start the backfill'}
          </Button>
        </Actions>
      </Stack>
    </form>
  );
}
