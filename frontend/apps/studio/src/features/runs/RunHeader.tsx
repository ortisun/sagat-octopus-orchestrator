import type { ReactNode } from 'react';
import { Link } from 'react-router';
import { path } from '../../api/client';
import type { Run } from '../../api/types';
import { runStatus } from '../../model/status';
import { StatusBadge } from '../../ui/StatusBadge';
import { isTerminal } from '../../api/follow';
import { CopyButton } from './CopyButton';
import { followWords } from './liveRun';
import type { LiveFollow } from './liveRun';
import { Elapsed, Moment, NowProvider, Relative } from './Now';
import { ActionsMenu } from './Operations';
import type { Operation } from './ops';
import { clip } from './time';

const FOLLOW_SYMBOL: Record<LiveFollow, string> = { idle: '○', connecting: '◌', live: '●', polling: '↻', reconnecting: '↯', ended: '■', unauthorized: '✕', failed: '✕' };
const FOLLOW_TONE: Record<LiveFollow, string> = { idle: 'idle', connecting: 'idle', live: 'ok', polling: 'waiting', reconnecting: 'waiting', ended: 'skip', unauthorized: 'bad', failed: 'bad' };

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div>
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

/** "Live", "Polling", "Reconnecting" or "Ended" as a word with a shape, never only a colour. */
export function FollowIndicator({ state }: { state: LiveFollow }) {
  const words = followWords(state);
  return (
    <span className="badge" data-tone={FOLLOW_TONE[state]} title={words.detail} data-testid="follow-state">
      <span aria-hidden="true">{FOLLOW_SYMBOL[state]}</span>
      <span>{words.label}</span>
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
  const view = runStatus(status);
  const finished = isTerminal(status);
  return (
    <header className="rn-header">
      <div className="rn-title">
        <div className="rn-heading-line">
          <h1>
            {workflowName ? clip(workflowName, 80) : 'Run'}
            <span className="visually-hidden"> run {run.runId}</span>
          </h1>
          <StatusBadge view={view} />
        </div>
        <div className="rn-run-reference">
          <code title={run.runId}>{run.runId}</code> <CopyButton text={run.runId} what="run id" />
          <span className="rn-status-help">{view.help}</span>
        </div>
        <div className="toolbar">
          <FollowIndicator state={follow} />
          <button type="button" className="btn" onClick={onRefresh} disabled={refreshing}>
            {refreshing ? 'Refreshing…' : 'Refresh'}
          </button>
          <ActionsMenu label="Run actions" buttonText="Run actions" operations={operations} onChoose={onChoose} />
        </div>
      </div>
      <NowProvider intervalMs={1000} enabled={!finished}>
        <dl className="rn-facts rn-grid">
          <Fact label="Workflow">
            <Link to={path`/workflows/${run.workflowId}`}>{workflowName ? clip(workflowName, 80) : `workflow ${run.workflowId.slice(0, 8)}`}</Link>, {version}
          </Fact>
          <Fact label="Started">
            <Moment at={run.createdAt} />
          </Fact>
          <Fact label={finished ? 'Finished' : 'Running for'}>
            {finished ? (
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
            )}
          </Fact>
          <Fact label="Started by">{run.startedBy ? clip(run.startedBy, 80) : 'unknown'}</Fact>
        </dl>
        <details className="rn-run-details">
          <summary>Run details</summary>
          <dl className="rn-facts rn-grid">
            <Fact label="Tenant / namespace">
              {tenant ? clip(tenant, 60) : 'unknown'} / {namespace ? clip(namespace, 60) : 'none chosen'}
            </Fact>
            <Fact label="Correlation id">{run.correlationId ? <code>{clip(run.correlationId, 100)}</code> : 'none'}</Fact>
            <Fact label="Trace id">
              {run.traceId ? (
                <>
                  <code>{clip(run.traceId, 64)}</code> <CopyButton text={run.traceId} what="trace id" />
                </>
              ) : (
                'no trace'
              )}
            </Fact>
            <Fact label="Deadline">
              {run.deadline ? (
                <>
                  <Moment at={run.deadline} /> (<Relative at={run.deadline} />)
                </>
              ) : (
                'none'
              )}
            </Fact>
            {run.parent ? (
              <Fact label="Child of">
                <Link to={path`/runs/${run.parent.runId}`}>run {run.parent.runId.slice(0, 8)}</Link>, step {clip(run.parent.node, 60)}
              </Fact>
            ) : null}
          </dl>
        </details>
      </NowProvider>
    </header>
  );
}
