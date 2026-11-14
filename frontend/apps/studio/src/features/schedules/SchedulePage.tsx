import { useQuery } from '@tanstack/react-query';
import { Link, useParams } from 'react-router';
import type { ScheduleDescription } from '../../api/endpoints';
import { useApp, useCan } from '../../app/context';
import { Banner } from '../../ui/Banner';
import { ErrorState } from '../../ui/ErrorState';
import { StatusBadge } from '../../ui/StatusBadge';
import { NotAllowed } from '../runs/NotAllowed';
import { useWorkflowIndex } from '../runs/queries';
import { Backfills } from './Backfills';
import { Decisions } from './Decisions';
import { ScheduleControls } from './ScheduleControls';
import { BUSINESS_CONFLICT_TEXT, BUSINESS_REUSE_TEXT, DEFAULT_CONFLICT, DEFAULT_REUSE, OVERLAP_TEXT, describeSpec, outcomeView, scheduleState } from './model';
import { Time, useNamespaceId } from './shared';
import { shortId } from '../runs/display';

const NEXT = 10;
const REFRESH_MS = 15_000;

/** `/schedules/:scheduleId`: what a schedule is, when it acts next, what it decided, and what a person may do to it. */
export function SchedulePage() {
  const allowed = useCan('schedule:read');
  const { scheduleId } = useParams();
  if (!allowed) return <NotAllowed what="Schedules" permission="schedule:read" />;
  // A different schedule is a different page: nothing of the last one carries over.
  return <Schedule key={scheduleId} id={scheduleId ?? ''} />;
}

function Schedule({ id }: { id: string }) {
  const { ep } = useApp();
  const namespace = useNamespaceId();
  const description = useQuery({
    queryKey: ['schedule', namespace.id, id],
    enabled: namespace.id !== undefined,
    queryFn: ({ signal }) => ep.getSchedule(namespace.id as string, id, { nextCount: NEXT, recentCount: 0 }, signal),
    // The next times move as the clock does; the platform computes them, so the page asks again.
    refetchInterval: REFRESH_MS,
  });
  if (namespace.pending || description.isPending) return <p role="status">Loading the schedule…</p>;
  if (namespace.error || description.error) return <ErrorState error={namespace.error ?? description.error} what="Loading the schedule" onRetry={() => void description.refetch()} />;
  return (
    <section aria-labelledby="schedule-title">
      <p>
        <Link to="/schedules">All schedules</Link>
      </p>
      <Summary schedule={description.data} />
      <ScheduleControls schedule={description.data} namespaceId={namespace.id as string} />
      <Upcoming schedule={description.data} />
      <Decisions namespaceId={namespace.id as string} scheduleId={id} />
      <Backfills namespaceId={namespace.id as string} scheduleId={id} />
    </section>
  );
}

function Summary({ schedule }: { schedule: ScheduleDescription }) {
  const workflows = useWorkflowIndex();
  const workflowId = schedule.action?.workflowId;
  const workflow = workflowId ? workflows.data?.byId.get(workflowId) : undefined;
  const policies = schedule.policies;
  const state = scheduleState(schedule);
  return (
    <>
      <h1 id="schedule-title" tabIndex={-1}>
        {schedule.name} <StatusBadge view={state} />
      </h1>
      {schedule.origin === 'TRIGGER' ? (
        <Banner tone="info">
          A cron trigger owns this schedule, so its rule is changed through the trigger. Pausing, resuming and running it now work once an operator has handed the cron triggers over to the schedule pass; until then the platform refuses them and says so.
        </Banner>
      ) : null}
      {schedule.exhausted ? <Banner tone="info">It has taken every action it was allowed, so it acts no more.</Banner> : null}
      {schedule.state === 'PAUSED' && schedule.note ? (
        <Banner tone="info" title="Paused:">
          {schedule.note}
          {schedule.noteBy ? ` — ${schedule.noteBy}` : ''}
        </Banner>
      ) : null}
      <dl className="batch-counts">
        <div>
          <dt>Workflow</dt>
          <dd>{workflow ? <Link to={`/workflows/${workflow.id}`}>{workflow.name}</Link> : workflowId ? shortId(workflowId) : '—'}</dd>
        </div>
        <div>
          <dt>Version</dt>
          <dd>{schedule.action?.versionPolicy === 'PINNED' ? 'Pinned to one version' : 'The active version'}</dd>
        </div>
        <div>
          <dt>When</dt>
          <dd>{describeSpec(schedule)}</dd>
        </div>
        {schedule.action?.businessIdPrefix ? (
          <>
            <div>
              <dt>Business id prefix</dt>
              <dd style={{ overflowWrap: 'anywhere' }}>
                <code>{schedule.action.businessIdPrefix}</code>, followed by a dash and the nominal time of the action
              </dd>
            </div>
            <div>
              <dt>If an open run has the id</dt>
              <dd>{BUSINESS_CONFLICT_TEXT[schedule.action.businessIdConflictPolicy ?? DEFAULT_CONFLICT].label}</dd>
            </div>
            <div>
              <dt>If a closed run remembers the id</dt>
              <dd>{BUSINESS_REUSE_TEXT[schedule.action.businessIdReusePolicy ?? DEFAULT_REUSE].label}</dd>
            </div>
          </>
        ) : null}
        <div>
          <dt>When a run is open</dt>
          <dd>{OVERLAP_TEXT[policies?.overlap ?? 'SKIP'].label}</dd>
        </div>
        <div>
          <dt>Catching up</dt>
          <dd>{policies?.catchupMode === 'ALL' ? 'Every missed time' : 'Only the latest missed time'}</dd>
        </div>
        <div>
          <dt>Pauses on failure</dt>
          <dd>{policies?.pauseOnFailure ? 'Yes' : 'No'}</dd>
        </div>
        <div>
          <dt>Actions left</dt>
          <dd>{schedule.remainingActions === null || schedule.remainingActions === undefined ? 'No limit' : schedule.remainingActions}</dd>
        </div>
        <div>
          <dt>Acts for</dt>
          <dd>{schedule.authorizedBy ?? '—'}</dd>
        </div>
        <div>
          <dt>Created</dt>
          <dd>
            <Time value={schedule.createdAt} />
          </dd>
        </div>
      </dl>
      {schedule.action?.inputDigest ? (
        <p className="hint">
          The input of its runs is not shown. Its fingerprint (SHA-256) is <code>{schedule.action.inputDigest.slice(0, 16)}…</code>
        </p>
      ) : null}
    </>
  );
}

function Upcoming({ schedule }: { schedule: ScheduleDescription }) {
  const entries = Object.entries(schedule.counters ?? {}).filter(([, count]) => count > 0);
  return (
    <>
      <h2>Next times</h2>
      {schedule.state === 'PAUSED' || schedule.exhausted ? (
        <p>None: {schedule.exhausted ? 'it has no action left' : 'it is paused'}.</p>
      ) : schedule.nextActions.length === 0 ? (
        <p>None: its rule gives no further time.</p>
      ) : (
        <ol>
          {schedule.nextActions.map((next) => (
            <li key={next.nominalTime}>
              <Time value={next.actualTime} />
              {next.actualTime !== next.nominalTime ? (
                <span className="hint">
                  {' '}
                  (nominally <Time value={next.nominalTime} />, moved by the jitter)
                </span>
              ) : null}
            </li>
          ))}
        </ol>
      )}
      <h2>Now</h2>
      <p>
        {schedule.openRuns.length === 0 ? 'No run it started is open.' : `${schedule.openRuns.length} run${schedule.openRuns.length === 1 ? '' : 's'} it started ${schedule.openRuns.length === 1 ? 'is' : 'are'} open: `}
        {schedule.openRuns.map((runId, index) => (
          <span key={runId}>
            {index > 0 ? ', ' : ''}
            <Link to={`/runs/${runId}`}>{shortId(runId)}</Link>
          </span>
        ))}
        {schedule.bufferedCount > 0 ? ` ${schedule.bufferedCount} time${schedule.bufferedCount === 1 ? ' is' : 's are'} waiting for a run to end.` : ''}
      </p>
      {entries.length > 0 ? (
        <dl className="batch-counts">
          {entries.map(([outcome, count]) => (
            <div key={outcome}>
              <dt>{outcomeView(outcome).label}</dt>
              <dd>{count}</dd>
            </div>
          ))}
        </dl>
      ) : null}
    </>
  );
}
