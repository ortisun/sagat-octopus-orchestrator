import { path } from '../../api/client';
import type { Run } from '../../api/types';
import { runStatus } from '../../model/status';
import { Badge, Button, Code, Disclosure, Facts, Heading, Hint, Link, Row, Stack, StatusBadge, VisuallyHidden } from '../../ui';
import type { Tone } from '../../ui';
import { isTerminal } from '../../api/follow';
import { CopyButton } from './CopyButton';
import { followWords } from './liveRun';
import type { LiveFollow } from './liveRun';
import { Elapsed, Moment, NowProvider, Relative } from './Now';
import { ActionsMenu } from './Operations';
import type { Operation } from './ops';
import { clip } from './time';
import { useApp } from '../../app/context';
import { originLabel } from './queries';
import { ScheduleOrigin } from './ScheduleOrigin';
import { TraceLink } from './TraceLink';
import { shortId } from './display';

const FOLLOW_SYMBOL: Record<LiveFollow, string> = { idle: '○', connecting: '◌', live: '●', polling: '↻', reconnecting: '↯', ended: '■', unauthorized: '✕', failed: '✕' };
const FOLLOW_TONE: Record<LiveFollow, Tone> = { idle: 'idle', connecting: 'idle', live: 'ok', polling: 'waiting', reconnecting: 'waiting', ended: 'skip', unauthorized: 'bad', failed: 'bad' };

/** "Live", "Polling", "Reconnecting" or "Ended" as a word with a shape, never only a colour. */
export function FollowIndicator({ state }: { state: LiveFollow }) {
  const words = followWords(state);
  return (
    <span data-testid="follow-state">
      <Badge tone={FOLLOW_TONE[state]} title={words.detail}>
        <span aria-hidden="true">{FOLLOW_SYMBOL[state]}</span> <span>{words.label}</span>
      </Badge>
    </span>
  );
}

/**
 * What the run is, who started it and where it stands, and what its operator may do. The times of
 * a running run count up on the page's clock (a display only: nothing is decided from it; whether
 * the run is over and what may be done come from the status the platform sent).
 */
export function RunHeader({
  run,
  status,
  workflowName,
  version,
  tenant,
  namespace,
  follow,
  operations,
  onChoose,
  onRefresh,
  refreshing,
}: {
  run: Run;
  status: string;
  workflowName: string | undefined;
  version: string;
  tenant: string | undefined;
  namespace: string | undefined;
  follow: LiveFollow;
  operations: readonly Operation[];
  onChoose: (operation: Operation, opener: HTMLElement | null) => void;
  onRefresh: () => void;
  refreshing: boolean;
}) {
  const { settings } = useApp();
  const view = runStatus(status);
  const finished = isTerminal(status);
  // the entry of a run started with a delay is held until then; once it ends or is stopped the run says so no more
  const held = run.startDelayedUntil !== undefined && status === 'WAITING';
  return (
    <header className="rn-header">
      <Stack gap={2}>
        <div className="rn-title">
          <Row justify="between" align="start" gap={3}>
            <Row gap={2}>
              <Heading level={1} look={2}>
                {originLabel(run.origin) ?? (workflowName ? clip(workflowName, 80) : 'Run')}
                <VisuallyHidden> run {run.runId}</VisuallyHidden>
              </Heading>
              <StatusBadge view={view} />
            </Row>
            <Row gap={2}>
              <FollowIndicator state={follow} />
              <Button onClick={onRefresh} disabled={refreshing}>
                {refreshing ? 'Refreshing…' : 'Refresh'}
              </Button>
              <ActionsMenu label="Run actions" buttonText="Run actions" operations={operations} onChoose={onChoose} />
            </Row>
          </Row>
          <Row gap={1.5}>
            <Code>{run.runId}</Code> <CopyButton text={run.runId} what="run id" />
            <Hint>{held ? 'Held by its start delay: nothing runs until the entry is released.' : view.help}</Hint>
          </Row>
        </div>
        <NowProvider intervalMs={1000} enabled={!finished}>
          <div className="rn-facts">
            <Facts
              columns="multi"
              items={[
                {
                  term: 'Workflow',
                  value: (
                    <>
                      {run.origin ? originLabel(run.origin) : <Link to={path`/workflows/${run.workflowId}`}>{workflowName ? clip(workflowName, 80) : `workflow ${shortId(run.workflowId)}`}</Link>}, {version}
                    </>
                  ),
                },
                { term: 'Started', value: <Moment at={run.createdAt} /> },
                {
                  term: finished ? 'Finished' : 'Running for',
                  value: finished ? (
                    run.finishedAt ? (
                      <>
                        <Moment at={run.finishedAt} /> (took <Elapsed from={run.createdAt} to={run.finishedAt} />)
                      </>
                    ) : (
                      'not recorded'
                    )
                  ) : (
                    <>
                      <Elapsed from={run.createdAt} /> so far
                    </>
                  ),
                },
                { term: 'Started by', value: run.startedBy ? clip(run.startedBy, 80) : 'unknown' },
                ...(run.businessId ? [{ term: 'Business id', value: clip(run.businessId, 200) }] : []),
                ...(run.schedule ? [{ term: 'Started by schedule', value: <ScheduleOrigin schedule={run.schedule} /> }] : []),
                ...(run.resetFrom
                  ? [
                      {
                        term: 'Reset from',
                        value: (
                          <>
                            <Link to={path`/runs/${run.resetFrom.runId}`}>{`run ${shortId(run.resetFrom.runId)}`}</Link>; this run keeps the first {run.resetFrom.sequence} {run.resetFrom.sequence === 1 ? 'event' : 'events'} of it
                          </>
                        ),
                      },
                    ]
                  : run.rerunOf
                    ? [{ term: 'Started again from', value: <Link to={path`/runs/${run.rerunOf}`}>{`run ${shortId(run.rerunOf)}`}</Link> }]
                    : []),
                ...(held && run.startDelayedUntil ? [{ term: 'Starts at', value: <Moment at={run.startDelayedUntil} /> }] : []),
              ]}
            />
          </div>
          <Disclosure summary="Run details">
            <Facts
              columns="multi"
              items={[
                {
                  term: 'Tenant / namespace',
                  value: (
                    <>
                      {tenant ? clip(tenant, 60) : 'unknown'} / {namespace ? clip(namespace, 60) : 'none chosen'}
                    </>
                  ),
                },
                { term: 'Correlation id', value: run.correlationId ? <Code>{clip(run.correlationId, 100)}</Code> : 'none' },
                {
                  term: 'Trace id',
                  value: run.traceId ? (
                    <>
                      <Code>{clip(run.traceId, 64)}</Code> <CopyButton text={run.traceId} what="trace id" /> <TraceLink template={settings.traceUrl} traceId={run.traceId} />
                    </>
                  ) : (
                    'no trace'
                  ),
                },
                {
                  term: 'Deadline',
                  value: run.deadline ? (
                    <>
                      <Moment at={run.deadline} /> (<Relative at={run.deadline} />)
                    </>
                  ) : (
                    'none'
                  ),
                },
                ...(run.parent
                  ? [
                      {
                        term: 'Child of',
                        value: (
                          <>
                            <Link to={path`/runs/${run.parent.runId}`}>run {shortId(run.parent.runId)}</Link>, step {clip(run.parent.node, 60)}
                          </>
                        ),
                      },
                    ]
                  : []),
              ]}
            />
          </Disclosure>
        </NowProvider>
      </Stack>
    </header>
  );
}
