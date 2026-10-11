/** How a status looks. Always a word and a shape, and a tone for colour: never colour alone. */
export interface StatusView {
  label: string;
  symbol: string;
  tone: 'idle' | 'running' | 'waiting' | 'ok' | 'bad' | 'skip';
  /** What the status means, for people who do not know the vocabulary. */
  help: string;
}

const NODE: Record<string, StatusView> = {
  NOT_STARTED: { label: 'Not started', symbol: '○', tone: 'idle', help: 'The run has not reached this step.' },
  PENDING: { label: 'Pending', symbol: '◔', tone: 'idle', help: 'Waiting for the steps before it.' },
  READY: { label: 'Ready', symbol: '◔', tone: 'idle', help: 'Ready to be taken by a worker.' },
  LEASED: { label: 'Leased', symbol: '▶', tone: 'running', help: 'A worker holds it and is about to start.' },
  RUNNING: { label: 'Running', symbol: '▶', tone: 'running', help: 'A worker is running it.' },
  WAITING: { label: 'Waiting', symbol: '⏸', tone: 'waiting', help: 'Waiting for a timer, an event, other steps or a person.' },
  RETRY_WAIT: { label: 'Retry scheduled', symbol: '↻', tone: 'waiting', help: 'It failed and will be tried again.' },
  SUCCEEDED: { label: 'Succeeded', symbol: '✓', tone: 'ok', help: 'Done.' },
  FAILED: { label: 'Failed', symbol: '✕', tone: 'bad', help: 'It failed and will not be tried again.' },
  CANCELLED: { label: 'Cancelled', symbol: '⊘', tone: 'skip', help: 'Stopped before it finished.' },
  SKIPPED: { label: 'Skipped', symbol: '⤼', tone: 'skip', help: 'The run took another branch.' },
};

const RUN: Record<string, StatusView> = {
  RUNNING: { label: 'Running', symbol: '▶', tone: 'running', help: 'Steps are being worked on.' },
  WAITING: { label: 'Waiting', symbol: '⏸', tone: 'waiting', help: 'Nothing to do until a timer, an event or a person.' },
  PAUSED: { label: 'Paused', symbol: '⏸', tone: 'waiting', help: 'An operator paused it.' },
  CANCELLING: { label: 'Cancelling', symbol: '⊘', tone: 'waiting', help: 'Stopping.' },
  COMPENSATING: { label: 'Compensating', symbol: '↶', tone: 'waiting', help: 'Undoing what the saga did.' },
  MANUAL_REVIEW: { label: 'Needs a person', symbol: '☝', tone: 'bad', help: 'A decision is needed before it can go on.' },
  SUCCEEDED: { label: 'Succeeded', symbol: '✓', tone: 'ok', help: 'Finished.' },
  FAILED: { label: 'Failed', symbol: '✕', tone: 'bad', help: 'Finished with a failure.' },
  CANCELLED: { label: 'Cancelled', symbol: '⊘', tone: 'skip', help: 'Stopped on request.' },
  TIMED_OUT: { label: 'Timed out', symbol: '⌛', tone: 'bad', help: 'It passed its deadline.' },
  COMPENSATED: { label: 'Compensated', symbol: '↶', tone: 'ok', help: 'Everything the saga did was undone.' },
  PARTIAL: { label: 'Partly undone', symbol: '◐', tone: 'bad', help: 'Some effects could not be undone.' },
  COMPENSATION_FAILED: { label: 'Compensation failed', symbol: '✕', tone: 'bad', help: 'Undoing failed.' },
};

const UNKNOWN: StatusView = { label: 'Unknown', symbol: '?', tone: 'idle', help: 'A status this version of the Studio does not know.' };

export function nodeStatus(status: string): StatusView {
  return NODE[status] ?? { ...UNKNOWN, label: clip(status) };
}

export function runStatus(status: string): StatusView {
  return RUN[status] ?? { ...UNKNOWN, label: clip(status) };
}

function clip(text: string): string {
  return text.length > 32 ? `${text.slice(0, 32)}…` : text;
}
