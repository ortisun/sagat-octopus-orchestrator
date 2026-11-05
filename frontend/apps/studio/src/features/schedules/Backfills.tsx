import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import type { FormEvent } from 'react';
import type { Overlap, ScheduleBackfill } from '../../api/endpoints';
import { useApp, useCan } from '../../app/context';
import { useAnnounce } from '../../ui/Announcer';
import { Button } from '../../ui/Button';
import { Dialog } from '../../ui/Dialog';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import { StatusBadge } from '../../ui/StatusBadge';
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
  return (
    <section aria-labelledby="backfills-title">
      <h2 id="backfills-title">Backfills</h2>
      <p>A backfill acts on the times of a range that has passed, for example after the schedule was created late. A time that already has a decision is never given a second run.</p>
      {operate && start ? <NewBackfill namespaceId={namespaceId} scheduleId={scheduleId} /> : null}
      {list.isPending ? <p role="status">Loading the backfills…</p> : null}
      {list.error ? <ErrorState error={list.error} what="Listing the backfills" onRetry={() => void list.refetch()} /> : null}
      {list.isSuccess && items.length === 0 ? <p>No backfill yet.</p> : null}
      {items.length > 0 ? (
        <table>
          <caption className="visually-hidden">The latest backfills of the schedule</caption>
          <thead>
            <tr>
              <th scope="col">Range</th>
              <th scope="col">State</th>
              <th scope="col">When a run is open</th>
              <th scope="col">So far</th>
              <th scope="col">
                <span className="visually-hidden">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {items.map((backfill) => (
              <tr key={backfill.backfillId}>
                <th scope="row">
                  <Time value={backfill.start} /> to <Time value={backfill.end} />
                </th>
                <td>
                  <StatusBadge view={STATE[backfill.state]} />
                </td>
                <td>{OVERLAP_TEXT[backfill.overlap].label}</td>
                <td>
                  {Object.entries(backfill.counts).length === 0
                    ? '—'
                    : Object.entries(backfill.counts)
                        .map(([outcome, count]) => `${count} ${outcomeView(outcome).label.toLowerCase()}`)
                        .join(', ')}
                </td>
                <td>{operate && open(backfill) ? <Cancel namespaceId={namespaceId} scheduleId={scheduleId} backfill={backfill} /> : null}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
    </section>
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
      <p>
        <Button onClick={() => setShown(true)}>New backfill</Button>
      </p>
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
      <p>It acts on every time of the schedule from the start up to, not including, the end, at most 1,000 of them. The end cannot be in the future.</p>
      <Field label="From" hint="Such as 2026-11-01T00:00:00Z.">
        <input type="text" autoComplete="off" value={start} disabled={command.sending} onChange={(event) => setStart(event.target.value)} />
      </Field>
      <Field label="Up to, not including" hint="Such as 2026-11-02T00:00:00Z.">
        <input type="text" autoComplete="off" value={end} disabled={command.sending} onChange={(event) => setEnd(event.target.value)} />
      </Field>
      <Field label="If a run is still open" hint={overlap === '' ? 'Each time waits for room, so none is dropped.' : OVERLAP_TEXT[overlap].help}>
        <select value={overlap} disabled={command.sending} onChange={(event) => setOverlap(event.target.value as Overlap | '')}>
          <option value="">Wait for room (recommended)</option>
          {OVERLAPS.map((name) => (
            <option key={name} value={name}>
              {OVERLAP_TEXT[name].label}
            </option>
          ))}
        </select>
      </Field>
      {problem ? (
        <p role="alert" className="error">
          {problem}
        </p>
      ) : null}
      {command.error ? <ErrorState error={command.error} what="Starting the backfill" /> : null}
      <div className="toolbar">
        <Button type="submit" variant="primary" disabled={command.sending}>
          {command.sending ? 'Starting…' : 'Start the backfill'}
        </Button>
        <Button onClick={onClose}>Close</Button>
      </div>
    </form>
  );
}
