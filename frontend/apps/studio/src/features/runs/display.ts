import type { Run, RuntimeEdge, RuntimeNode } from '../../api/types';
import { kindOf } from '../../model/registry';
import { nodeStatus } from '../../model/status';
import { clip } from './time';

export interface KindView {
  /** "Step", "Condition", "Undo of pay", "Lookup for pay". */
  label: string;
  /** Forward work, the compensation the engine derived (`x~c`), or the lookup of whether an effect happened (`x~r`). */
  variant: 'forward' | 'undo' | 'lookup';
  /** The step an undo or a lookup is about. */
  forward?: string;
}

const MAX_ID = 80;

/** What a runtime node is, for a person. A node id may be hostile text: it is clipped, never interpreted. */
export function kindView(node: RuntimeNode): KindView {
  const undo = node.kind === 'COMPENSATION' || node.id.endsWith('~c');
  const lookup = node.kind === 'RECONCILIATION' || node.id.endsWith('~r');
  if (undo || lookup) {
    const forward = clip(node.forward ?? node.id.replace(/~[cr]$/, ''), MAX_ID);
    return undo ? { label: `Undo of ${forward}`, variant: 'undo', forward } : { label: `Lookup for ${forward}`, variant: 'lookup', forward };
  }
  return { label: kindOf(node.type)?.label ?? clip(node.type, 40), variant: 'forward' };
}

const WAITING: Record<string, string> = {
  operator: 'Waiting for a person',
  timer: 'Waiting for a timer',
  event: 'Waiting for an event',
  branches: 'Waiting for the branches',
  child: 'Waiting for the child run',
  saga: 'Waiting for the saga',
  reconciliation: 'Waiting for the lookup of whether it happened',
};

export function waitingWords(node: RuntimeNode): string | undefined {
  return node.waitingOn === null || node.waitingOn === undefined ? undefined : (WAITING[node.waitingOn] ?? 'Waiting');
}

export function needsAPerson(node: RuntimeNode): boolean {
  return node.waitingOn === 'operator';
}

/** Failed, waiting for a person, or about to be retried: what the table puts first and filters on (`needsAttention`). */
export function isAttention(node: RuntimeNode): boolean {
  return node.status === 'FAILED' || node.waitingOn === 'operator' || node.status === 'RETRY_WAIT';
}

/** The attempt number to show: nothing for the first attempt, which is the usual case. */
export function attemptWords(node: RuntimeNode): string | undefined {
  return node.attempt !== undefined && node.attempt > 1 ? `attempt ${node.attempt}` : undefined;
}

/** One line for assistive technology: the card's name. */
export function nodeSummary(node: RuntimeNode): string {
  const parts = [clip(node.id, MAX_ID), kindView(node).label, nodeStatus(node.status).label];
  const attempt = attemptWords(node);
  if (attempt) parts.push(attempt);
  if (node.failureCode) parts.push(`failure ${clip(node.failureCode, 40)}`);
  const waiting = waitingWords(node);
  if (waiting) parts.push(waiting.toLowerCase());
  return parts.join(', ');
}

const EDGE_WORDS: Record<RuntimeEdge['kind'], string> = {
  next: 'Next',
  error: 'On error',
  true: 'If true',
  false: 'If false',
  branch: 'Branch',
  timeout: 'On timeout',
  compensation: 'Undo after',
  reconcile: 'Lookup',
};

export function edgeKindWords(kind: RuntimeEdge['kind']): string {
  return EDGE_WORDS[kind] ?? clip(String(kind), 24);
}

const HEX = /^[0-9a-f]{16,128}$/i;
const MAX_REF = 200;

/**
 * How a `payloadRef` is shown: the platform keeps the payload and the journal carries only its
 * digest, so the Studio says "redacted" and shows a short form of the digest, with the whole of
 * it available to copy (G144). It never shows or fetches the payload.
 */
export function digestView(ref: string | null | undefined): { short: string; full: string } | undefined {
  if (ref === null || ref === undefined || ref === '') return undefined;
  const full = ref.slice(0, MAX_REF);
  const hex = full.replace(/^sha256:/i, '');
  return { short: HEX.test(hex) ? `sha256:${hex.slice(0, 12)}…` : clip(full, 24), full };
}

/** Every status a run can have, in the order a person reads them (a status missing here is a compile error). */
const ALL_RUN_STATUSES: Record<Run['status'], true> = {
  RUNNING: true,
  WAITING: true,
  PAUSED: true,
  CANCELLING: true,
  COMPENSATING: true,
  MANUAL_REVIEW: true,
  SUCCEEDED: true,
  FAILED: true,
  CANCELLED: true,
  TIMED_OUT: true,
  COMPENSATED: true,
  PARTIAL: true,
  COMPENSATION_FAILED: true,
};
export const RUN_STATUSES = Object.keys(ALL_RUN_STATUSES) as Run['status'][];
