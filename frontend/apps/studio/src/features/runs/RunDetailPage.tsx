import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { useParams } from 'react-router';
import { followRun, isTerminal } from '../../api/follow';
import type { RuntimeNode } from '../../api/types';
import { useApp, useCan, useNamespace, useSession } from '../../app/context';
import '../../styles/runs.css';
import { useAnnounce } from '../../ui/Announcer';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { Tabs } from '../../ui/Tabs';
import { viewPolicy } from './canvas';
import { followWords, LiveRunStore } from './liveRun';
import { JournalStore, NO_FILTER } from './journal';
import type { EntryFilter } from './journal';
import { GraphView } from './GraphView';
import { NodePanel } from './NodePanel';
import { NotAllowed } from './NotAllowed';
import { NowProvider, Relative } from './Now';
import { OperationDialog } from './OperationDialog';
import type { Decision } from './OperationDialog';
import { runOperations, stepOperations } from './ops';
import type { Operation, RunFacts } from './ops';
import { useVersions, versionLabel } from './queries';
import { RunHeader } from './RunHeader';
import { TableView } from './TableView';
import { TimelineView } from './TimelineView';
import { useCanOperate } from './useOperate';
import { formatDuration } from './time';

const REDUCED_MOTION = '(prefers-reduced-motion: reduce)';

function subscribeMotion(listener: () => void): () => void {
  const query = globalThis.matchMedia?.(REDUCED_MOTION);
  query?.addEventListener('change', listener);
  return () => query?.removeEventListener('change', listener);
}

/** D148: people who ask for less motion start on the table, and the tabs are always there to switch. */
function usePrefersReducedMotion(): boolean {
  return useSyncExternalStore(subscribeMotion, () => globalThis.matchMedia?.(REDUCED_MOTION).matches ?? false, () => false);
}

/** `/runs/:runId`: one run, its graph, its table, its timeline and what its operators may do. */
export function RunDetailPage() {
  const { runId } = useParams();
  const allowed = useCan('run:read');
  if (!allowed) return <NotAllowed what="Runs" permission="run:read" />;
  if (runId === undefined || runId === '') return <ErrorState error={new Error('no run')} what="Opening the run" />;
  // Another run is another page: nothing of the last one (stores, selection, dialogs) carries over.
  return <RunDetail key={runId} runId={runId} />;
}

type TabId = 'graph' | 'table' | 'timeline';

function RunDetail({ runId }: { runId: string }) {
  const { api, ep } = useApp();
  const queries = useQueryClient();
  const announce = useAnnounce();
  const namespace = useNamespace();
  const { info } = useSession();
  const can = useCanOperate();
  const reducedMotion = usePrefersReducedMotion();

  // The journal first: the live run hands it the events of each update.
  const [journal] = useState(
    () =>
      new JournalStore((cursor, signal) => ep.journal(runId, { limit: 200, ...(cursor ? { cursor } : {}) }, signal)),
  );
  const [live] = useState(
    () =>
      new LiveRunStore({
        loadGraph: (signal) => ep.graph(runId, signal),
        follow: ({ since, onChanges, onState }) => followRun({ client: api, runId, since, onChanges, onState }),
        onEvents: (events) => {
          journal.pushLive(events);
          // A pause or a resume is not always a change of status; the run resource says which.
          if (events.some((e) => e.eventType === 'run.paused' || e.eventType === 'run.resumed' || e.eventType === 'run.finished')) void queries.invalidateQueries({ queryKey: ['run', runId] });
        },
        onReset: () => {
          journal.reload();
          void queries.invalidateQueries({ queryKey: ['run', runId] });
        },
        announce,
      }),
  );
  useEffect(() => {
    live.start();
    return () => live.dispose();
  }, [live]);
  useEffect(() => () => journal.dispose(), [journal]);

  const snapshot = useSyncExternalStore(live.subscribe, live.getSnapshot);
  const { model } = snapshot;

  const run = useQuery({ queryKey: ['run', runId], queryFn: ({ signal }) => ep.getRun(runId, signal) });
  const workflowId = run.data?.workflowId;
  const workflow = useQuery({ queryKey: ['workflow', workflowId], enabled: workflowId !== undefined, staleTime: 60_000, queryFn: ({ signal }) => ep.getWorkflow(workflowId as string, signal) });
  const versions = useVersions(workflowId);
  const status = model?.status ?? run.data?.status;

  // The status changed: the run resource (finish time, paused) is read again.
  const lastStatus = useRef(status);
  useEffect(() => {
    if (lastStatus.current !== undefined && status !== lastStatus.current) void queries.invalidateQueries({ queryKey: ['run', runId] });
    lastStatus.current = status;
  }, [status, queries, runId]);

  const facts: RunFacts | undefined = useMemo(() => (status === undefined ? undefined : { status, paused: status === 'PAUSED' || run.data?.paused === true }), [status, run.data?.paused]);

  const [tab, setTab] = useState<TabId | undefined>(undefined);
  const [selectedId, setSelectedId] = useState<string | undefined>(undefined);
  const [filter, setFilter] = useState<EntryFilter>(NO_FILTER);
  const [decision, setDecision] = useState<Decision | undefined>(undefined);
  const [drawAnyway, setDrawAnyway] = useState(false);
  const main = useRef<HTMLDivElement>(null);

  const policy = useMemo(() => viewPolicy(model?.nodes.size ?? 0), [model?.nodes.size]);
  const currentTab: TabId = tab ?? (policy.start === 'table' || reducedMotion ? 'table' : 'graph');

  const select = useCallback(
    (id: string | undefined) => {
      setSelectedId(id);
      if (id !== undefined) announce(`Details of ${id.length > 60 ? `${id.slice(0, 60)}…` : id} are shown beside the list.`);
    },
    [announce],
  );
  const closePanel = () => {
    setSelectedId(undefined);
    main.current?.focus();
  };

  // The dialog reports its closing more than once (the button, then the native close event): only the first counts.
  const deciding = useRef(false);
  const choose = (operation: Operation, node: RuntimeNode | undefined, opener: HTMLElement | null) => {
    deciding.current = true;
    setDecision({ action: operation.action, ...(operation.scope === 'step' && node !== undefined ? { stepId: node.id } : {}), opener });
  };
  const closeDecision = (opener: HTMLElement | null) => {
    if (!deciding.current) return;
    deciding.current = false;
    setDecision(undefined);
    // The native dialog returns focus to what had it, which may be gone by now (a menu that closed).
    setTimeout(() => {
      if (opener?.isConnected === true) opener.focus();
      else main.current?.focus();
    }, 0);
  };

  const refresh = () => {
    void queries.invalidateQueries({ queryKey: ['run', runId] });
    void live.refresh();
    journal.reload();
  };

  if (run.isError && run.data === undefined) {
    return (
      <section>
        <h1>Run</h1>
        <ErrorState error={run.error} what="Loading the run" onRetry={() => void run.refetch()} />
      </section>
    );
  }
  if (run.data === undefined || facts === undefined || status === undefined) {
    if (snapshot.phase === 'error') {
      return (
        <section>
          <h1>Run</h1>
          <ErrorState error={snapshot.error} what="Loading the run" onRetry={() => void live.refresh()} />
        </section>
      );
    }
    return (
      <section aria-busy="true">
        <h1>Run</h1>
        <p role="status">Loading the run…</p>
      </section>
    );
  }

  const selectedNode = selectedId === undefined ? undefined : model?.nodes.get(selectedId);
  const runOps = runOperations(facts, can);
  const operationsFor = (node: RuntimeNode) => stepOperations(node, facts, can);
  const finished = isTerminal(status);

  // Until there is a graph: reading it, or the reason it could not be read.
  const waiting =
    snapshot.phase === 'error' ? (
      <ErrorState error={snapshot.error} what="Loading the graph" onRetry={() => void live.refresh()} />
    ) : (
      <p role="status">Loading the graph…</p>
    );
  const graphPanel =
    model === undefined ? (
      waiting
    ) : policy.start === 'table' && !drawAnyway ? (
      <div className="rn-big">
        <Banner tone="info" title={`This run has ${model.nodes.size.toLocaleString()} steps.`}>
          Drawing a graph this large would be slow, so the table is the way to read it. You can still draw it: expect it to take a while.
        </Banner>
        <Button onClick={() => setDrawAnyway(true)}>Draw the graph anyway</Button>
        <Button onClick={() => setTab('table')}>Open the table view</Button>
      </div>
    ) : (
      <GraphView model={model} order={snapshot.order} flash={snapshot.flash} policy={policy} selectedId={selectedId} onSelect={select} onShowTable={() => setTab('table')} />
    );

  const tablePanel =
    model === undefined ? (
      waiting
    ) : (
      <TableView model={model} order={snapshot.order} selectedId={selectedId} onSelect={select} operationsFor={operationsFor} onChoose={(operation, node, opener) => choose(operation, node, opener)} />
    );

  const timelinePanel = <TimelineView journal={journal} filter={filter} onFilter={setFilter} onSelect={select} selectedId={selectedId} />;

  return (
    <NowProvider intervalMs={15_000}>
      <section className="rn-run">
        <RunHeader
          run={run.data}
          status={status}
          workflowName={workflow.data?.name}
          version={versionLabel(versions.data, run.data.versionId, workflow.data?.activeVersion)}
          tenant={info?.tenant}
          namespace={namespace}
          follow={snapshot.follow}
          operations={runOps}
          onChoose={(operation, opener) => choose(operation, undefined, opener)}
          onRefresh={refresh}
          refreshing={snapshot.reloading}
        />

        {snapshot.lagMs > 0 ? (
          <Banner tone="info" title="The graph may be behind the journal.">
            When it was read, the projection of the graph was {formatDuration(snapshot.lagMs)} behind the journal. It catches up as updates arrive; the timeline is always the journal itself.
          </Banner>
        ) : null}
        {snapshot.follow === 'unauthorized' ? <Banner tone="error" title="Live updates stopped.">Your session is no longer valid. Sign in again.</Banner> : null}
        {snapshot.follow === 'failed' ? (
          snapshot.followError ? (
            <ErrorState error={snapshot.followError} what="Following the run" />
          ) : (
            <Banner tone="error" title="This run cannot be followed.">
              {followWords('failed').detail}
            </Banner>
          )
        ) : null}
        {snapshot.follow === 'polling' ? <Banner tone="info">The live stream is not available. The Studio asks the platform for changes every few seconds instead.</Banner> : null}
        {snapshot.reloadError !== undefined ? <ErrorState error={snapshot.reloadError} what="Reading the graph again" onRetry={() => void live.refresh()} /> : null}
        {run.isError ? <ErrorState error={run.error} what="Refreshing the run" onRetry={() => void run.refetch()} /> : null}
        {finished ? (
          <p className="hint">
            The run is over{run.data.finishedAt ? <> (<Relative at={run.data.finishedAt} />)</> : null}. Its graph and journal stay as they were.
          </p>
        ) : null}

        <div className={selectedId === undefined ? 'rn-body' : 'rn-body with-panel'}>
          <div className="rn-main" ref={main} tabIndex={-1}>
            <Tabs
              label="Views of the run"
              selected={currentTab}
              onSelect={(id) => setTab(id as TabId)}
              tabs={[
                { id: 'graph', label: 'Graph', panel: graphPanel },
                { id: 'table', label: 'Table', panel: tablePanel },
                { id: 'timeline', label: 'Timeline', panel: timelinePanel },
              ]}
            />
          </div>
          {selectedId !== undefined && model !== undefined ? (
            <NodePanel
              node={selectedNode}
              selectedId={selectedId}
              model={model}
              journal={journal}
              runActive={!finished}
              operations={selectedNode === undefined ? [] : operationsFor(selectedNode)}
              onChoose={(operation, opener) => choose(operation, selectedNode, opener)}
              onSelect={select}
              onClose={closePanel}
              onShowInTimeline={(id) => {
                setFilter({ ...NO_FILTER, stepId: id });
                setTab('timeline');
              }}
            />
          ) : null}
        </div>

        <OperationDialog decision={decision} run={facts} live={live} runId={runId} onClose={closeDecision} />
      </section>
    </NowProvider>
  );
}
