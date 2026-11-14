import { useQuery } from '@tanstack/react-query';
import { Link } from 'react-router';
import { path } from '../../api/client';
import type { Run } from '../../api/types';
import { useApp, useCan } from '../../app/context';
import { useNamespaceId } from '../batch/queries';
import { shortId } from './display';
import { Moment } from './Now';
import { clip } from './time';

/**
 * What each kind of action is called, as the decisions of a schedule call it. A kind this Studio does not know is shown as the platform
 * wrote it, clipped: the set only grows, and what the platform says is better than "another kind of action".
 */
function kindWords(kind: string): string {
  switch (kind) {
    case 'SCHEDULED':
      return 'scheduled';
    case 'TRIGGERED':
      return 'run now';
    case 'BACKFILL':
      return 'backfill';
    default:
      return clip(kind, 40);
  }
}

/**
 * "nightly (scheduled), nominal time 4 Nov 2026, 02:00": the schedule that started the run, by which kind of action and for which
 * nominal time, the one the schedule calculated.
 *
 * Only a session that may read schedules gets the link to the schedule and the name: the schedule page refuses everyone else, so for them
 * the schedule is called by the end of its identifier, as plain text, and nothing about it is asked (`NamedSchedule`, which holds the
 * hooks that ask, is not rendered for them: not the schedule, and not the list of namespaces either). For a session that may read them
 * the link is always shown: a schedule that cannot be described any more (deleted, in a namespace this session does not see) is called
 * by the end of its identifier, and the page the link leads to says it was not found.
 */
export function ScheduleOrigin({ schedule }: { schedule: NonNullable<Run['schedule']> }) {
  const mayRead = useCan('schedule:read');
  return (
    <>
      {mayRead ? <NamedSchedule scheduleId={schedule.scheduleId} /> : `schedule ${shortId(schedule.scheduleId)}`} ({kindWords(schedule.kind)}), nominal time{' '}
      <Moment at={schedule.nominalTime} />
    </>
  );
}

/** The link to the schedule, called by its name when the platform still describes it and by the end of its identifier otherwise. */
function NamedSchedule({ scheduleId }: { scheduleId: string }) {
  const { ep } = useApp();
  const namespace = useNamespaceId();
  const described = useQuery({
    queryKey: ['schedule-name', namespace.id, scheduleId],
    enabled: namespace.id !== undefined,
    queryFn: ({ signal }) => ep.getSchedule(namespace.id as string, scheduleId, { nextCount: 0, recentCount: 0 }, signal),
    // only the name is wanted: a missing schedule is an answer, not something to ask again about
    retry: false,
    staleTime: 60_000,
  });
  const label = described.data?.name ? clip(described.data.name, 80) : `schedule ${shortId(scheduleId)}`;
  return <Link to={path`/schedules/${scheduleId}`}>{label}</Link>;
}
