import type { BatchCreateRequest, BatchJob, BatchSelector } from '../../api/endpoints';
import type { StatusView } from '../../model/status';

/**
 * What a batch job can do. `UPDATE` and `REPIN` are not offered: the platform refuses them while a
 * declared update cannot be applied to many runs and while worker deployments do not exist.
 */
export const BATCH_ACTIONS = ['CANCEL', 'PAUSE', 'RESUME', 'RERUN', 'SIGNAL', 'RETRY_STEP', 'MARK_MANUAL_REVIEW'] as const;
export type BatchActionName = (typeof BATCH_ACTIONS)[number];

export interface ActionText {
  label: string;
  /** What it does to each run, in the words of the consequence a person confirms. */
  effect: string;
}

export const ACTION_TEXT: Record<BatchActionName, ActionText> = {
  CANCEL: { label: 'Cancel', effect: 'asks each run to stop. A run that compensates is compensated by the run; the batch undoes nothing.' },
  PAUSE: { label: 'Pause', effect: 'pauses each run that is not over. Runs that are over are skipped.' },
  RESUME: { label: 'Resume', effect: 'resumes each paused run. Runs that are not paused are skipped.' },
  RERUN: { label: 'Rerun', effect: 'starts a new run from the input of each run that has ended. Runs that have not ended are skipped.' },
  SIGNAL: { label: 'Send a signal', effect: 'sends the signal to each run with a step waiting for it. Signals are never kept for later: other runs are skipped.' },
  RETRY_STEP: { label: 'Retry a step', effect: 'asks for another attempt of the named step in each run where it waits for a person. Steps whose outcome is unknown are never repeated blindly: those items fail.' },
  MARK_MANUAL_REVIEW: { label: 'Hold a step for review', effect: 'holds the named step, in each run where it has not started, for a person to decide.' },
};

export const MIN_REASON = 8;
export const MAX_REASON = 2000;
export const MAX_PAYLOAD_BYTES = 256 * 1024;
const NAME = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/;
const STEP = /^[A-Za-z0-9_][A-Za-z0-9_.:~-]{0,199}$/;

/** The filters of the run list, as the page holds them. */
export interface RunFilters {
  status: string;
  workflowId: string;
  origin: string;
}

/** The selector of a batch job made from the filters of the run list: what the person sees is what the job takes. */
export function selectorOf(filters: RunFilters): BatchSelector {
  const filter: Record<string, unknown> = {};
  if (filters.status !== '') filter['status'] = [filters.status];
  if (filters.workflowId !== '') filter['workflowId'] = filters.workflowId;
  if (filters.origin !== '') filter['origin'] = filters.origin;
  return { filter };
}

/** The filters in words, for the confirmation. */
export function describeFilters(filters: RunFilters, workflowName?: string): string {
  const parts: string[] = [];
  if (filters.status !== '') parts.push(`status ${filters.status.toLowerCase().replace(/_/g, ' ')}`);
  if (filters.workflowId !== '') parts.push(`workflow ${workflowName ?? filters.workflowId}`);
  if (filters.origin !== '') parts.push(`origin ${filters.origin.toLowerCase().replace(/_/g, ' ')}`);
  return parts.length === 0 ? 'every run of the namespace' : `the runs with ${parts.join(' and ')}`;
}

export interface Draft {
  action: BatchActionName;
  signalName: string;
  payload: string;
  stepId: string;
  maxItems: string;
  rate: string;
}

export const EMPTY_DRAFT: Draft = { action: 'CANCEL', signalName: '', payload: '{}', stepId: '', maxItems: '', rate: '' };

/** The reason of a decision, trimmed, 8 to 2,000 characters. */
export function reasonProblem(reason: string): string | undefined {
  const length = [...reason.trim()].length;
  if (length < MIN_REASON) return `Give a reason of at least ${MIN_REASON} characters.`;
  if (length > MAX_REASON) return `Keep the reason to ${MAX_REASON} characters or fewer.`;
  return undefined;
}

/** `maxItems` as a number, or why it is not one. An empty field leaves the platform's default. */
export function maxItemsOf(text: string): { value?: number } | { problem: string } {
  const trimmed = text.trim();
  if (trimmed === '') return {};
  if (!/^\d+$/.test(trimmed)) return { problem: 'Use a whole number of runs for the most items, from 1 to 100,000.' };
  const value = Number(trimmed);
  if (value < 1 || value > 100_000) return { problem: 'Use a whole number of runs for the most items, from 1 to 100,000.' };
  return { value };
}

function rateOf(text: string): { value?: number } | { problem: string } {
  const trimmed = text.trim();
  if (trimmed === '') return {};
  const value = Number(trimmed);
  if (!Number.isFinite(value) || value <= 0 || value > 200) return { problem: 'Use a rate above 0 and at most 200 runs a second.' };
  return { value };
}

/** The request a draft makes, or the first thing wrong with it (the platform checks it again). */
export function requestOf(draft: Draft, selector: BatchSelector, reason: string): { body: BatchCreateRequest } | { problem: string } {
  const why = reasonProblem(reason);
  if (why) return { problem: why };
  const max = maxItemsOf(draft.maxItems);
  if ('problem' in max) return max;
  const rate = rateOf(draft.rate);
  if ('problem' in rate) return rate;
  const body: BatchCreateRequest = { selector, action: draft.action, reason: reason.trim() };
  if (max.value !== undefined) body.maxItems = max.value;
  if (rate.value !== undefined) body.itemsPerSecond = rate.value;
  if (draft.action === 'SIGNAL') {
    if (!NAME.test(draft.signalName)) return { problem: 'Use a signal name of 1–128 characters, starting with a letter or digit; then letters, digits, or _ . : - .' };
    let payload: unknown;
    try {
      payload = JSON.parse(draft.payload);
    } catch {
      return { problem: 'Enter a valid JSON object.' };
    }
    if (payload === null || typeof payload !== 'object' || Array.isArray(payload)) return { problem: 'Enter a JSON object.' };
    if (new TextEncoder().encode(JSON.stringify(payload)).length > MAX_PAYLOAD_BYTES) return { problem: 'The payload must be at most 256 KiB.' };
    body.signal = { name: draft.signalName, payload: payload as Record<string, unknown> };
  }
  if (draft.action === 'RETRY_STEP' || draft.action === 'MARK_MANUAL_REVIEW') {
    if (!STEP.test(draft.stepId)) return { problem: 'Name the step: letters, digits, or _ . : ~ - , without spaces.' };
    body.stepId = draft.stepId;
  }
  return { body };
}

// --- progress and words ----------------------------------------------------------------------

export const ACTIVE_STATES = ['SELECTING', 'RUNNING', 'CANCELLING'] as const;

export function isActive(state: string | undefined): boolean {
  return (ACTIVE_STATES as readonly string[]).includes(state ?? '');
}

/** How far a job is: the items that have an outcome of the items selected so far. */
export function progress(job: Pick<BatchJob, 'counts' | 'state'>): { done: number; total: number; percent: number } {
  const { selected, succeeded, skipped, failed, cancelled } = job.counts;
  const done = succeeded + skipped + failed + cancelled;
  if (selected === 0) return { done, total: 0, percent: isActive(job.state) ? 0 : 100 };
  return { done, total: selected, percent: Math.min(100, Math.floor((100 * done) / selected)) };
}

const JOB: Record<string, StatusView> = {
  SELECTING: { label: 'Selecting', symbol: '◔', tone: 'waiting', help: 'Choosing the runs the job will take.' },
  RUNNING: { label: 'Running', symbol: '▶', tone: 'running', help: 'Applying the action, one run at a time.' },
  CANCELLING: { label: 'Cancelling', symbol: '⊘', tone: 'waiting', help: 'Stopping after the run in hand.' },
  COMPLETED: { label: 'Completed', symbol: '✓', tone: 'ok', help: 'Every run has an outcome.' },
  CANCELLED: { label: 'Cancelled', symbol: '⊘', tone: 'skip', help: 'Stopped on request; what was applied stays applied.' },
  FAILED: { label: 'Failed', symbol: '✕', tone: 'bad', help: 'The job stopped because something it needs is gone.' },
};

const ITEM: Record<string, StatusView> = {
  PENDING: { label: 'Waiting', symbol: '○', tone: 'idle', help: 'Not applied yet.' },
  DEFERRED: { label: 'Deferred', symbol: '↻', tone: 'waiting', help: 'Refused for now; it will be tried again.' },
  SUCCEEDED: { label: 'Succeeded', symbol: '✓', tone: 'ok', help: 'The action was applied.' },
  SKIPPED: { label: 'Skipped', symbol: '⤼', tone: 'skip', help: 'The action did not apply to the run as it was.' },
  FAILED: { label: 'Failed', symbol: '✕', tone: 'bad', help: 'The action could not be applied.' },
  CANCELLED: { label: 'Cancelled', symbol: '⊘', tone: 'skip', help: 'The job was cancelled before this run.' },
};

const UNKNOWN: StatusView = { label: 'Unknown', symbol: '?', tone: 'idle', help: 'A state this version of the Studio does not know.' };

export const jobState = (state: string): StatusView => JOB[state] ?? UNKNOWN;
export const itemState = (state: string): StatusView => ITEM[state] ?? UNKNOWN;
export const ITEM_STATES = Object.keys(ITEM);
export const JOB_STATES = Object.keys(JOB);

const REASON: Record<string, string> = {
  ALREADY_TERMINAL: 'The run had already ended.',
  NOT_APPLICABLE: 'The run was not in a state this action applies to.',
  NOT_WAITING: 'No step of the run was waiting for this.',
  RUN_CONTINUED: 'The run had not ended.',
  HIDDEN_FROM_CREATOR: 'The run is not visible to the person who started the job.',
  OPERATION_NOT_DECLARED: 'The workflow does not declare this operation.',
  TEST_RUN: 'A test run is never changed by a batch.',
  QUOTA_EXCEEDED: 'A limit was reached; it will be tried again.',
  RATE_LIMITED: 'The platform asked for a pause; it will be tried again.',
  CONCURRENCY_QUEUE_FULL: 'The run is queued behind others; it will be tried again.',
  LOCK_WAIT: 'The run was busy; it will be tried again.',
  RETRIES_EXHAUSTED: 'It was tried again too often.',
};

/** A closed reason code in words; a code this version does not know is shown as it is. */
export function reasonText(code: string | null | undefined): string {
  if (code === null || code === undefined || code === '') return '';
  return REASON[code] ?? code;
}

const FAILURE: Record<string, string> = {
  AUTHORITY_REVOKED: 'The person who started the job lost a permission it needs, so it stopped.',
  SELECTION_FAILED: 'The platform could not select the runs.',
};

export function failureText(code: string | null | undefined): string {
  if (code === null || code === undefined) return '';
  return FAILURE[code] ?? code;
}
