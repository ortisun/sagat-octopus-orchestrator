import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { Link, useNavigate } from 'react-router';
import { path } from '../../api/client';
import { isApiError } from '../../api/errors';
import { isTerminal } from '../../api/follow';
import type { ResetWrite } from '../../api/endpoints';
import type { ReapplyEntry, ReapplyPolicy, Reset, ResetPlan, ResetPoint } from '../../api/types';
import { useApp, useCan, useNamespace, useSession } from '../../app/context';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { Dialog } from '../../ui/Dialog';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import { useAnnounce } from '../../ui/Announcer';
import { refusal } from './OperationDialog';
import { MAX_REASON, checkReason } from './ops';
import { clip, formatLocal } from './time';

/** What a place a run can be reset to is, in words: where it is in the run, not the number of a row of the journal. */
export function pointLabel(point: Pick<ResetPoint, 'kind' | 'node'>): string {
  const node = point.node === null || point.node === undefined ? '' : clip(point.node, 60);
  switch (point.kind) {
    case 'RUN_STARTED':
      return 'The start of the run';
    case 'STEP_COMPLETED':
      return `After step ${node}`;
    case 'CHILD_FINISHED':
      return `After child run ${node}`;
    case 'COLLECTION_FINISHED':
      return `After collection ${node}`;
    case 'LOOP_FINISHED':
      return `After loop ${node}`;
  }
}

/** What a step declares about running it again: said in words, not only coloured. */
const MARKERS: Record<string, string> = {
  compensateWith: 'it has a compensation',
  reconcileWith: 'it can be reconciled with the provider',
  irreversible: 'what it does cannot be undone',
  highRisk: 'it is marked high risk',
};

/** Why a signal or an Update was not delivered again, or was refused, in words. A reason this Studio does not know is shown as the platform wrote it. */
const REASONS: Record<string, string> = {
  LEGACY_EVENT: 'it was recorded before the platform said what it was',
  NOT_RETAINED: 'its content is no longer kept',
  PREVIOUS_SIGNAL_NOT_DELIVERED: 'an earlier signal was not delivered, and a later one would reach its wait',
  NOT_WAITING: 'no step of the new run was waiting for it',
  UPDATE_REJECTED: 'the validator of the update declined it',
  REVISION_CONFLICT: 'the state was not at the revision the update was applied to',
  UPDATE_NOT_ACCEPTED: 'the declaration does not accept the update',
};

const OUTCOMES: Record<string, string> = {
  WILL_TRY: 'will be tried',
  REAPPLIED: 'delivered',
  REJECTED: 'rejected',
  NOT_REAPPLIED: 'not delivered',
};

/** A signal or an Update a reset delivers again, or did, in words: its name and what came of it, never what was sent. */
export function entryLabel(entry: ReapplyEntry): string {
  // The names of kind, outcome and reason are open (a later platform may add one): what is not known is shown as written.
  const what = entry.kind === 'SIGNAL' ? 'Signal' : entry.kind === 'UPDATE' ? 'Update' : clip(entry.kind, 30);
  const name = clip(entry.kind === 'SIGNAL' ? (entry.name ?? '?') : entry.kind === 'UPDATE' ? (entry.operation ?? '?') : (entry.name ?? entry.operation ?? '?'), 60);
  const because = entry.reason === undefined ? '' : `: ${REASONS[entry.reason] ?? clip(entry.reason, 60)}`;
  return `${what} ${name} (event ${entry.sequence}): ${OUTCOMES[entry.outcome] ?? clip(entry.outcome, 30)}${because}`;
}

/** What the person may ask to have delivered again after the point, and what each choice means. */
const POLICIES: { value: ReapplyPolicy; label: string }[] = [
  { value: 'NONE', label: 'Nothing: the new run does not receive them' },
  { value: 'SIGNALS', label: 'The signals the run received' },
  { value: 'UPDATES', label: 'The updates whose arguments were kept' },
  { value: 'ALL', label: 'Both, in the order they arrived' },
];

/**
 * Starts a new run from a point of a run that ended. The run offers it only to someone who may operate and start runs,
 * and only when the run is over; the form belongs to one authenticated context and one run and never persists anything.
 */
export function ResetRun({ runId, status, businessId }: { runId: string; status: string; businessId?: string | null }) {
  const operate = useCan('run:operate');
  const start = useCan('run:start');
  const namespace = useNamespace();
  useSession();
  const { session } = useApp();
  if (!operate || !start || !isTerminal(status)) return null;
  return <ScopedReset key={`${namespace ?? ''}|${session.revision()}|${runId}`} runId={runId} businessId={businessId ?? undefined} />;
}

function ScopedReset({ runId, businessId }: { runId: string; businessId?: string }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button onClick={() => setOpen(true)}>Reset to a point…</Button>
      <Dialog open={open} title="Reset to a point" onClose={() => setOpen(false)}>
        {open ? <ResetForm runId={runId} {...(businessId === undefined ? {} : { businessId })} onClose={() => setOpen(false)} /> : null}
      </Dialog>
    </>
  );
}

function ResetForm({ runId, businessId, onClose }: { runId: string; businessId?: string; onClose: () => void }) {
  const { ep, session } = useApp();
  const queries = useQueryClient();
  const announce = useAnnounce();
  const navigate = useNavigate();
  const [points, setPoints] = useState<ResetPoint[] | undefined>(undefined);
  const [pointsError, setPointsError] = useState<unknown>(undefined);
  const [chosen, setChosen] = useState<number | undefined>(undefined);
  const [plan, setPlan] = useState<ResetPlan | undefined>(undefined);
  const [planError, setPlanError] = useState<unknown>(undefined);
  const [acknowledged, setAcknowledged] = useState(false);
  const [dropId, setDropId] = useState(false);
  const [reapply, setReapply] = useState<ReapplyPolicy>('NONE');
  const [made, setMade] = useState<Reset | undefined>(undefined);
  const [reason, setReason] = useState('');
  const [submitted, setSubmitted] = useState(false);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<unknown>(undefined);
  // The key goes with the exact request: sending the same thing again (after a timeout) keeps it, so the platform
  // makes one reset; changing anything makes a new one.
  const previous = useRef<{ print: string; key: string } | undefined>(undefined);
  const pending = useRef<AbortController | undefined>(undefined);
  const reasonRef = useRef<HTMLTextAreaElement>(null);
  useEffect(() => () => pending.current?.abort(), []);

  // where the run can be reset to
  useEffect(() => {
    const controller = new AbortController();
    ep.resetPoints(runId, controller.signal).then(
      (found) => {
        if (!controller.signal.aborted) setPoints(found);
      },
      (cause: unknown) => {
        if (!controller.signal.aborted) setPointsError(cause);
      },
    );
    return () => controller.abort();
  }, [ep, runId]);

  // what a reset to the point chosen would do
  useEffect(() => {
    if (chosen === undefined) return;
    const controller = new AbortController();
    ep.previewReset(runId, chosen, reapply, controller.signal).then(
      (found) => {
        if (!controller.signal.aborted) setPlan(found);
      },
      (cause: unknown) => {
        if (!controller.signal.aborted) setPlanError(cause);
      },
    );
    return () => controller.abort();
  }, [ep, runId, chosen, reapply]);

  // Choosing another point forgets what was read for the one before and what was acknowledged about it.
  const choose = (sequence: number) => {
    setChosen(sequence);
    setPlan(undefined);
    setPlanError(undefined);
    setAcknowledged(false);
  };

  // Asking for something else to be delivered again reads what the reset would do again; what was acknowledged about
  // the steps is about the steps and stays.
  const chooseReapply = (policy: ReapplyPolicy) => {
    setReapply(policy);
    setPlan(undefined);
    setPlanError(undefined);
  };

  const reasonCheck = checkReason(reason, 'run');
  const reasonProblem = submitted ? reasonCheck.problem : undefined;
  const mustAcknowledge = plan?.requiresAcknowledgement === true;
  const acknowledgeProblem = submitted && mustAcknowledge && !acknowledged ? 'Confirm that you know these steps will run again.' : undefined;

  const send = async () => {
    if (chosen === undefined) return;
    const body: ResetWrite = {
      sequence: chosen,
      reason: reason.trim(),
      acknowledgeReexecution: mustAcknowledge && acknowledged,
      // keeping the identifier is the default and is not sent (and only a run that has one offers the choice)
      ...(businessId !== undefined && dropId ? { businessId: 'DROP' as const } : {}),
      // nothing delivered again is the default and is not sent
      ...(reapply === 'NONE' ? {} : { reapply }),
    };
    const print = JSON.stringify(body);
    if (previous.current?.print !== print) previous.current = { print, key: crypto.randomUUID() };
    const controller = new AbortController();
    pending.current = controller;
    const token = session.token();
    const current = () => !controller.signal.aborted && session.token() === token;
    setSending(true);
    setError(undefined);
    try {
      const done: Reset = await ep.resetRun(runId, body, previous.current.key, controller.signal);
      if (!current()) return;
      announce('The reset was made. The new run starts from that point.');
      void queries.invalidateQueries({ queryKey: ['run', runId] });
      // what was delivered again is said before the person is taken away: it is the only place the report is read
      if ((done.reapplyReport ?? []).length > 0) {
        setMade(done);
        return;
      }
      onClose();
      if (done.newRunId) void navigate(path`/runs/${done.newRunId}`);
    } catch (cause) {
      if (current()) setError(cause);
    } finally {
      if (current()) setSending(false);
    }
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (sending) return;
    setSubmitted(true);
    if (chosen === undefined || plan === undefined) return;
    if (reasonCheck.problem !== undefined) {
      reasonRef.current?.focus();
      return;
    }
    if (mustAcknowledge && !acknowledged) return;
    void send();
  };

  if (made !== undefined) return <ResetMade made={made} onClose={onClose} />;

  return (
    <form onSubmit={submit} noValidate className="rn-decision">
      <p>A reset starts a new run from a point of this run: what the run had done up to the point is kept as it was, and the new run goes on from there. This run stays as it is.</p>
      <p className="rn-audit">This decision is recorded with your name and reason.</p>

      {pointsError !== undefined ? <ErrorState error={pointsError} what="Reading where this run can be reset to" /> : null}
      {points === undefined && pointsError === undefined ? <p role="status">Reading where this run can be reset to…</p> : null}
      {points !== undefined && points.length === 0 ? (
        <Banner tone="info" title="This run cannot be reset to any point.">
          A point is the start of the run, or the end of a step at its root where nothing else was in flight.
        </Banner>
      ) : null}
      {points !== undefined && points.length > 0 ? (
        <fieldset className="rn-outcome">
          <legend>Reset to</legend>
          {points.map((point) => (
            <label key={point.sequence}>
              <input type="radio" name="point" value={point.sequence} checked={chosen === point.sequence} onChange={() => choose(point.sequence)} /> {pointLabel(point)}{' '}
              <span className="hint">({formatLocal(point.occurredAt)})</span>
            </label>
          ))}
        </fieldset>
      ) : null}
      {points !== undefined && points.length > 0 ? (
        <fieldset className="rn-outcome">
          <legend>Deliver again what the run received after that point</legend>
          {POLICIES.map((policy) => (
            <label key={policy.value}>
              <input type="radio" name="reapply" value={policy.value} checked={reapply === policy.value} onChange={() => chooseReapply(policy.value)} /> {policy.label}
            </label>
          ))}
          <span className="hint">They go to the new run in the order they arrived, when it is made. A signal that nothing waits for ends the signals; an update goes only to the state it was applied to.</span>
        </fieldset>
      ) : null}
      {submitted && chosen === undefined && points !== undefined && points.length > 0 ? (
        <span className="error" role="alert">
          Choose where to reset the run to.
        </span>
      ) : null}

      {planError !== undefined ? <ErrorState error={planError} what="Reading what the reset would do" /> : null}
      {chosen !== undefined && plan === undefined && planError === undefined ? <p role="status">Reading what the reset would do…</p> : null}
      {plan !== undefined ? <Plan plan={plan} policy={reapply} /> : null}

      {plan !== undefined && mustAcknowledge ? (
        <div>
          <label>
            <input type="checkbox" checked={acknowledged} onChange={(event) => setAcknowledged(event.target.checked)} /> I know that these steps ran before and will run again, and that running them again repeats what they did.
          </label>
          {acknowledgeProblem ? (
            <span className="error" role="alert">
              {acknowledgeProblem}
            </span>
          ) : null}
        </div>
      ) : null}

      {businessId !== undefined ? (
        <fieldset className="rn-outcome">
          <legend>Business identifier {clip(businessId, 60)}</legend>
          <label>
            <input type="radio" name="business" checked={!dropId} onChange={() => setDropId(false)} /> Keep it on the new run (refused while another open run holds it)
          </label>
          <label>
            <input type="radio" name="business" checked={dropId} onChange={() => setDropId(true)} /> Start the new run without it
          </label>
        </fieldset>
      ) : null}

      <Field label="Reason" hint={`8 to ${MAX_REASON} characters, kept with the decision. ${reasonCheck.length} / ${MAX_REASON}`} error={reasonProblem}>
        <textarea ref={reasonRef} value={reason} rows={4} onChange={(event) => setReason(event.target.value)} />
      </Field>

      {error !== undefined ? <ErrorState error={refusal(error)} what="Resetting the run" /> : null}

      <div className="dialog-actions">
        <Button onClick={onClose}>Cancel</Button>
        <Button type="submit" variant="primary" disabled={sending} aria-disabled={plan === undefined ? true : undefined}>
          {sending ? 'Sending…' : 'Confirm: reset the run'}
        </Button>
      </div>
    </form>
  );
}

/** What a reset to the chosen point keeps and what it runs again, by step, with what each declares. */
function Plan({ plan, policy }: { plan: ResetPlan; policy: ReapplyPolicy }) {
  return (
    <section aria-label="What the reset does" className="rn-plan">
      <p>
        <strong>Kept as it was:</strong> {plan.restored.length === 0 ? 'nothing; the new run starts from the beginning.' : plan.restored.map((step) => `${clip(step.node, 60)}${step.kind === 'APPROVAL' ? ' (an approval decision)' : ''}`).join(', ')}
      </p>
      <p>
        <strong>Runs again:</strong> {plan.reexecuted.length === 0 ? 'nothing; the run has nothing left to do.' : null}
      </p>
      {plan.reexecuted.length > 0 ? (
        <ul>
          {plan.reexecuted.map((step) => (
            <li key={step.node}>
              {clip(step.node, 60)}
              {step.completedInSource ? ' (it completed in this run: running it again repeats what it did)' : ' (it had not completed)'}
              {step.markers.length > 0 ? `; ${step.markers.map((marker) => MARKERS[marker] ?? marker).join('; ')}` : ''}
            </li>
          ))}
        </ul>
      ) : null}
      {policy === 'NONE' ? null : plan.reapply.length === 0 ? (
        <p>
          <strong>Delivered again:</strong> nothing came after this point.
        </p>
      ) : (
        <>
          <p>
            <strong>Delivered again, in this order:</strong>
          </p>
          <ul>
            {plan.reapply.map((entry) => (
              <li key={entry.sequence}>{entryLabel(entry)}</li>
            ))}
          </ul>
        </>
      )}
    </section>
  );
}

/** What a reset that delivered signals and updates again says before the person goes to the new run: what came of each. */
function ResetMade({ made, onClose }: { made: Reset; onClose: () => void }) {
  const navigate = useNavigate();
  const report = made.reapplyReport ?? [];
  return (
    <div className="rn-decision">
      <p>The reset was made. The new run starts from that point.</p>
      <section aria-label="What was delivered again" className="rn-plan">
        <p>
          <strong>Delivered again, in the order the run received them:</strong>
        </p>
        <ul>
          {report.map((entry) => (
            <li key={entry.sequence}>{entryLabel(entry)}</li>
          ))}
        </ul>
      </section>
      <div className="dialog-actions">
        <Button onClick={onClose}>Close</Button>
        {made.newRunId ? (
          <Button
            variant="primary"
            onClick={() => {
              onClose();
              void navigate(path`/runs/${made.newRunId ?? ''}`);
            }}
          >
            Open the new run
          </Button>
        ) : null}
      </div>
    </div>
  );
}

/** The resets made from a run, each with a link to the run it made. */
export function ResetsMade({ runId }: { runId: string }) {
  const { ep } = useApp();
  const allowed = useCan('run:read');
  const namespace = useNamespace();
  const query = useQuery({
    queryKey: ['run', runId, 'resets', namespace ?? ''],
    queryFn: ({ signal }) => ep.runResets(runId, signal),
    enabled: allowed,
  });
  if (!allowed) return null;
  if (query.isError) {
    // a platform that does not serve the register (an older one than this Studio) has no resets to show
    if (isApiError(query.error) && query.error.code === 'NOT_FOUND') return null;
    return <ErrorState error={query.error} what="Reading the resets made from this run" onRetry={() => void query.refetch()} />;
  }
  const items = query.data?.items ?? [];
  if (items.length === 0) return null;
  return (
    <section aria-label="Resets made from this run" className="rn-resets">
      <h2>Resets made from this run</h2>
      <ul>
        {items.map((reset) => (
          <li key={reset.resetId}>
            {reset.newRunId ? <Link to={path`/runs/${reset.newRunId}`}>A run reset to point {reset.sequence}</Link> : <>A reset to point {reset.sequence}</>}{' '}
            <span className="hint">
              ({reset.state.toLowerCase().replace('_', ' ')}
              {reset.createdAt ? `, ${formatLocal(reset.createdAt)}` : ''})
            </span>
            {(reset.reapplyReport ?? []).length > 0 ? (
              <details>
                <summary>Delivered again ({(reset.reapplyReport ?? []).length})</summary>
                <ul>
                  {(reset.reapplyReport ?? []).map((entry) => (
                    <li key={entry.sequence}>{entryLabel(entry)}</li>
                  ))}
                </ul>
              </details>
            ) : null}
          </li>
        ))}
      </ul>
    </section>
  );
}
