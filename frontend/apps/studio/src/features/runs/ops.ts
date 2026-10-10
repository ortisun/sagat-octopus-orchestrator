import { isTerminal } from '../../api/follow';
import type { OperationRequest, RuntimeNode } from '../../api/types';

export type Action = OperationRequest['action'];

/** What the page knows of the run that decides which operations make sense. */
export interface RunFacts {
  status: string;
  /** The run is paused (it can be, and still show another status, such as needing a person). */
  paused: boolean;
}

export type Permission = 'run:operate' | 'saga:override';

export interface Operation {
  action: Action;
  scope: 'run' | 'step';
  /** The words on the button. */
  label: string;
  /** Whether it can be asked for now. */
  enabled: boolean;
  /** Why not, in words, when it cannot. */
  reason?: string;
  /** The permission the platform asks for. */
  needs?: Permission;
}

const LABELS: Record<Action, string> = {
  PAUSE: 'Pause run',
  RESUME: 'Resume run',
  CANCEL: 'Cancel run',
  RETRY_STEP: 'Retry step',
  RETRY_COMPENSATION: 'Retry compensation',
  SKIP_COMPENSATION: 'Skip compensation',
  RESOLVE_STEP: 'Resolve step',
  RERUN_AS_NEW: 'Run again as new',
  MARK_MANUAL_REVIEW: 'Mark for manual review',
};

export function actionLabel(action: Action): string {
  return LABELS[action];
}

/** The decisions about one step, in the order they are offered. */
export const STEP_ACTIONS: readonly Action[] = ['RETRY_STEP', 'RETRY_COMPENSATION', 'SKIP_COMPENSATION', 'RESOLVE_STEP'];
/** What an operator can do to the run as a whole, and what the platform does not offer yet (G145). */
export const RUN_ACTIONS: readonly Action[] = ['PAUSE', 'RESUME', 'CANCEL', 'RERUN_AS_NEW', 'MARK_MANUAL_REVIEW'];

const NOT_BUILT = 'The platform does not offer this yet.';

/** The error codes that leave it unknown whether a step's effect happened; the platform asks for more to retry one (`FailureCode::leaves_outcome_unknown`). */
const OUTCOME_UNKNOWN = new Set(['LEASE_EXPIRED', 'ATTEMPT_TIMEOUT', 'OUTCOME_UNKNOWN', 'RECONCILIATION_FAILED', 'RECONCILIATION_UNKNOWN']);

export function isCompensation(id: string): boolean {
  return id.endsWith('~c');
}

function noPermission(permission: Permission): string {
  return `You need the ${permission} permission to do this.`;
}

function op(action: Action, scope: Operation['scope'], needs: Permission | undefined, refusal: string | undefined): Operation {
  return {
    action,
    scope,
    label: LABELS[action],
    enabled: refusal === undefined,
    ...(refusal === undefined ? {} : { reason: refusal }),
    ...(needs === undefined ? {} : { needs }),
  };
}

/**
 * The operations on the run as a whole, with the reason for each one that is not available. The
 * platform is the authority (it answers 403 and 409); this is what the Studio can tell in advance.
 */
export function runOperations(run: RunFacts, can: (permission: string) => boolean): Operation[] {
  const finished = isTerminal(run.status);
  const stopping = run.status === 'CANCELLING' || run.status === 'COMPENSATING';
  const refuse = (action: Action): string | undefined => {
    if (!can('run:operate')) return noPermission('run:operate');
    if (finished) return 'The run is finished.';
    if (action === 'PAUSE') {
      if (run.paused) return 'The run is already paused.';
      if (stopping) return 'The run is already being stopped.';
    }
    if (action === 'RESUME' && !run.paused) return 'The run is not paused.';
    if (action === 'CANCEL' && stopping) return 'The run is already being stopped.';
    return undefined;
  };
  return RUN_ACTIONS.map((action) => {
    if (action === 'RERUN_AS_NEW' || action === 'MARK_MANUAL_REVIEW') return op(action, 'run', undefined, NOT_BUILT);
    return op(action, 'run', 'run:operate', refuse(action));
  });
}

/**
 * The decisions about one step. Only a step that waits for a person can be decided (`waitingOn`
 * is `operator`); a compensation can be retried or skipped, any step can be resolved.
 */
export function stepOperations(node: RuntimeNode, run: RunFacts, can: (permission: string) => boolean): Operation[] {
  const finished = isTerminal(run.status);
  const waiting = node.waitingOn === 'operator';
  const undo = isCompensation(node.id);
  const refuse = (action: Action, needs: Permission): string | undefined => {
    if (!can(needs)) return noPermission(needs);
    if (finished) return 'The run is finished.';
    if (action === 'RETRY_STEP' && undo) return 'This is a compensation: use Retry compensation.';
    if (action === 'RETRY_COMPENSATION' && !undo) return 'Only a compensation (an id ending in ~c) can be retried this way: use Retry step.';
    if (action === 'SKIP_COMPENSATION' && !undo) return 'Only a compensation can be skipped.';
    if (!waiting) return 'This step is not waiting for a decision.';
    if (action === 'RETRY_STEP' && !can('saga:override') && node.failureCode !== undefined && node.failureCode !== null && OUTCOME_UNKNOWN.has(node.failureCode)) {
      return `This step may already have happened (${node.failureCode}). Retrying it needs the saga:override permission.`;
    }
    return undefined;
  };
  return STEP_ACTIONS.map((action) => {
    const needs: Permission = action === 'SKIP_COMPENSATION' || action === 'RESOLVE_STEP' ? 'saga:override' : 'run:operate';
    return op(action, 'step', needs, refuse(action, needs));
  });
}

/** What the platform will do, in the words the confirmation shows (D149). */
export function consequence(action: Action, step?: string): string {
  const which = step === undefined ? 'the step' : `step ${step}`;
  switch (action) {
    case 'PAUSE':
      return 'Pauses the run: no new step is started or handed to a worker until someone resumes it. A step a worker already holds may still finish.';
    case 'RESUME':
      return 'Resumes the run: the steps that were held back are released.';
    case 'CANCEL':
      return 'Stops the run. Completed steps of a saga are compensated according to its cancel policy; the steps still running are asked to stop.';
    case 'RETRY_STEP':
      return `Asks for another attempt of ${which}; the earlier attempts stay in the journal.`;
    case 'RETRY_COMPENSATION':
      return `Asks for another attempt at undoing ${which}; the earlier attempts stay in the journal.`;
    case 'SKIP_COMPENSATION':
      return `Gives up on undoing ${which}. The saga ends partly undone and cannot be compensated again.`;
    case 'RESOLVE_STEP':
      return `Declares that ${which} did or did not happen. This overrides what the platform knows.`;
    case 'RERUN_AS_NEW':
    case 'MARK_MANUAL_REVIEW':
      return NOT_BUILT;
  }
}

/** The step decisions that ask for the step id to be typed back (D149). */
export function asksForStepId(action: Action): boolean {
  return action === 'SKIP_COMPENSATION' || action === 'RESOLVE_STEP';
}

export const MIN_REASON = 8;
export const MAX_REASON = 2000;
/** The platform's rule for a failure code (`FailureCode::parse`). */
export const ERROR_CODE = /^[A-Za-z0-9_.:-]{1,64}$/;

export interface ReasonCheck {
  length: number;
  /** What is wrong with it, or nothing. */
  problem?: string;
}

/** The reason is counted after trimming; for the decisions about a step it must be plain text on one line. */
export function checkReason(reason: string, scope: Operation['scope']): ReasonCheck {
  const trimmed = reason.trim();
  const length = [...trimmed].length;
  if (length < MIN_REASON) return { length, problem: `Write at least ${MIN_REASON} characters: the reason is kept with the decision.` };
  if (length > MAX_REASON) return { length, problem: `At most ${MAX_REASON} characters (${length - MAX_REASON} too many).` };
  // eslint-disable-next-line no-control-regex
  if (scope === 'step' && /[\u0000-\u001f\u007f-\u009f]/.test(trimmed)) return { length, problem: 'Write the reason as plain text on one line.' };
  return { length };
}
