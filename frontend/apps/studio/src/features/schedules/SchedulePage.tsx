import { useQuery } from '@tanstack/react-query';
import { useParams } from 'react-router';
import type { ScheduleDescription } from '../../api/endpoints';
import { useApp, useCan } from '../../app/context';
import { Banner, Code, ErrorState, Facts, FocusHeading, Heading, Hint, Link, Loading, Page, Stack, StatusBadge, Text } from '../../ui';
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
  if (namespace.pending || description.isPending) return <Loading>Loading the schedule…</Loading>;
  if (namespace.error || description.error) return <ErrorState error={namespace.error ?? description.error} what="Loading the schedule" onRetry={() => void description.refetch()} />;
  return (
    <Page>
      <Stack as="section" gap={4} aria-labelledby="schedule-title">
        <Text>
          <Link to="/schedules">All schedules</Link>
        </Text>
        <Summary schedule={description.data} />
        <ScheduleControls schedule={description.data} namespaceId={namespace.id as string} />
        <Upcoming schedule={description.data} />
        <Decisions namespaceId={namespace.id as string} scheduleId={id} />
        <Backfills namespaceId={namespace.id as string} scheduleId={id} />
      </Stack>
    </Page>
  );
}

function Summary({ schedule }: { schedule: ScheduleDescription }) {
  const workflows = useWorkflowIndex();
  const workflowId = schedule.action?.workflowId;
  const workflow = workflowId ? workflows.data?.byId.get(workflowId) : undefined;
  const policies = schedule.policies;
  const state = scheduleState(schedule);
  const prefix = schedule.action?.businessIdPrefix;
  return (
    <>
      {/* The controls move focus here when the button that was pressed is gone (ScheduleControls). */}
      <FocusHeading level={1} id="schedule-title">
        {schedule.name} <StatusBadge view={state} />
      </FocusHeading>
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
      <Facts
        items={[
          { term: 'Workflow', value: workflow ? <Link to={`/workflows/${workflow.id}`}>{workflow.name}</Link> : workflowId ? shortId(workflowId) : '—' },
          { term: 'Version', value: schedule.action?.versionPolicy === 'PINNED' ? 'Pinned to one version' : 'The active version' },
          { term: 'When', value: describeSpec(schedule) },
          ...(prefix
            ? [
                {
                  term: 'Business id prefix',
                  value: (
                    <span style={{ overflowWrap: 'anywhere' }}>
                      <Code>{prefix}</Code>, followed by a dash and the nominal time of the action
                    </span>
                  ),
                },
                { term: 'If an open run has the id', value: BUSINESS_CONFLICT_TEXT[schedule.action?.businessIdConflictPolicy ?? DEFAULT_CONFLICT].label },
                { term: 'If a closed run remembers the id', value: BUSINESS_REUSE_TEXT[schedule.action?.businessIdReusePolicy ?? DEFAULT_REUSE].label },
              ]
            : []),
          { term: 'When a run is open', value: OVERLAP_TEXT[policies?.overlap ?? 'SKIP'].label },
          { term: 'Catching up', value: policies?.catchupMode === 'ALL' ? 'Every missed time' : 'Only the latest missed time' },
          { term: 'Pauses on failure', value: policies?.pauseOnFailure ? 'Yes' : 'No' },
          { term: 'Actions left', value: schedule.remainingActions === null || schedule.remainingActions === undefined ? 'No limit' : schedule.remainingActions },
          { term: 'Acts for', value: schedule.authorizedBy ?? '—' },
          { term: 'Created', value: <Time value={schedule.createdAt} /> },
        ]}
      />
      {schedule.action?.inputDigest ? (
        <Hint>
          The input of its runs is not shown. Its fingerprint (SHA-256) is <Code>{schedule.action.inputDigest.slice(0, 16)}…</Code>
        </Hint>
      ) : null}
    </>
  );
}

function Upcoming({ schedule }: { schedule: ScheduleDescription }) {
  const entries = Object.entries(schedule.counters ?? {}).filter(([, count]) => count > 0);
  return (
    <>
      <Stack gap={2}>
        <Heading level={2}>Next times</Heading>
        {schedule.state === 'PAUSED' || schedule.exhausted ? (
          <Text>None: {schedule.exhausted ? 'it has no action left' : 'it is paused'}.</Text>
        ) : schedule.nextActions.length === 0 ? (
          <Text>None: its rule gives no further time.</Text>
        ) : (
          <ol>
            {schedule.nextActions.map((next) => (
              <li key={next.nominalTime}>
                <Time value={next.actualTime} />
                {next.actualTime !== next.nominalTime ? (
                  <Text as="span" muted>
                    {' '}
                    (nominally <Time value={next.nominalTime} />, moved by the jitter)
                  </Text>
                ) : null}
              </li>
            ))}
          </ol>
        )}
      </Stack>
      <Stack gap={2}>
        <Heading level={2}>Now</Heading>
        <Text>
          {schedule.openRuns.length === 0 ? 'No run it started is open.' : `${schedule.openRuns.length} run${schedule.openRuns.length === 1 ? '' : 's'} it started ${schedule.openRuns.length === 1 ? 'is' : 'are'} open: `}
          {schedule.openRuns.map((runId, index) => (
            <span key={runId}>
              {index > 0 ? ', ' : ''}
              <Link to={`/runs/${runId}`}>{shortId(runId)}</Link>
            </span>
          ))}
          {schedule.bufferedCount > 0 ? ` ${schedule.bufferedCount} time${schedule.bufferedCount === 1 ? ' is' : 's are'} waiting for a run to end.` : ''}
        </Text>
        {entries.length > 0 ? <Facts items={entries.map(([outcome, count]) => ({ term: outcomeView(outcome).label, value: count }))} /> : null}
      </Stack>
    </>
  );
}
