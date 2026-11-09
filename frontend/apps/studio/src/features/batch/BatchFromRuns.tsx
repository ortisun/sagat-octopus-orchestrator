import { useEffect, useMemo, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { Link, useNavigate } from 'react-router';
import type { BatchPreview } from '../../api/endpoints';
import { useApp, useCan, useNamespace, useSession } from '../../app/context';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { Dialog } from '../../ui/Dialog';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import { useAnnounce } from '../../ui/Announcer';
import { ACTION_TEXT, BATCH_ACTIONS, EMPTY_DRAFT, describeFilters, maxItemsOf, requestOf, selectorOf } from './model';
import type { BatchActionName, Draft, RunFilters } from './model';
import { notServed, useNamespaceId } from './queries';
import { shortId } from '../runs/display';

const DEFAULT_MAX = 10_000;

/**
 * The button of the run list that turns its filters into a batch job. It belongs to one session and
 * one namespace: nothing typed in it outlives either.
 */
export function BatchFromRuns({ filters, workflowName }: { filters: RunFilters; workflowName?: string }) {
  const allowed = useCan('run:batch');
  const namespace = useNamespace();
  useSession();
  const { session } = useApp();
  if (!allowed) return null;
  return <Scoped key={`${namespace ?? ''}|${session.revision()}`} filters={filters} {...(workflowName ? { workflowName } : {})} />;
}

function Scoped({ filters, workflowName }: { filters: RunFilters; workflowName?: string }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button onClick={() => setOpen(true)}>Batch operation…</Button>
      <Dialog open={open} title="Batch operation" onClose={() => setOpen(false)}>
        {open ? <BatchForm filters={filters} {...(workflowName ? { workflowName } : {})} onClose={() => setOpen(false)} /> : null}
      </Dialog>
    </>
  );
}

function BatchForm({ filters, workflowName, onClose }: { filters: RunFilters; workflowName?: string; onClose: () => void }) {
  const { ep, session } = useApp();
  const announce = useAnnounce();
  const navigate = useNavigate();
  const namespace = useNamespaceId();
  const [draft, setDraft] = useState<Draft>(EMPTY_DRAFT);
  const [reason, setReason] = useState('');
  const [preview, setPreview] = useState<BatchPreview>();
  const [previewedFor, setPreviewedFor] = useState('');
  const [previewing, setPreviewing] = useState(false);
  const [sending, setSending] = useState(false);
  const [problem, setProblem] = useState<string>();
  const [error, setError] = useState<unknown>();
  const previous = useRef<{ fingerprint: string; key: string } | undefined>(undefined);
  const pending = useRef<AbortController | undefined>(undefined);
  useEffect(() => () => pending.current?.abort(), []);

  const selector = useMemo(() => selectorOf(filters), [filters]);
  const set = (changes: Partial<Draft>) => setDraft((current) => ({ ...current, ...changes }));
  // What was counted is what the job would take: a different limit is a different count.
  const counted = preview !== undefined && previewedFor === draft.maxItems.trim() ? preview : undefined;
  const parsed = maxItemsOf(draft.maxItems);
  const limit = ('value' in parsed ? parsed.value : undefined) ?? DEFAULT_MAX;

  const run = async <T,>(start: (signal: AbortSignal) => Promise<T>, finish: (result: T) => void, busy: (on: boolean) => void) => {
    const controller = new AbortController();
    pending.current?.abort();
    pending.current = controller;
    const token = session.token();
    const current = () => !controller.signal.aborted && session.token() === token;
    setError(undefined);
    busy(true);
    try {
      const result = await start(controller.signal);
      if (current()) finish(result);
    } catch (cause) {
      if (current()) setError(cause);
    } finally {
      if (current()) busy(false);
    }
  };

  const countThem = () => {
    if (namespace.id === undefined || previewing || sending) return;
    const max = maxItemsOf(draft.maxItems);
    if ('problem' in max) {
      setProblem(max.problem);
      return;
    }
    setProblem(undefined);
    setPreview(undefined);
    void run(
      (signal) => ep.previewBatch(namespace.id as string, { selector, ...(max.value !== undefined ? { maxItems: max.value } : {}) }, signal),
      (result) => {
        setPreview(result);
        setPreviewedFor(draft.maxItems.trim());
        announce(result.capped ? `More than ${limit} runs match.` : `${result.matched} runs match.`);
      },
      setPreviewing,
    );
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (namespace.id === undefined || sending || previewing) return;
    if (counted === undefined) {
      setProblem('Count the runs first, so you know how many the job will take.');
      return;
    }
    if (counted.matched === 0) {
      setProblem('No run matches these filters: there is nothing to do.');
      return;
    }
    const made = requestOf(draft, selector, reason);
    if ('problem' in made) {
      setProblem(made.problem);
      return;
    }
    setProblem(undefined);
    const fingerprint = JSON.stringify(made.body);
    if (previous.current?.fingerprint !== fingerprint) previous.current = { fingerprint, key: crypto.randomUUID() };
    const key = previous.current.key;
    void run(
      (signal) => ep.createBatch(namespace.id as string, made.body, key, signal),
      (job) => {
        announce('The batch job was accepted.');
        onClose();
        void navigate(`/batches/${job.batchId}`);
      },
      setSending,
    );
  };

  if (namespace.pending) return <p role="status">Loading the namespace…</p>;
  if (namespace.id === undefined) return <Banner tone="error">The namespace could not be found. Choose one in the menu.</Banner>;
  const action = ACTION_TEXT[draft.action];
  const idle = !sending && !previewing;
  return (
    <form onSubmit={submit} noValidate>
      <p>
        A batch job applies one action to {describeFilters(filters, workflowName)}, one run at a time, at a fixed rate. It selects the runs that exist when it is
        created. What it applies is not undone if you cancel it.
      </p>
      <Field label="Action">
        <select value={draft.action} disabled={!idle} onChange={(event) => set({ action: event.target.value as BatchActionName })}>
          {BATCH_ACTIONS.map((name) => (
            <option key={name} value={name}>
              {ACTION_TEXT[name].label}
            </option>
          ))}
        </select>
      </Field>
      <p>
        <strong>{action.label}</strong> {action.effect}
      </p>
      {draft.action === 'SIGNAL' ? (
        <>
          <Field label="Signal name">
            <input autoComplete="off" spellCheck={false} maxLength={128} value={draft.signalName} disabled={!idle} onChange={(event) => set({ signalName: event.target.value })} />
          </Field>
          <Field label="Payload (JSON object)" hint="At most 256 KiB. Do not include credentials or secrets.">
            <textarea rows={5} autoComplete="off" spellCheck={false} value={draft.payload} disabled={!idle} onChange={(event) => set({ payload: event.target.value })} />
          </Field>
        </>
      ) : null}
      {draft.action === 'RETRY_STEP' || draft.action === 'MARK_MANUAL_REVIEW' ? (
        <Field label="Step" hint="The id of a step of the workflow, such as charge, or of a compensation, such as reserve~c.">
          <input autoComplete="off" spellCheck={false} maxLength={200} value={draft.stepId} disabled={!idle} onChange={(event) => set({ stepId: event.target.value })} />
        </Field>
      ) : null}
      <div className="toolbar">
        <Field label="Most runs" hint={`Leave empty for ${DEFAULT_MAX.toLocaleString('en')}; at most 100,000.`}>
          <input inputMode="numeric" autoComplete="off" value={draft.maxItems} disabled={!idle} onChange={(event) => set({ maxItems: event.target.value })} />
        </Field>
        <Field label="Runs a second" hint="Leave empty for 20; at most 200.">
          <input inputMode="decimal" autoComplete="off" value={draft.rate} disabled={!idle} onChange={(event) => set({ rate: event.target.value })} />
        </Field>
      </div>
      <div className="toolbar">
        <Button onClick={countThem} disabled={!idle}>
          {previewing ? 'Counting…' : 'Count the runs'}
        </Button>
        {counted ? (
          <p role="status">
            {counted.capped
              ? `More than ${limit.toLocaleString('en')} runs match; the job takes the first ${limit.toLocaleString('en')}.`
              : `${counted.matched.toLocaleString('en')} ${counted.matched === 1 ? 'run matches' : 'runs match'}.`}
            {counted.sample.length > 0 ? (
              <>
                {' '}
                For example{' '}
                {counted.sample.slice(0, 3).map((id, index) => (
                  <span key={id}>
                    {index > 0 ? ', ' : ''}
                    <Link to={`/runs/${id}`}>{shortId(id)}</Link>
                  </span>
                ))}
                .
              </>
            ) : null}
          </p>
        ) : null}
      </div>
      <Field label="Reason" hint="8 to 2,000 characters. It is kept with the job.">
        <textarea rows={3} autoComplete="off" value={reason} disabled={!idle} onChange={(event) => setReason(event.target.value)} />
      </Field>
      {problem ? (
        <p role="alert" className="error">
          {problem}
        </p>
      ) : null}
      {error && notServed(error) ? (
        <Banner tone="info">Batch operations are not enabled on this deployment.</Banner>
      ) : error ? (
        <ErrorState error={error} what="The batch operation" />
      ) : null}
      <div className="toolbar">
        <Button type="submit" variant="primary" disabled={!idle}>
          {sending ? 'Starting…' : 'Start the batch job'}
        </Button>
        <Button onClick={onClose}>Close</Button>
      </div>
    </form>
  );
}
