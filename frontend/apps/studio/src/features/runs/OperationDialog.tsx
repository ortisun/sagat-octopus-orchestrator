import { useQueryClient } from '@tanstack/react-query';
import { useRef, useState, useSyncExternalStore } from 'react';
import type { FormEvent } from 'react';
import { ApiError, isApiError } from '../../api/errors';
import type { OperationRequest, RuntimeNode } from '../../api/types';
import { useApp } from '../../app/context';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { Dialog } from '../../ui/Dialog';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import { useAnnounce } from '../../ui/Announcer';
import type { LiveRunStore } from './liveRun';
import { ERROR_CODE, MAX_REASON, actionLabel, asksForStepId, checkReason, consequence, runOperations, stepOperations } from './ops';
import type { Action, Operation, RunFacts } from './ops';
import { clip } from './time';
import { useCanOperate } from './useOperate';

/** What the person is about to decide: an action, and the step it is about (none for the run as a whole). */
export interface Decision {
  action: Action;
  stepId?: string;
  /** What had focus when it was asked for, so focus returns there. */
  opener: HTMLElement | null;
}

/** The facts a decision rests on, as the person saw them when they opened it. */
function facts(decision: Decision, node: RuntimeNode | undefined, run: RunFacts): string {
  if (decision.stepId === undefined) return `${run.status}|${run.paused}`;
  return node === undefined ? 'gone' : [node.status, node.attempt, node.generation, node.waitingOn, node.failureCode].join('|');
}

/**
 * The words the platform's refusal is shown with. A 409 can be the run having moved on since the
 * version this decision was made against (`REVISION_CONFLICT`) or the state not allowing it any
 * more (`STATE_CONFLICT`, usually another operator was first); each says which.
 */
export function refusal(error: unknown): unknown {
  if (!isApiError(error) || error.code !== 'CONFLICT') return error;
  const message =
    error.reason === 'REVISION_CONFLICT'
      ? 'The run changed while you were deciding. Refresh to see what happened, then decide again.'
      : error.reason === 'STATE_CONFLICT'
        ? `${error.message.replace(/[.\s]+$/, '')}. Another operator may have decided first, or the run moved on. Refresh to see the current state.`
        : error.message;
  return new ApiError({ code: error.code, status: error.status, message, retryable: false, traceId: error.traceId, details: error.details });
}

/** Asks for a decision to be confirmed in plain words, and sends it. */
export function OperationDialog({
  decision,
  run,
  live,
  runId,
  onClose,
}: {
  decision: Decision | undefined;
  run: RunFacts;
  live: LiveRunStore;
  runId: string;
  onClose: (opener: HTMLElement | null) => void;
}) {
  const opener = decision?.opener ?? null;
  const title = decision === undefined ? 'Decision' : `${actionLabel(decision.action)}${decision.stepId === undefined ? '' : `: ${clip(decision.stepId, 40)}`}`;
  return (
    <Dialog open={decision !== undefined} title={title} onClose={() => onClose(opener)}>
      {decision === undefined ? null : <DecisionForm decision={decision} run={run} live={live} runId={runId} onDone={() => onClose(opener)} />}
    </Dialog>
  );
}

function DecisionForm({ decision, run, live, runId, onDone }: { decision: Decision; run: RunFacts; live: LiveRunStore; runId: string; onDone: () => void }) {
  const { ep } = useApp();
  const queries = useQueryClient();
  const announce = useAnnounce();
  const can = useCanOperate();
  const snapshot = useSyncExternalStore(live.subscribe, live.getSnapshot);
  const { action, stepId } = decision;
  const scope = stepId === undefined ? 'run' : 'step';
  const node = stepId === undefined ? undefined : snapshot.model?.nodes.get(stepId);

  const [baseline, setBaseline] = useState(() => facts(decision, node, run));
  const [reason, setReason] = useState('');
  const [outcome, setOutcome] = useState<'SUCCEEDED' | 'FAILED' | ''>('');
  const [errorCode, setErrorCode] = useState('');
  const [typed, setTyped] = useState('');
  const [submitted, setSubmitted] = useState(false);
  const [sending, setSending] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<unknown>(undefined);
  // The key goes with the exact request: sending the same thing again (after a timeout) keeps it, so the platform
  // does it once; changing anything makes a new one.
  const sent = useRef<{ print: string; key: string } | undefined>(undefined);
  const reasonRef = useRef<HTMLTextAreaElement>(null);

  const current = facts(decision, node, run);
  const moved = current !== baseline;
  const operation: Operation | undefined = (stepId === undefined ? runOperations(run, can) : node === undefined ? [] : stepOperations(node, run, can)).find((o) => o.action === action);
  const available = operation?.enabled === true && !moved;

  const reasonCheck = checkReason(reason, scope);
  const reasonProblem = submitted ? reasonCheck.problem : undefined;
  const idProblem = submitted && asksForStepId(action) && typed !== stepId ? 'Type the step id exactly as shown to confirm.' : undefined;
  const outcomeProblem = submitted && action === 'RESOLVE_STEP' && outcome === '' ? 'Choose what happened to the step.' : undefined;
  const codeProblem = action === 'RESOLVE_STEP' && outcome === 'FAILED' && errorCode !== '' && !ERROR_CODE.test(errorCode) ? 'Use letters, digits and _ . : - only, up to 64 characters.' : undefined;

  const request = (): OperationRequest => ({
    action,
    ...(stepId === undefined ? {} : { stepId }),
    ...(action === 'RESOLVE_STEP' && outcome !== '' ? { outcome, ...(outcome === 'FAILED' && errorCode !== '' ? { errorCode } : {}) } : {}),
    reason: reason.trim(),
    // The run as the person saw it, brought up to date only while the facts they decided on have not changed.
    expectedRunVersion: snapshot.model?.projectionSeq ?? 0,
  });

  const send = async () => {
    const body = request();
    const print = JSON.stringify(body);
    if (sent.current?.print !== print) sent.current = { print, key: crypto.randomUUID() };
    setSending(true);
    setError(undefined);
    try {
      await ep.operate(runId, body, sent.current.key);
      announce(`${actionLabel(action)} requested${stepId === undefined ? '' : ` for ${clip(stepId, 40)}`}. The platform recorded your decision; the graph shows its effect when it happens.`);
      void queries.invalidateQueries({ queryKey: ['run', runId] });
      onDone();
    } catch (cause) {
      setError(cause);
    } finally {
      setSending(false);
    }
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    setSubmitted(true);
    if (reasonCheck.problem !== undefined) {
      reasonRef.current?.focus();
      return;
    }
    if (!available || (asksForStepId(action) && typed !== stepId) || (action === 'RESOLVE_STEP' && (outcome === '' || codeProblem !== undefined))) return;
    void send();
  };

  const refresh = async () => {
    setRefreshing(true);
    try {
      await live.refresh();
      void queries.invalidateQueries({ queryKey: ['run', runId] });
      const latest = live.getSnapshot().model;
      setBaseline(facts(decision, stepId === undefined ? undefined : latest?.nodes.get(stepId), run));
      setError(undefined);
      announce('The page is up to date. Review the decision and confirm again.');
    } finally {
      setRefreshing(false);
    }
  };

  const conflict = isApiError(error) && error.code === 'CONFLICT';

  return (
    <form onSubmit={submit} noValidate className="rn-decision">
      <p>{consequence(action, stepId === undefined ? undefined : clip(stepId, 60))}</p>
      <p className="rn-audit">This decision is recorded with your name and reason.</p>

      {moved ? (
        <Banner tone="warn" title="The run changed while you were deciding.">
          {stepId === undefined ? 'Its state is not what it was when you opened this.' : 'The step is not in the state it was in when you opened this.'} Look at the change, then confirm again.{' '}
          <Button onClick={() => setBaseline(current)}>Review the change</Button>
        </Banner>
      ) : null}
      {!moved && operation !== undefined && !operation.enabled ? (
        <Banner tone="warn" title="This is not available any more.">
          {operation.reason}
        </Banner>
      ) : null}

      {action === 'RESOLVE_STEP' ? (
        <fieldset className="rn-outcome" aria-describedby={outcomeProblem ? 'rn-outcome-error' : undefined}>
          <legend>What happened to the step?</legend>
          <label>
            <input type="radio" name="outcome" value="SUCCEEDED" checked={outcome === 'SUCCEEDED'} onChange={() => setOutcome('SUCCEEDED')} /> It happened: declare the step succeeded
          </label>
          <label>
            <input type="radio" name="outcome" value="FAILED" checked={outcome === 'FAILED'} onChange={() => setOutcome('FAILED')} /> It did not happen: declare the step failed
          </label>
          {outcomeProblem ? (
            <span id="rn-outcome-error" className="error" role="alert">
              {outcomeProblem}
            </span>
          ) : null}
        </fieldset>
      ) : null}
      {action === 'RESOLVE_STEP' && outcome === 'FAILED' ? (
        <Field label="Error code (optional)" hint="Letters, digits and _ . : - only. Left empty it is recorded as DECLARED_FAILED_BY_OPERATOR." error={codeProblem}>
          <input type="text" value={errorCode} maxLength={64} autoComplete="off" spellCheck={false} onChange={(event) => setErrorCode(event.target.value)} />
        </Field>
      ) : null}

      <Field label="Reason" hint={`8 to ${MAX_REASON} characters, kept with the decision. ${reasonCheck.length} / ${MAX_REASON}`} error={reasonProblem}>
        <textarea ref={reasonRef} value={reason} rows={4} onChange={(event) => setReason(event.target.value)} />
      </Field>

      {asksForStepId(action) && stepId !== undefined ? (
        <Field label="Type the step id to confirm" hint={`The step is ${clip(stepId, 60)}.`} error={idProblem}>
          <input type="text" value={typed} autoComplete="off" spellCheck={false} onChange={(event) => setTyped(event.target.value)} />
        </Field>
      ) : null}

      {error !== undefined ? (
        <div>
          <ErrorState error={refusal(error)} what={actionLabel(action)} {...(isApiError(error) && error.retryable && !conflict ? { onRetry: () => void send() } : {})} />
          {conflict ? (
            <Button onClick={() => void refresh()} disabled={refreshing}>
              {refreshing ? 'Refreshing…' : 'Refresh and decide again'}
            </Button>
          ) : null}
        </div>
      ) : null}

      <div className="dialog-actions">
        <Button onClick={onDone}>Cancel</Button>
        <Button type="submit" variant={action === 'CANCEL' || action === 'SKIP_COMPENSATION' || action === 'RESOLVE_STEP' ? 'danger' : 'primary'} disabled={sending} aria-disabled={available ? undefined : true}>
          {sending ? 'Sending…' : `Confirm: ${actionLabel(action)}`}
        </Button>
      </div>
    </form>
  );
}
