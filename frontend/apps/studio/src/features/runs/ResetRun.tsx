import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useId, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { Link, useNavigate } from 'react-router';
import { path } from '../../api/client';
import { ApiError, isApiError } from '../../api/errors';
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

/** Why a reset that waited for its run to end was not made, in words. A name this Studio does not know is shown as the platform wrote it. */
const FAILURES = new Map<string, string>([
  ['BOUNDARY_NO_LONGER_ELIGIBLE', 'the point is not one the run could be reset to once it had ended'],
  ['ADMISSION_REFUSED', 'a quota or the platform refused the new run'],
  ['BUSINESS_ID_IN_USE', 'another open run holds the business identifier'],
  ['AUTHORITY_REVOKED', 'the person who asked no longer has the permissions a reset needs'],
  ['REEXECUTION_NOT_ACKNOWLEDGED', 'steps completed after the point and the request did not say they may run again'],
  ['REAPPLY_TOO_MANY', 'there were too many signals and updates to deliver again'],
  ['CONCURRENCY_PERMIT_UNAVAILABLE', 'the limit on runs at once left no room for the new run'],
  ['RUN_PURGED', 'the run was deleted before the new run could be made'],
  ['SOURCE_DID_NOT_END', 'the run being cancelled had not ended within the time a reset waits for it'],
]);

function failureWords(reason: string | null | undefined): string {
  if (reason === null || reason === undefined || reason === '') return 'no reason was recorded';
  return FAILURES.get(reason) ?? clip(reason, 60);
}

/** What a reset is, in words, for the list of resets made from a run. The names of the states are open: what is not known is shown as written. */
function stateWords(reset: Reset): string {
  switch (reset.state) {
    case 'CREATED':
      return 'created';
    case 'PENDING_SOURCE':
      return 'waiting for this run to end';
    case 'FAILED':
      return `could not be made: ${failureWords(reset.failureReason)}`;
    default:
      return clip(String(reset.state), 30);
  }
}

/** A reset that has no new run: the run it came from is still being cancelled, or the reset failed. */
function notMade(reset: Reset): boolean {
  return reset.state === 'PENDING_SOURCE' || reset.state === 'FAILED';
}

/** The refusals the Studio words itself, because the advice of the platform is for someone who writes requests, not for someone who uses this screen. */
const REFUSALS = new Map<string, { message: string; remediation: string }>([
  ['RESET_CANCEL_SOURCE_NOT_AVAILABLE', { message: 'This platform has not been set up to cancel a run for a reset.', remediation: 'Wait for the run to end, then reset it.' }],
  ['PENDING_RESET_LIMIT', { message: 'This namespace already has as many resets waiting for a run to end as it may.', remediation: 'Wait until one of them has been made or has failed, then try again.' }],
]);

function resetRefusal(error: unknown): unknown {
  if (!isApiError(error) || error.reason === undefined) return refusal(error);
  const words = REFUSALS.get(error.reason);
  if (words === undefined) return refusal(error);
  return new ApiError({ code: error.code, status: error.status, message: words.message, remediation: words.remediation, traceId: error.traceId, details: error.details });
}

/** What the person may ask to have delivered again after the point, and what each choice means. */
const POLICIES: { value: ReapplyPolicy; label: string }[] = [
  { value: 'NONE', label: 'Nothing: the new run does not receive them' },
  { value: 'SIGNALS', label: 'The signals the run received' },
  { value: 'UPDATES', label: 'The updates whose arguments were kept' },
  { value: 'ALL', label: 'Both, in the order they arrived' },
];

/**
 * Starts a new run from a point of a run. The run offers it only to someone who may operate and start runs. For a run
 * that ended it is a reset now; for a run that has not ended it cancels the run and makes the new run when it has
 * ended. The form belongs to one authenticated context and one run and never persists anything.
 */
export function ResetRun({ runId, status, businessId }: { runId: string; status: string; businessId?: string | null }) {
  const operate = useCan('run:operate');
  const start = useCan('run:start');
  const namespace = useNamespace();
  useSession();
  const { session } = useApp();
  if (!operate || !start) return null;
  return <ScopedReset key={`${namespace ?? ''}|${session.revision()}|${runId}`} runId={runId} businessId={businessId ?? undefined} cancelSource={!isTerminal(status)} />;
}

function ScopedReset({ runId, businessId, cancelSource }: { runId: string; businessId?: string; cancelSource: boolean }) {
  // What the dialog was opened for stays what it is until it is closed: if the run ends meanwhile (the cancellation
  // this very form asked for), a request sent again after a lost answer must still be the same request.
  const [opened, setOpened] = useState<'cancel' | 'plain' | undefined>(undefined);
  const close = () => setOpened(undefined);
  return (
    <>
      <Button onClick={() => setOpened(cancelSource ? 'cancel' : 'plain')}>{cancelSource ? 'Cancel and reset to a point…' : 'Reset to a point…'}</Button>
      <Dialog open={opened !== undefined} title={opened === 'cancel' ? 'Cancel the run and reset it when it ends' : 'Reset to a point'} onClose={close}>
        {opened !== undefined ? <ResetForm runId={runId} cancelSource={opened === 'cancel'} {...(businessId === undefined ? {} : { businessId })} onClose={close} /> : null}
      </Dialog>
    </>
  );
}

function ResetForm({ runId, businessId, cancelSource, onClose }: { runId: string; businessId?: string; cancelSource: boolean; onClose: () => void }) {
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
  const cannotTellId = useId();
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
  // The steps of a run that has not ended go on while it is cancelled, and the Studio cannot know whether one that is in
  // flight will complete after the point: the acknowledgement is asked for whatever the plan says (the platform looks at
  // it again on the journal the run ended with).
  const mustAcknowledge = cancelSource || plan?.requiresAcknowledgement === true;
  const acknowledgeProblem = submitted && mustAcknowledge && !acknowledged ? (cancelSource ? 'Confirm that you know steps may run again.' : 'Confirm that you know these steps will run again.') : undefined;

  const send = async () => {
    if (chosen === undefined) return;
    const body: ResetWrite = {
      sequence: chosen,
      reason: reason.trim(),
      acknowledgeReexecution: mustAcknowledge && acknowledged,
      // a run that ended is reset at once and an older platform does not know the field: only a run that has not ended says it
      ...(cancelSource ? { sourcePolicy: 'CANCEL_SOURCE' as const } : {}),
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
      // the run, and the list of the resets made from it (a query under the run's own key), are read again
      void queries.invalidateQueries({ queryKey: ['run', runId] });
      // a run that is still being cancelled has no new run to go to, and a reset that failed will never have one:
      // the person is told what happened and stays where they are
      if (notMade(done)) {
        announce(done.state === 'FAILED' ? 'The reset could not be made.' : 'The run is being cancelled. The new run is made when it has ended.');
        setMade(done);
        return;
      }
      announce('The reset was made. The new run starts from that point.');
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

  if (made !== undefined) return notMade(made) ? <ResetNotMade made={made} onClose={onClose} /> : <ResetMade made={made} onClose={onClose} />;

  return (
    <form onSubmit={submit} noValidate className="rn-decision">
      {cancelSource ? (
        <Banner tone="warn" title="This run has not ended.">
          Confirming cancels it, in the ordinary way. The new run is made once it has ended; if the reset can no longer be made by then, or the run has not ended within the time the platform waits for it, the reset fails and the list of resets made from this run says why.
        </Banner>
      ) : null}
      <p>A reset starts a new run from a point of this run: what the run had done up to the point is kept as it was, and the new run goes on from there.{cancelSource ? '' : ' This run stays as it is.'}</p>
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
            <input type="checkbox" checked={acknowledged} aria-describedby={cancelSource ? cannotTellId : undefined} onChange={(event) => setAcknowledged(event.target.checked)} />{' '}
            {cancelSource
              ? 'I know that steps that completed after the point, including any that complete before this run ends, will run again, and that running them again repeats what they did.'
              : 'I know that these steps ran before and will run again, and that running them again repeats what they did.'}
          </label>
          {cancelSource ? (
            <span id={cannotTellId} className="hint">
              The Studio cannot tell which steps will complete while the run is being cancelled, so this is always asked.
            </span>
          ) : null}
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

      {error !== undefined ? <ErrorState error={resetRefusal(error)} what="Resetting the run" /> : null}

      <div className="dialog-actions">
        {/* "Cancel" beside a button that cancels the run would be read as the run being cancelled */}
        <Button onClick={onClose}>{cancelSource ? 'Do not cancel the run' : 'Cancel'}</Button>
        <Button type="submit" variant="primary" disabled={sending} aria-disabled={plan === undefined ? true : undefined}>
          {sending ? 'Sending…' : cancelSource ? 'Confirm: cancel the run and reset it when it ends' : 'Confirm: reset the run'}
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

/** What the person is told when the reset is not a new run yet (the run is being cancelled), or never will be (it failed). */
function ResetNotMade({ made, onClose }: { made: Reset; onClose: () => void }) {
  return (
    <div className="rn-decision">
      {made.state === 'FAILED' ? (
        <p>The reset could not be made: {failureWords(made.failureReason)}.</p>
      ) : (
        <>
          <p>The run is being cancelled.</p>
          <p>The new run is made once it has ended; you will find it under Resets made from this run. If the reset can no longer be made by then, or the run has not ended within the time the platform waits for it, that list says why.</p>
        </>
      )}
      <div className="dialog-actions">
        <Button variant="primary" onClick={onClose}>
          Close
        </Button>
      </div>
    </div>
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

/**
 * The resets made from a run, each with a link to the run it made; one that waits for this run to end, or that failed, says so.
 * The engine makes a waiting reset when its run has ended and nothing tells this page, so the list is read again when the
 * window is focused again (whatever the age of the last read) and after a reset is made from the page. It is not read on a timer.
 */
export function ResetsMade({ runId }: { runId: string }) {
  const { ep } = useApp();
  const allowed = useCan('run:read');
  const namespace = useNamespace();
  const query = useQuery({
    queryKey: ['run', runId, 'resets', namespace ?? ''],
    queryFn: ({ signal }) => ep.runResets(runId, signal),
    enabled: allowed,
    refetchOnWindowFocus: 'always',
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
              ({stateWords(reset)}
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
