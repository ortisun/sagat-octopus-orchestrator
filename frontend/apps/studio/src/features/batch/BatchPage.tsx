import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { useParams } from 'react-router';
import type { BatchJob } from '../../api/endpoints';
import { useApp, useCan } from '../../app/context';
import { Actions, Banner, Button, Code, ColumnHeader, Dialog, Empty, ErrorState, Facts, FocusHeading, Heading, Link, Loading, Page, Progress, Row, RowHeader, Select, Stack, StatusBadge, Table, TableBody, TableCell, TableHead, TableRow, Text, TextArea, useAnnounce } from '../../ui';
import { NotAllowed } from '../runs/NotAllowed';
import { ACTION_TEXT, ITEM_STATES, failureText, isActive, itemState, jobState, progress, reasonProblem, reasonText } from './model';
import type { BatchActionName } from './model';
import { notServed, useNamespaceId } from './queries';
import { shortId } from '../runs/display';

const JOB_REFRESH_MS = 2_000;
const ITEMS_REFRESH_MS = 3_000;
const PAGE = 100;

/** `/batches/:batchId`: how far a job is, what became of each run, and the way to stop it. */
export function BatchPage() {
  const allowed = useCan('run:read');
  const { batchId } = useParams();
  if (!allowed) return <NotAllowed what="Batch operations" permission="run:read" />;
  // A different job is a different page: nothing of the last one carries over.
  return <Job key={batchId} id={batchId ?? ''} />;
}

function Job({ id }: { id: string }) {
  const { ep } = useApp();
  const namespace = useNamespaceId();
  const job = useQuery({
    queryKey: ['batch', namespace.id, id],
    enabled: namespace.id !== undefined,
    queryFn: ({ signal }) => ep.getBatch(namespace.id as string, id, signal),
    refetchInterval: (query) => (isActive(query.state.data?.state) ? JOB_REFRESH_MS : false),
  });
  if (namespace.pending || job.isPending) return <Loading>Loading the batch job…</Loading>;
  if (namespace.error || job.error) {
    if (notServed(job.error)) return <Banner tone="info">Batch operations are not enabled on this deployment.</Banner>;
    return <ErrorState error={namespace.error ?? job.error} what="Loading the batch job" onRetry={() => void job.refetch()} />;
  }
  return (
    <Page>
      <Stack as="section" gap={4} aria-labelledby="batch-title">
        <Text>
          <Link to="/batches">All batch jobs</Link>
        </Text>
        <Summary job={job.data} namespaceId={namespace.id as string} />
        <Items job={job.data} namespaceId={namespace.id as string} />
      </Stack>
    </Page>
  );
}

function Summary({ job, namespaceId }: { job: BatchJob; namespaceId: string }) {
  const canCancel = useCan('run:batch');
  const { done, total, percent } = progress(job);
  const action = ACTION_TEXT[job.action as BatchActionName];
  const state = jobState(job.state);
  const counts = job.counts;
  // The button that opened the dialog of the cancellation goes when the job is cancelling (it cannot be cancelled twice): the dialog
  // closes over nothing, and focus would fall to the top of the page for a person at a keyboard. It goes to the heading, which
  // says what the job is now.
  const heading = useRef<HTMLHeadingElement>(null);
  const [cancelled, setCancelled] = useState(0);
  useEffect(() => {
    if (cancelled > 0) heading.current?.focus();
  }, [cancelled]);
  return (
    <>
      <FocusHeading level={1} id="batch-title" ref={heading}>
        Batch job: {action?.label ?? job.action} <StatusBadge view={state} />
      </FocusHeading>
      <Text>
        Started by {job.createdBy} on <time dateTime={job.createdAt}>{new Date(job.createdAt).toLocaleString('en')}</time>
        {job.maxItems ? `, at most ${job.maxItems.toLocaleString('en')} runs` : ''}
        {job.itemsPerSecond ? `, ${job.itemsPerSecond} a second` : ''}.
      </Text>
      {job.hasPayload ? <Text>It carries a payload, which is not shown.</Text> : null}
      {job.failureReason ? (
        <Banner tone="error" title="The job stopped.">
          {failureText(job.failureReason)}
        </Banner>
      ) : null}
      <Text>
        <Progress label="Runs with an outcome" value={done} max={total} /> {done} of {total} runs have an outcome ({percent}%).
        {isActive(job.state) ? ' This page follows the job while it works.' : ''}
      </Text>
      <Facts
        items={[
          { term: 'Succeeded', value: counts.succeeded },
          { term: 'Skipped', value: counts.skipped },
          { term: 'Failed', value: counts.failed },
          { term: 'Cancelled', value: counts.cancelled },
          { term: 'To be tried again', value: counts.deferred },
        ]}
      />
      {canCancel && (job.state === 'SELECTING' || job.state === 'RUNNING') ? (
        <Row>
          <CancelJob id={job.batchId} namespaceId={namespaceId} onCancelled={() => setCancelled((count) => count + 1)} />
        </Row>
      ) : null}
    </>
  );
}

function CancelJob({ id, namespaceId, onCancelled }: { id: string; namespaceId: string; onCancelled: () => void }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button variant="danger" onClick={() => setOpen(true)}>
        Cancel the job
      </Button>
      <Dialog open={open} title="Cancel the batch job" onClose={() => setOpen(false)}>
        {open ? <CancelForm id={id} namespaceId={namespaceId} onCancelled={onCancelled} onClose={() => setOpen(false)} /> : null}
      </Dialog>
    </>
  );
}

function CancelForm({ id, namespaceId, onCancelled, onClose }: { id: string; namespaceId: string; onCancelled: () => void; onClose: () => void }) {
  const { ep } = useApp();
  const queries = useQueryClient();
  const announce = useAnnounce();
  const [reason, setReason] = useState('');
  const [problem, setProblem] = useState<string>();
  const [error, setError] = useState<unknown>();
  const [sending, setSending] = useState(false);
  const key = useRef<{ reason: string; key: string } | undefined>(undefined);
  const pending = useRef<AbortController | undefined>(undefined);
  useEffect(() => () => pending.current?.abort(), []);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (sending) return;
    const why = reasonProblem(reason);
    if (why) {
      setProblem(why);
      return;
    }
    setProblem(undefined);
    setError(undefined);
    const trimmed = reason.trim();
    if (key.current?.reason !== trimmed) key.current = { reason: trimmed, key: crypto.randomUUID() };
    const controller = new AbortController();
    pending.current = controller;
    setSending(true);
    void ep
      .cancelBatch(namespaceId, id, trimmed, key.current.key, controller.signal)
      .then(async () => {
        if (controller.signal.aborted) return;
        announce('The batch job is being cancelled.');
        await queries.invalidateQueries({ queryKey: ['batch', namespaceId, id] });
        onClose();
        onCancelled();
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted) setError(cause);
      })
      .finally(() => {
        if (!controller.signal.aborted) setSending(false);
      });
  };

  return (
    <form onSubmit={submit} noValidate>
      <Stack gap={3}>
        <Text>
          The job stops after the run in hand. The runs that were waiting become cancelled. <strong>What was already applied stays applied:</strong> cancelling the job does not undo it.
        </Text>
        <TextArea label="Reason" hint="8 to 2,000 characters." rows={3} value={reason} disabled={sending} onChange={setReason} />
        {problem ? <Banner tone="error">{problem}</Banner> : null}
        {error ? <ErrorState error={error} what="Cancelling the job" /> : null}
        <Actions>
          <Button onClick={onClose}>Close</Button>
          <Button type="submit" variant="danger" disabled={sending}>
            {sending ? 'Cancelling…' : 'Confirm cancellation'}
          </Button>
        </Actions>
      </Stack>
    </form>
  );
}

function Items({ job, namespaceId }: { job: BatchJob; namespaceId: string }) {
  const { ep } = useApp();
  const [state, setState] = useState('');
  const items = useInfiniteQuery({
    queryKey: ['batch-items', namespaceId, job.batchId, state],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) => ep.batchItems(namespaceId, job.batchId, { limit: PAGE, ...(pageParam ? { cursor: pageParam } : {}), ...(state ? { state } : {}) }, signal),
    getNextPageParam: (last) => last.nextCursor ?? undefined,
    refetchInterval: isActive(job.state) ? ITEMS_REFRESH_MS : false,
  });
  // The page stops following the runs when the job ends, and the last read may be up to three seconds older than the job's last
  // step (the page opened while it was still selecting, say): one more read when it goes from at work to over, so what the page
  // says of a finished job is what the job did.
  const { refetch } = items;
  const active = isActive(job.state);
  const wasActive = useRef(active);
  useEffect(() => {
    if (wasActive.current && !active) void refetch();
    wasActive.current = active;
  }, [active, refetch]);
  const rows = items.data?.pages.flatMap((page) => page.items) ?? [];
  return (
    <Stack as="section" gap={3} aria-labelledby="batch-items-title">
      <Heading level={2} id="batch-items-title">
        Runs
      </Heading>
      <Row gap={3} align="end">
        <Select
          label="Outcome"
          width={240}
          value={state}
          placeholder="All outcomes"
          onChange={setState}
          options={[{ value: '', label: 'All outcomes' }, ...ITEM_STATES.map((name) => ({ value: name, label: itemState(name).label }))]}
        />
      </Row>
      {items.isPending ? <Loading>Loading the runs…</Loading> : null}
      {items.error ? <ErrorState error={items.error} what="Listing the runs of the job" onRetry={() => void items.refetch()} /> : null}
      {items.isSuccess && rows.length === 0 ? <Empty title={state ? 'No run with this outcome.' : isActive(job.state) ? 'No run has been selected yet.' : 'The job took no run.'} /> : null}
      {rows.length > 0 ? (
        <Table label="The runs of the job and what became of each">
          <TableHead>
            <TableRow>
              <ColumnHeader>#</ColumnHeader>
              <ColumnHeader>Run</ColumnHeader>
              <ColumnHeader>Outcome</ColumnHeader>
              <ColumnHeader>Why</ColumnHeader>
              <ColumnHeader>Tries</ColumnHeader>
              <ColumnHeader>New run</ColumnHeader>
            </TableRow>
          </TableHead>
          <TableBody>
            {rows.map((item) => (
              <TableRow key={item.ordinal}>
                <TableCell>{item.ordinal}</TableCell>
                <RowHeader>
                  <Link to={`/runs/${item.runId}`}>{shortId(item.runId)}</Link>
                </RowHeader>
                <TableCell>
                  <StatusBadge view={itemState(item.state)} />
                </TableCell>
                <TableCell>{item.errorCode ? <Code>{item.errorCode}</Code> : reasonText(item.reason)}</TableCell>
                <TableCell>{item.attempts ?? 0}</TableCell>
                <TableCell>{item.newRunId ? <Link to={`/runs/${item.newRunId}`}>{shortId(item.newRunId)}</Link> : null}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      ) : null}
      {items.hasNextPage ? (
        <Row>
          <Button onClick={() => void items.fetchNextPage()} disabled={items.isFetchingNextPage}>
            {items.isFetchingNextPage ? 'Loading…' : 'Load more'}
          </Button>
        </Row>
      ) : null}
    </Stack>
  );
}
