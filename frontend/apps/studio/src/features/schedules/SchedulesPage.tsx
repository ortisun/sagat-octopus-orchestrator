import { useInfiniteQuery } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { Link } from 'react-router';
import { useApp, useCan } from '../../app/context';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import { StatusBadge } from '../../ui/StatusBadge';
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
    <section aria-labelledby="schedules-title">
      <h1 id="schedules-title">Schedules</h1>
      <p>A schedule starts a run of a workflow at the times its rule gives. The platform decides each time on its own clock; this page only shows what it decided.</p>
      {canCreate ? (
        <p>
          <Link className="btn primary" to="/schedules/new">
            New schedule
          </Link>
        </p>
      ) : null}
      <div className="toolbar">
        <Field label="State">
          <select value={state} onChange={(event) => setState(event.target.value as '' | 'ACTIVE' | 'PAUSED')}>
            <option value="">All states</option>
            <option value="ACTIVE">Active</option>
            <option value="PAUSED">Paused</option>
          </select>
        </Field>
        <Field label="Workflow">
          <select value={workflowId} onChange={(event) => setWorkflowId(event.target.value)}>
            <option value="">All workflows</option>
            {(workflows.data?.list ?? []).map((workflow) => (
              <option key={workflow.id} value={workflow.id}>
                {workflow.name}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Name starts with">
          <input type="search" autoComplete="off" maxLength={128} value={typed} onChange={(event) => setTyped(event.target.value)} />
        </Field>
      </div>
      {namespace.pending || list.isPending ? <p role="status">Loading the schedules…</p> : null}
      {namespace.error || list.error ? <ErrorState error={namespace.error ?? list.error} what="Listing schedules" onRetry={() => void list.refetch()} /> : null}
      {list.isSuccess && schedules.length === 0 ? <p>{filtered ? 'No schedule matches these filters.' : 'No schedule yet.'}</p> : null}
      {schedules.length > 0 ? (
        <table>
          <caption className="visually-hidden">Schedules</caption>
          <thead>
            <tr>
              <th scope="col">Name</th>
              <th scope="col">State</th>
              <th scope="col">Workflow</th>
              <th scope="col">When</th>
              <th scope="col">Next time</th>
              <th scope="col">When a run is open</th>
            </tr>
          </thead>
          <tbody>
            {schedules.map((schedule) => {
              const workflowId = schedule.action?.workflowId;
              const workflow = workflowId ? workflows.data?.byId.get(workflowId) : undefined;
              const overlap = schedule.policies?.overlap ?? 'SKIP';
              return (
                <tr key={schedule.scheduleId}>
                  <th scope="row">
                    <Link to={`/schedules/${schedule.scheduleId}`}>{schedule.name}</Link>
                    {schedule.origin === 'TRIGGER' ? <small className="muted"> · cron trigger</small> : null}
                  </th>
                  <td>
                    <StatusBadge view={scheduleState(schedule)} />
                  </td>
                  <td>{workflow ? workflow.name : workflowId ? shortId(workflowId) : '—'}</td>
                  <td>{describeSpec(schedule)}</td>
                  <td>{schedule.state === 'PAUSED' || schedule.exhausted ? '—' : <Time value={schedule.nextActionAt} />}</td>
                  <td>{OVERLAP_TEXT[overlap].label}</td>
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
