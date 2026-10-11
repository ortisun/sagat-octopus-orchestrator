import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { Link } from 'react-router';
import { path } from '../../api/client';
import { isTerminal } from '../../api/follow';
import type { Run } from '../../api/types';
import { useApp, useCan, useNamespace } from '../../app/context';
import { runStatus } from '../../model/status';
import '../../styles/runs.css';
import { useAnnounce } from '../../ui/Announcer';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import { StatusBadge } from '../../ui/StatusBadge';
import { RUN_STATUSES } from './display';
import { Elapsed, Moment, NowProvider } from './Now';
import { NotAllowed } from './NotAllowed';
import { useVersions, useWorkflowIndex, versionLabel } from './queries';
import type { WorkflowIndex } from './queries';
import { RunListStore } from './runList';
import { ScrollRegion } from './ScrollRegion';
import { clip } from './time';
import { useWindowedRows } from './useWindow';

const PAGE_SIZE = 50;
const ROW_ESTIMATE = 56;
const COLUMNS = 6;

export interface RunsPageProps {
  /** The compact list shown beside `/runs/:id` on a wide viewport. */
  variant?: 'page' | 'master';
  selectedRunId?: string;
}

/** `/runs`: the runs of the namespace, newest first, kept up to date while the tab is open. */
export function RunsPage({ variant = 'page', selectedRunId }: RunsPageProps = {}) {
  const allowed = useCan('run:read');
  const namespace = useNamespace();
  const [status, setStatus] = useState('');
  const [workflowId, setWorkflowId] = useState('');
  const workflows = useWorkflowIndex();
  if (!allowed) return <NotAllowed what="Runs" permission="run:read" />;
  return (
    <section className={variant === 'master' ? 'rn-runs-page rn-runs-master-page' : 'rn-runs-page'}>
      {variant === 'master' ? <h2>Runs</h2> : <h1>Runs</h1>}
      <div className="toolbar rn-filters">
        <Field label="Status">
          <select value={status} onChange={(event) => setStatus(event.target.value)}>
            <option value="">All statuses</option>
            {RUN_STATUSES.map((value) => (
              <option key={value} value={value}>
                {runStatus(value).label}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Workflow" hint={workflows.isError ? 'The workflows could not be listed; runs show their ids.' : workflows.data?.truncated ? 'Only the first workflows are listed.' : undefined}>
          <select value={workflowId} onChange={(event) => setWorkflowId(event.target.value)} disabled={workflows.data === undefined}>
            <option value="">All workflows</option>
            {(workflows.data?.list ?? []).map((workflow) => (
              <option key={workflow.id} value={workflow.id}>
                {clip(workflow.name, 80)}
              </option>
            ))}
          </select>
        </Field>
      </div>
      {namespace === undefined ? (
        <p role="status">Waiting for a namespace…</p>
      ) : (
        // A different namespace or filter is a different list: nothing of the last one carries over.
        <RunList
          key={`${namespace}|${status}|${workflowId}`}
          namespace={namespace}
          status={status}
          workflowId={workflowId}
          workflows={workflows.data}
          compact={variant === 'master'}
          selectedRunId={selectedRunId}
        />
      )}
    </section>
  );
}

function RunList({
  namespace,
  status,
  workflowId,
  workflows,
  compact,
  selectedRunId,
}: {
  namespace: string;
  status: string;
  workflowId: string;
  workflows: WorkflowIndex | undefined;
  compact: boolean;
  selectedRunId: string | undefined;
}) {
  const { ep } = useApp();
  const announce = useAnnounce();
  const [store] = useState(
    () =>
      new RunListStore({
        fetchPage: (cursor, signal) => ep.listRuns({ limit: PAGE_SIZE, ...(cursor ? { cursor } : {}), ...(status ? { status } : {}), ...(workflowId ? { workflowId } : {}) }, signal),
        onLoaded: ({ kind, count, total, hasMore }) => {
          if (kind === 'more') announce(`${count} more runs loaded, ${total} in all${hasMore ? '' : '; that is all of them'}.`);
          else announce(`${total} runs${hasMore ? ', more can be loaded' : ''}.`);
        },
      }),
  );
  useEffect(() => {
    store.start();
    return () => store.dispose();
  }, [store]);
  const snapshot = useSyncExternalStore(store.subscribe, store.getSnapshot);
  const scroller = useRef<HTMLDivElement>(null);
  const { items: windowItems, padTop, padBottom, measure } = useWindowedRows({ count: snapshot.items.length, estimate: ROW_ESTIMATE, scroller });
  const running = useMemo(() => snapshot.items.some((run) => !isTerminal(run.status)), [snapshot.items]);
  const filtered = status !== '' || workflowId !== '';

  return (
    <NowProvider intervalMs={1000} enabled={running}>
      <div className="toolbar">
        <Button onClick={() => void store.refresh()} disabled={snapshot.refreshing || snapshot.phase === 'loading'}>
          {snapshot.refreshing ? 'Refreshing…' : 'Refresh'}
        </Button>
        <span className="hint">
          {snapshot.updatedAt === undefined ? 'Reading the runs…' : `Updated ${new Date(snapshot.updatedAt).toLocaleTimeString()}. The newest runs refresh every 5 seconds while this tab is visible.`}
        </span>
      </div>

      {snapshot.phase === 'error' ? <ErrorState error={snapshot.error} what="Loading the runs" onRetry={() => void store.refresh()} /> : null}
      {snapshot.refreshError !== undefined ? <ErrorState error={snapshot.refreshError} what="Refreshing the runs" /> : null}
      {snapshot.phase === 'loading' ? <p role="status">Loading the runs of {clip(namespace, 60)}…</p> : null}

      {snapshot.phase === 'ready' && snapshot.items.length === 0 ? <p className="empty">{filtered ? 'No run matches these filters.' : `No run has been started in ${clip(namespace, 60)} yet.`}</p> : null}

      {snapshot.items.length > 0 ? (
        <ScrollRegion label="Runs" scroller={scroller}>
          {compact ? (
            <ul className="rn-master-list" aria-label="Runs, newest first">
              {padTop > 0 ? <li aria-hidden="true" className="rn-pad" style={{ height: padTop }} /> : null}
              {windowItems.map((item) => {
                const itemRun = snapshot.items[item.index];
                return itemRun === undefined ? null : (
                  <RunMasterRow key={itemRun.runId} run={itemRun} index={item.index} measure={measure} workflow={workflows?.byId.get(itemRun.workflowId)} selected={itemRun.runId === selectedRunId} />
                );
              })}
              {padBottom > 0 ? <li aria-hidden="true" className="rn-pad" style={{ height: padBottom }} /> : null}
            </ul>
          ) : (
            <table className="data rn-fixed rn-runs" aria-rowcount={snapshot.items.length + 1}>
              <caption className="visually-hidden">Runs, newest first</caption>
              <thead>
                <tr aria-rowindex={1}>
                  <th scope="col">Run</th>
                  <th scope="col">Workflow</th>
                  <th scope="col">Status</th>
                  <th scope="col">Started</th>
                  <th scope="col">Finished / duration</th>
                  <th scope="col">Started by</th>
                </tr>
              </thead>
              <tbody>
                {padTop > 0 ? (
                  <tr aria-hidden="true" className="rn-pad">
                    <td colSpan={COLUMNS} style={{ height: padTop }} />
                  </tr>
                ) : null}
                {windowItems.map((item) => {
                  const itemRun = snapshot.items[item.index];
                  return itemRun === undefined ? null : <RunRow key={itemRun.runId} run={itemRun} index={item.index} measure={measure} workflows={workflows} />;
                })}
                {padBottom > 0 ? (
                  <tr aria-hidden="true" className="rn-pad">
                    <td colSpan={COLUMNS} style={{ height: padBottom }} />
                  </tr>
                ) : null}
              </tbody>
            </table>
          )}
        </ScrollRegion>
      ) : null}

      {snapshot.moreError !== undefined ? <ErrorState error={snapshot.moreError} what="Loading more runs" onRetry={() => void store.loadMore()} /> : null}
      {snapshot.hasMore ? (
        <p>
          <Button onClick={() => void store.loadMore()} disabled={snapshot.loadingMore}>
            {snapshot.loadingMore ? 'Loading…' : 'Load more'}
          </Button>
        </p>
      ) : null}
      {snapshot.capped ? <Banner tone="info">Showing the newest {snapshot.items.length.toLocaleString()} runs. Narrow the filters to see others.</Banner> : null}
      {snapshot.items.length > 0 ? <p className="hint">{snapshot.items.length.toLocaleString()} runs loaded.</p> : null}
    </NowProvider>
  );
}

function RunMasterRow({
  run,
  index,
  measure,
  workflow,
  selected,
}: {
  run: Run;
  index: number;
  measure: (element: Element | null) => void;
  workflow: WorkflowIndex['list'][number] | undefined;
  selected: boolean;
}) {
  const finished = isTerminal(run.status);
  return (
    <li ref={measure} data-index={index} data-status={run.status} className="rn-master-row">
      <Link to={path`/runs/${run.runId}`} aria-current={selected ? 'page' : undefined}>
        <span className="rn-master-name">{workflow ? clip(workflow.name, 80) : `workflow ${run.workflowId.slice(0, 8)}`}</span>
        <StatusBadge view={runStatus(run.status)} />
        <code title={run.runId}>{run.runId.slice(-8)}</code>
        <span className="rn-master-duration">
          {finished ? (run.finishedAt ? <Elapsed from={run.createdAt} to={run.finishedAt} /> : '—') : <Elapsed from={run.createdAt} />}
        </span>
        <span className="rn-master-meta">
          <Moment at={run.createdAt} /> · {run.startedBy ? clip(run.startedBy, 40) : 'unknown starter'}
        </span>
      </Link>
    </li>
  );
}

function RunRow({ run, index, measure, workflows }: { run: Run; index: number; measure: (element: Element | null) => void; workflows: WorkflowIndex | undefined }) {
  const workflow = workflows?.byId.get(run.workflowId);
  const versions = useVersions(run.workflowId);
  const finished = isTerminal(run.status);
  return (
    <tr ref={measure} data-index={index} aria-rowindex={index + 2} data-status={run.status}>
      <td>
        <Link className="rn-id" to={path`/runs/${run.runId}`}>
          {run.runId}
        </Link>
        {run.status === 'MANUAL_REVIEW' ? (
          <div>
            <span className="rn-marker">
              <span aria-hidden="true">☝</span> A person must decide
            </span>
          </div>
        ) : null}
      </td>
      <td>
        {workflow ? clip(workflow.name, 80) : `workflow ${run.workflowId.slice(0, 8)}`}
        <div className="hint">{versionLabel(versions.data, run.versionId, workflow?.activeVersion)}</div>
      </td>
      <td>
        <StatusBadge view={runStatus(run.status)} />
      </td>
      <td>
        <Moment at={run.createdAt} />
      </td>
      <td>
        {finished ? (
          run.finishedAt ? (
            <>
              <Moment at={run.finishedAt} />
              <div className="hint">
                took <Elapsed from={run.createdAt} to={run.finishedAt} />
              </div>
            </>
          ) : (
            '—'
          )
        ) : (
          <span>
            running for <Elapsed from={run.createdAt} />
          </span>
        )}
      </td>
      <td>{run.startedBy ? clip(run.startedBy, 60) : '—'}</td>
    </tr>
  );
}
