import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { Link, useParams } from 'react-router';
import type { BatchJob } from '../../api/endpoints';
import { useApp, useCan } from '../../app/context';
import { useAnnounce } from '../../ui/Announcer';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { Dialog } from '../../ui/Dialog';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import { StatusBadge } from '../../ui/StatusBadge';
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
  if (namespace.pending || job.isPending) return <p role="status">Loading the batch job…</p>;
  if (namespace.error || job.error) {
    if (notServed(job.error)) return <Banner tone="info">Batch operations are not enabled on this deployment.</Banner>;
    return <ErrorState error={namespace.error ?? job.error} what="Loading the batch job" onRetry={() => void job.refetch()} />;
  }
  return (
    <section aria-labelledby="batch-title">
      <p>
        <Link to="/batches">All batch jobs</Link>
      </p>
      <Summary job={job.data} namespaceId={namespace.id as string} />
      <Items job={job.data} namespaceId={namespace.id as string} />
    </section>
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
      <h1 id="batch-title" ref={heading} tabIndex={-1}>
        Batch job: {action?.label ?? job.action} <StatusBadge view={state} />
      </h1>
      <p>
        Started by {job.createdBy} on <time dateTime={job.createdAt}>{new Date(job.createdAt).toLocaleString('en')}</time>
        {job.maxItems ? `, at most ${job.maxItems.toLocaleString('en')} runs` : ''}
        {job.itemsPerSecond ? `, ${job.itemsPerSecond} a second` : ''}.
      </p>
      {job.hasPayload ? <p>It carries a payload, which is not shown.</p> : null}
      {job.failureReason ? (
        <Banner tone="error" title="The job stopped.">
          {failureText(job.failureReason)}
        </Banner>
      ) : null}
      <p>
        <progress max={Math.max(total, 1)} value={done} aria-label="Runs with an outcome" /> {done} of {total} runs have an outcome ({percent}%).
        {isActive(job.state) ? ' This page follows the job while it works.' : ''}
      </p>
      <dl className="batch-counts">
        <div><dt>Succeeded</dt><dd>{counts.succeeded}</dd></div>
        <div><dt>Skipped</dt><dd>{counts.skipped}</dd></div>
        <div><dt>Failed</dt><dd>{counts.failed}</dd></div>
        <div><dt>Cancelled</dt><dd>{counts.cancelled}</dd></div>
        <div><dt>To be tried again</dt><dd>{counts.deferred}</dd></div>
      </dl>
      {canCancel && (job.state === 'SELECTING' || job.state === 'RUNNING') ? <CancelJob id={job.batchId} namespaceId={namespaceId} onCancelled={() => setCancelled((count) => count + 1)} /> : null}
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
      <p>
        The job stops after the run in hand. The runs that were waiting become cancelled. <strong>What was already applied stays applied:</strong> cancelling the job does not undo it.
      </p>
      <Field label="Reason" hint="8 to 2,000 characters.">
        <textarea rows={3} autoComplete="off" value={reason} disabled={sending} onChange={(event) => setReason(event.target.value)} />
      </Field>
      {problem ? (
        <p role="alert" className="error">
          {problem}
        </p>
      ) : null}
      {error ? <ErrorState error={error} what="Cancelling the job" /> : null}
      <div className="toolbar">
        <Button type="submit" variant="danger" disabled={sending}>
          {sending ? 'Cancelling…' : 'Confirm cancellation'}
        </Button>
        <Button onClick={onClose}>Close</Button>
      </div>
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
    <section aria-labelledby="batch-items-title">
      <h2 id="batch-items-title">Runs</h2>
      <div className="toolbar">
        <Field label="Outcome">
          <select value={state} onChange={(event) => setState(event.target.value)}>
            <option value="">All outcomes</option>
            {ITEM_STATES.map((name) => (
              <option key={name} value={name}>
                {itemState(name).label}
              </option>
            ))}
          </select>
        </Field>
      </div>
      {items.isPending ? <p role="status">Loading the runs…</p> : null}
      {items.error ? <ErrorState error={items.error} what="Listing the runs of the job" onRetry={() => void items.refetch()} /> : null}
      {items.isSuccess && rows.length === 0 ? <p>{state ? 'No run with this outcome.' : isActive(job.state) ? 'No run has been selected yet.' : 'The job took no run.'}</p> : null}
      {rows.length > 0 ? (
        <table>
          <caption className="visually-hidden">The runs of the job and what became of each</caption>
          <thead>
            <tr>
              <th scope="col">#</th>
              <th scope="col">Run</th>
              <th scope="col">Outcome</th>
              <th scope="col">Why</th>
              <th scope="col">Tries</th>
              <th scope="col">New run</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((item) => (
              <tr key={item.ordinal}>
                <td>{item.ordinal}</td>
                <th scope="row">
                  <Link to={`/runs/${item.runId}`}>{shortId(item.runId)}</Link>
                </th>
                <td>
                  <StatusBadge view={itemState(item.state)} />
                </td>
                <td>{item.errorCode ? <code>{item.errorCode}</code> : reasonText(item.reason)}</td>
                <td>{item.attempts ?? 0}</td>
                <td>{item.newRunId ? <Link to={`/runs/${item.newRunId}`}>{shortId(item.newRunId)}</Link> : null}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
      {items.hasNextPage ? (
        <Button onClick={() => void items.fetchNextPage()} disabled={items.isFetchingNextPage}>
          {items.isFetchingNextPage ? 'Loading…' : 'Load more'}
        </Button>
      ) : null}
    </section>
  );
}
