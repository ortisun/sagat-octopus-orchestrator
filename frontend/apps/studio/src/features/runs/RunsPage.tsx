import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { Link, useLocation, useNavigate, useSearchParams } from 'react-router';
import { path } from '../../api/client';
import { isTerminal } from '../../api/follow';
import type { Run } from '../../api/types';
import { useApp, useCan, useNamespace } from '../../app/context';
import { runStatus } from '../../model/status';
import '../../styles/runs.css';
import { Badge, Banner, Button, ColumnHeader, Empty, ErrorState, Heading, Hint, Link as PageLink, Loading, Row, Select, StatusBadge, TableBody, TableCell, TableHead, TableRow, TextInput, VisuallyHidden, WindowedTable, useAnnounce, useMediaQuery } from '../../ui';
import { readBusinessPrefix } from './businessId';
import { RUN_STATUSES, shortId } from './display';
import { Elapsed, Moment, NowProvider } from './Now';
import { NotAllowed } from './NotAllowed';
import { originLabel, useVersions, useWorkflowIndex, versionLabel } from './queries';
import type { WorkflowIndex } from './queries';
import { RunListStore } from './runList';
import { BatchFromRuns } from '../batch/BatchFromRuns';
import { StartRun } from './StartRun';
import { ScrollRegion } from './ScrollRegion';
import { clip } from './time';
import { useWindowedRows } from './useWindow';

const PAGE_SIZE = 50;
/** How long the identifier being typed has to rest before the list asks for it. */
const TYPING_MS = 300;
const ROW_ESTIMATE = 56;
const COLUMNS = 6;
/** Below this width the table of six columns does not fit beside the navigation: the runs are shown as cards. */
const CARD_LIST = '(max-width: 74rem)';
const RUN_ID = /^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i;

/** What the address says of the filters: only values the page could have put there count. */
export function readFilters(params: URLSearchParams): { status: string; workflowId: string; business: string } {
  const status = params.get('status') ?? '';
  const workflowId = params.get('workflow') ?? '';
  return {
    status: (RUN_STATUSES as readonly string[]).includes(status) ? status : '',
    workflowId: RUN_ID.test(workflowId) ? workflowId : '',
    business: readBusinessPrefix(params.get('business')),
  };
}

/** Takes a run id the way people paste it (spaces around it, a path in front of it) and tells whether it is one. */
export function parseRunId(text: string): string | undefined {
  const last = text.trim().split('/').filter(Boolean).at(-1) ?? '';
  return RUN_ID.test(last) ? last.toLowerCase() : undefined;
}

/** Where a run can come from, besides a published workflow. The first entry is "no filter". */
const ORIGINS = [
  { value: '', label: 'All origins' },
  { value: 'AD_HOC_STEP', label: 'Ad hoc database step' },
  { value: 'AD_HOC_WORKFLOW', label: 'Ad hoc database workflow' },
  { value: 'CONNECTION_PROBE', label: 'Connection probe' },
];

export interface RunsPageProps {
  /** The compact list shown beside `/runs/:id` on a wide viewport. */
  variant?: 'page' | 'master';
  selectedRunId?: string;
}

/** `/runs`: the runs of the namespace, newest first, kept up to date while the tab is open. */
export function RunsPage({ variant = 'page', selectedRunId }: RunsPageProps = {}) {
  const allowed = useCan('run:read');
  const namespace = useNamespace();
  // The filters are the address's: a link carries them, the back button and a reload keep them.
  const [params, setParams] = useSearchParams();
  const { status, workflowId, business } = readFilters(params);
  const setFilter = (key: 'status' | 'workflow' | 'business', value: string) =>
    setParams((current) => {
      const next = new URLSearchParams(current);
      if (value === '') next.delete(key);
      else next.set(key, value);
      return next;
    });
  const [origin, setOrigin] = useState('');
  const workflows = useWorkflowIndex();
  // What is typed reaches the address once typing rests; an address that changes by itself (the back button, a link,
  // "Clear filters") is what the field then shows, instead of the field writing its old text over it.
  const [typed, setTyped] = useState(business);
  const written = useRef(business);
  useEffect(() => {
    if (business === written.current) return;
    written.current = business;
    setTyped(business);
  }, [business]);
  useEffect(() => {
    const wanted = readBusinessPrefix(typed.trim());
    if (wanted === business) return undefined;
    const timer = setTimeout(() => {
      written.current = wanted;
      setFilter('business', wanted);
    }, TYPING_MS);
    return () => clearTimeout(timer);
    // `setFilter` is made anew each render and only reads the current address
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [typed, business]);
  if (!allowed) return <NotAllowed what="Runs" permission="run:read" />;
  // Beside a run the filters share a narrow column, two to a line; on the page each has a width of its own.
  const filterWidth = variant === 'master' ? '100%' : '13rem';
  return (
    <section className={variant === 'master' ? 'rn-runs-page rn-runs-master-page' : 'rn-runs-page'}>
      <Heading level={variant === 'master' ? 2 : 1} look={variant === 'master' ? 3 : 1}>
        Runs
      </Heading>
      <Row className="rn-filters" gap={3} align="end">
        <Select
          label="Status"
          width={filterWidth}
          value={status}
          placeholder="All statuses"
          onChange={(value) => setFilter('status', value)}
          options={[{ value: '', label: 'All statuses' }, ...RUN_STATUSES.map((value) => ({ value, label: runStatus(value).label }))]}
        />
        <Select label="Origin" width={filterWidth} value={origin} placeholder="All origins" onChange={setOrigin} options={ORIGINS} />
        <Select
          label="Workflow"
          width={filterWidth}
          value={workflowId}
          placeholder="All workflows"
          hint={workflows.isError ? 'The workflows could not be listed; runs show their ids.' : workflows.data?.truncated ? 'Only the first workflows are listed.' : undefined}
          onChange={(value) => setFilter('workflow', value)}
          disabled={workflows.data === undefined}
          options={[{ value: '', label: 'All workflows' }, ...(workflows.data?.list ?? []).map((workflow) => ({ value: workflow.id, label: clip(workflow.name, 80) }))]}
        />
        <TextInput
          label="Business id starts with"
          width={filterWidth}
          hint="Your own identifier for a run, such as an order number."
          type="search"
          autoComplete="off"
          spellCheck={false}
          maxLength={200}
          value={typed}
          onChange={setTyped}
        />
        {status !== '' || workflowId !== '' || origin !== '' || business !== '' ? (
          <Button onClick={() => { setOrigin(''); setTyped(''); setParams((current) => { const next = new URLSearchParams(current); next.delete('status'); next.delete('workflow'); next.delete('business'); return next; }); }}>Clear filters</Button>
        ) : null}
        {variant === 'page' ? <OpenRun /> : null}
        {variant === 'page' ? <StartRun choices={(workflows.data?.list ?? []).filter((workflow) => workflow.activeVersion)} /> : null}
        {variant === 'page' ? <BatchFromRuns filters={{ status, workflowId, origin, businessIdPrefix: business }}{...(workflowId ? { workflowName: workflows.data?.list.find((workflow) => workflow.id === workflowId)?.name } : {})} /> : null}
      </Row>
      {namespace === undefined ? (
        <Loading>Waiting for a namespace…</Loading>
      ) : (
        // A different namespace or filter is a different list: nothing of the last one carries over.
        <RunList
          key={`${namespace}|${status}|${workflowId}|${origin}|${business}`}
          namespace={namespace}
          origin={origin}
          status={status}
          workflowId={workflowId}
          business={business}
          workflows={workflows.data}
          compact={variant === 'master'}
          selectedRunId={selectedRunId}
        />
      )}
    </section>
  );
}

function RunList({
  origin,
  namespace,
  status,
  workflowId,
  business,
  workflows,
  compact,
  selectedRunId,
}: {
  origin: string;
  namespace: string;
  status: string;
  workflowId: string;
  business: string;
  workflows: WorkflowIndex | undefined;
  compact: boolean;
  selectedRunId: string | undefined;
}) {
  const { ep } = useApp();
  const announce = useAnnounce();
  const [store] = useState(
    () =>
      new RunListStore({
        fetchPage: (cursor, signal) => ep.listRuns({ limit: PAGE_SIZE, ...(cursor ? { cursor } : {}), ...(status ? { status } : {}), ...(origin ? { origin } : {}), ...(workflowId ? { workflowId } : {}), ...(business ? { businessIdPrefix: business } : {}) }, signal),
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
  const filtered = status !== '' || workflowId !== '' || origin !== '' || business !== '';
  const cards = useMediaQuery(CARD_LIST);
  const asCards = compact || cards;

  return (
    <NowProvider intervalMs={1000} enabled={running}>
      <Row>
        <Button onClick={() => void store.refresh()} disabled={snapshot.refreshing || snapshot.phase === 'loading'}>
          {snapshot.refreshing ? 'Refreshing…' : 'Refresh'}
        </Button>
        <Hint>
          {snapshot.updatedAt === undefined ? 'Reading the runs…' : `Updated ${new Date(snapshot.updatedAt).toLocaleTimeString()}. The newest runs refresh every 5 seconds while this tab is visible.`}
        </Hint>
      </Row>

      {snapshot.phase === 'error' ? <ErrorState error={snapshot.error} what="Loading the runs" onRetry={() => void store.refresh()} /> : null}
      {snapshot.refreshError !== undefined ? <ErrorState error={snapshot.refreshError} what="Refreshing the runs" /> : null}
      {snapshot.phase === 'loading' ? <Loading>{`Loading the runs of ${clip(namespace, 60)}…`}</Loading> : null}

      {snapshot.phase === 'ready' && snapshot.items.length === 0 ? <Empty title={filtered ? 'No run matches these filters.' : `No run has been started in ${clip(namespace, 60)} yet.`} /> : null}

      {snapshot.items.length > 0 ? (
        <ScrollRegion label="Runs" scroller={scroller}>
          {asCards ? (
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
            <WindowedTable className="rn-fixed rn-runs" label="Runs, newest first" rowCount={snapshot.items.length + 1}>
              <TableHead>
                <TableRow aria-rowindex={1}>
                  <ColumnHeader scope="col">Run</ColumnHeader>
                  <ColumnHeader scope="col">Workflow</ColumnHeader>
                  <ColumnHeader scope="col">Status</ColumnHeader>
                  <ColumnHeader scope="col">Started</ColumnHeader>
                  <ColumnHeader scope="col">Finished / duration</ColumnHeader>
                  <ColumnHeader scope="col">Started by</ColumnHeader>
                </TableRow>
              </TableHead>
              <TableBody>
                {padTop > 0 ? (
                  <TableRow aria-hidden="true" className="rn-pad">
                    <TableCell colSpan={COLUMNS} style={{ height: padTop }} />
                  </TableRow>
                ) : null}
                {windowItems.map((item) => {
                  const itemRun = snapshot.items[item.index];
                  return itemRun === undefined ? null : <RunRow key={itemRun.runId} run={itemRun} index={item.index} measure={measure} workflows={workflows} />;
                })}
                {padBottom > 0 ? (
                  <TableRow aria-hidden="true" className="rn-pad">
                    <TableCell colSpan={COLUMNS} style={{ height: padBottom }} />
                  </TableRow>
                ) : null}
              </TableBody>
            </WindowedTable>
          )}
        </ScrollRegion>
      ) : null}

      {snapshot.moreError !== undefined ? <ErrorState error={snapshot.moreError} what="Loading more runs" onRetry={() => void store.loadMore()} /> : null}
      {snapshot.hasMore ? (
        <div>
          <Button onClick={() => void store.loadMore()} disabled={snapshot.loadingMore}>
            {snapshot.loadingMore ? 'Loading…' : 'Load more'}
          </Button>
        </div>
      ) : null}
      {snapshot.capped ? <Banner tone="info">Showing the newest {snapshot.items.length.toLocaleString()} runs. Narrow the filters to see others.</Banner> : null}
      {snapshot.items.length > 0 ? <Hint>{snapshot.items.length.toLocaleString()} runs loaded.</Hint> : null}
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
  // Opening a run keeps the filters, so the list beside it stays the list that was chosen.
  const { search } = useLocation();
  return (
    <li ref={measure} data-index={index} data-status={run.status} className="rn-master-row">
      {/* The whole card is the link, and it says which run is open (`aria-current`): the router's own link, since the interface module's link is a line of text. */}
      <Link to={{ pathname: path`/runs/${run.runId}`, search }} aria-current={selected ? 'page' : undefined}>
        <span className="rn-master-name" title={workflow?.name}>
          {originLabel(run.origin) ?? (workflow ? clip(workflow.name, 80) : `workflow ${shortId(run.workflowId)}`)}
        </span>
        <StatusBadge view={runStatus(run.status)} />
        <code title={run.runId}>{shortId(run.runId)}</code>
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
  const versions = useVersions(run.origin ? undefined : run.workflowId);
  const finished = isTerminal(run.status);
  const { search } = useLocation();
  return (
    <TableRow ref={measure} data-index={index} aria-rowindex={index + 2} data-status={run.status}>
      <TableCell>
        <PageLink className="rn-id" to={`${path`/runs/${run.runId}`}${search}`}>
          {run.runId}
        </PageLink>
        {run.businessId ? (
          <div className="rn-quiet">
            <VisuallyHidden>Business id </VisuallyHidden>
            {clip(run.businessId, 80)}
          </div>
        ) : null}
        {run.status === 'MANUAL_REVIEW' ? (
          <div>
            <Badge tone="bad">
              <span aria-hidden="true">☝</span> A person must decide
            </Badge>
          </div>
        ) : null}
        {run.startDelayedUntil && run.status === 'WAITING' ? (
          <div className="rn-quiet">
            Starts <Moment at={run.startDelayedUntil} />
          </div>
        ) : null}
      </TableCell>
      <TableCell>
        {originLabel(run.origin) ?? (workflow ? clip(workflow.name, 80) : `workflow ${shortId(run.workflowId)}`)}
        <div className="rn-quiet">{versionLabel(versions.data, run.versionId, workflow?.activeVersion)}</div>
      </TableCell>
      <TableCell>
        <StatusBadge view={runStatus(run.status)} />
      </TableCell>
      <TableCell>
        <Moment at={run.createdAt} />
      </TableCell>
      <TableCell>
        {finished ? (
          run.finishedAt ? (
            <>
              <Moment at={run.finishedAt} />
              <div className="rn-quiet">
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
      </TableCell>
      <TableCell>{run.startedBy ? clip(run.startedBy, 60) : '—'}</TableCell>
    </TableRow>
  );
}

/** Opens a run from its id, wherever the person got it (a log, a ticket, a message). */
function OpenRun() {
  const navigate = useNavigate();
  const [text, setText] = useState('');
  const [invalid, setInvalid] = useState(false);
  return (
    <form
      className="rn-open-run"
      role="search"
      aria-label="Open a run"
      onSubmit={(event) => {
        event.preventDefault();
        const id = parseRunId(text);
        if (id === undefined) {
          setInvalid(true);
          return;
        }
        setInvalid(false);
        void navigate(path`/runs/${id}`);
      }}
    >
      <TextInput
        type="search"
        label="Open a run by id"
        width="20rem"
        mono
        error={invalid ? 'That is not a run id. It looks like 01a13037-a113-7388-932b-5659bdd2aa84.' : undefined}
        value={text}
        autoComplete="off"
        spellCheck={false}
        onChange={(next) => {
          setText(next);
          setInvalid(false);
        }}
      />
      <Button type="submit">Open</Button>
    </form>
  );
}
