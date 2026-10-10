/** How a journal event is told to a person. The names are the platform's (`RunEvent::name`). */
const LABELS: Record<string, string> = {
  'run.started': 'The run started',
  'run.paused': 'An operator paused the run',
  'run.resumed': 'An operator resumed the run',
  'run.stop_requested': 'The run was asked to stop',
  'run.finished': 'The run finished',
  'run.signalled': 'A signal reached the run',
  'step.routed': 'The run moved on from the step',
  'step.released': 'The step is ready for a worker',
  'step.leased': 'A worker took the step',
  'step.started': 'A worker started the step',
  'step.completed': 'The step completed',
  'step.retry_scheduled': 'The step failed and a retry was scheduled',
  'step.retry_due': 'The retry is due',
  'step.failed': 'The step failed',
  'step.cancel_requested': 'The step was asked to stop',
  'step.cancelled': 'The step was cancelled',
  'step.needs_review': 'The step needs a person to decide',
  'step.retry_requested': 'An operator asked for another attempt',
  'step.skipped': 'An operator gave up on the step',
  'step.resolved': 'An operator settled the step',
  'condition.evaluated': 'The condition was evaluated',
  'timer.started': 'The timer started',
  'timer.fired': 'The timer fired',
  'wait.started': 'The run started waiting for an event',
  'wait.satisfied': 'The awaited event arrived',
  'wait.timed_out': 'The wait ran out of time',
  'parallel.forked': 'The branches started',
  'parallel.joined': 'The branches joined',
  'child.started': 'A child run started',
  'child.finished': 'The child run finished',
  'end.reached': 'The run reached its end',
  'saga.entered': 'The saga began',
  'saga.completed': 'The saga completed',
  'saga.aborting': 'The saga is being undone',
  'compensation.planned': 'The undo steps were planned',
  'saga.compensated': 'The saga finished undoing',
  'reconciliation.requested': 'The platform asked whether the effect happened',
  'reconciliation.concluded': 'The platform found out whether the effect happened',
};

export function eventLabel(type: string): string {
  return LABELS[type] ?? type;
}

/** The kind of a journal event for filters and for colour: what a reader should look at first. */
export type EventTone = 'normal' | 'problem' | 'decision';

export function eventTone(type: string): EventTone {
  if (type === 'step.failed' || type === 'step.retry_scheduled' || type === 'step.needs_review' || type === 'wait.timed_out' || type === 'saga.aborting') return 'problem';
  if (type === 'step.retry_requested' || type === 'step.skipped' || type === 'step.resolved' || type === 'run.paused' || type === 'run.resumed' || type === 'run.stop_requested') return 'decision';
  return 'normal';
}
