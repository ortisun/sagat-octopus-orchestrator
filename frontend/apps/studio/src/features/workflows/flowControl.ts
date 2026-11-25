import { setField } from '../../model/edit';
import { findNode } from '../../model/ir';
import type { IrDocument, IrNode } from '../../model/ir';

/**
 * The flow-control gates of feature 022 as the Studio edits them: `concurrency` (at most `limit` holders of one key at
 * once, US02) and `rateLimit` (at most `limit` grants of one key in any window of `per`, US07), declared at the top level
 * (run starts) or on an `activity` or `script` node (that step's attempts). The shapes are `concurrencyPolicy` and
 * `rateLimitPolicy` of `specs/001-platform-foundation/contracts/workflow.schema.json`.
 *
 * The bounds below are the platform's own (`crates/domain/src/workflow/flow_control.rs` and
 * `crates/infra-ir/src/flow_control.rs`): the Studio says beside a field what publication would refuse, and nothing
 * else (decision D022-150 item 2). What a key may read is checked by the platform's validation.
 */

export type Gate = 'concurrency' | 'rateLimit';
export type Member = 'group' | 'key' | 'limit' | 'per' | 'queueTimeoutMs';
export type Owner = { level: 'workflow' } | { level: 'node'; nodeId: string };

export const WORKFLOW: Owner = { level: 'workflow' };

/** The node kinds that may declare a gate (a root node; the Studio edits root nodes only). */
export const GATED_NODE_TYPES: readonly string[] = ['activity', 'script'];

export const RATE_PERIODS = ['second', 'minute', 'hour'] as const;
export type RatePeriod = (typeof RATE_PERIODS)[number];

/** `MAX_CONCURRENCY_LIMIT`. */
export const MAX_CONCURRENCY_LIMIT = 10_000;
/** `RatePeriod::max_limit`. */
export const RATE_CEILING: Record<RatePeriod, number> = { second: 100, minute: 6_000, hour: 10_000 };
/** `MAX_QUEUE_TIMEOUT`, in milliseconds. */
export const MAX_QUEUE_TIMEOUT_MS = 86_400_000;
/** `MAX_KEY_EXPRESSION_BYTES`: the bounded evaluator's source limit. */
export const MAX_KEY_EXPRESSION_BYTES = 1024;
/** The pattern of an explicit group (`GroupName::parse`). */
export const GROUP_NAME = /^[a-z][a-z0-9.-]{0,63}$/;

/** The members of a policy in the canonical order of the IR: alphabetical, as the fixtures and the SDKs write them. */
const MEMBERS: Record<Gate, readonly Member[]> = {
  concurrency: ['group', 'key', 'limit', 'queueTimeoutMs'],
  rateLimit: ['group', 'key', 'limit', 'per', 'queueTimeoutMs'],
};

const WINDOW_WORDS: Record<RatePeriod, string> = { second: 'a second', minute: 'a minute', hour: 'an hour' };

export function windowWords(per: RatePeriod): string {
  return WINDOW_WORDS[per];
}

export function isRatePeriod(value: unknown): value is RatePeriod {
  return RATE_PERIODS.some((p) => p === value);
}

export function mayDeclareGates(node: IrNode): boolean {
  return GATED_NODE_TYPES.includes(node.type);
}

function isObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

export type PolicyView =
  | { state: 'absent' }
  | { state: 'policy'; policy: Record<string, unknown>; unknownMembers: string[] }
  | { state: 'unreadable' };

/** What a gate's value is: none, a policy (with the members this Studio does not know named), or something else. */
export function readPolicy(value: unknown): PolicyView {
  if (value === undefined) return { state: 'absent' };
  if (!isObject(value)) return { state: 'unreadable' };
  const known = new Set<string>(MEMBERS.rateLimit);
  return { state: 'policy', policy: value, unknownMembers: Object.keys(value).filter((member) => !known.has(member)) };
}

/** The known members of a gate first, in the canonical order, then the others untouched in their own order. */
export function ordered(gate: Gate, policy: Record<string, unknown>): Record<string, unknown> {
  const known = MEMBERS[gate];
  const out: Record<string, unknown> = {};
  for (const member of known) if (policy[member] !== undefined) out[member] = policy[member];
  for (const [member, value] of Object.entries(policy)) if (!(known as readonly string[]).includes(member) && value !== undefined) out[member] = value;
  return out;
}

export function policyOf(doc: IrDocument, owner: Owner, gate: Gate): unknown {
  return owner.level === 'workflow' ? doc[gate] : findNode(doc, owner.nodeId)?.[gate];
}

function withPolicy(doc: IrDocument, owner: Owner, gate: Gate, value: unknown): IrDocument {
  if (owner.level === 'node') return setField(doc, owner.nodeId, gate, value);
  const rest = Object.fromEntries(Object.entries(doc).filter(([key]) => key !== gate)) as IrDocument;
  return value === undefined ? rest : { ...rest, [gate]: value };
}

/** Turns a gate on with an empty policy for the author to complete: no member is defaulted (decision D022-150 item 1). */
export function addPolicy(doc: IrDocument, owner: Owner, gate: Gate): IrDocument {
  return policyOf(doc, owner, gate) === undefined ? withPolicy(doc, owner, gate, {}) : doc;
}

export function removePolicy(doc: IrDocument, owner: Owner, gate: Gate): IrDocument {
  return withPolicy(doc, owner, gate, undefined);
}

/** Sets or clears (`undefined` or an empty text) one member; a policy that is not an object is left as it is. */
export function setPolicyField(doc: IrDocument, owner: Owner, gate: Gate, member: Member, value: unknown): IrDocument {
  const current = policyOf(doc, owner, gate);
  if (current !== undefined && !isObject(current)) return doc;
  const next = { ...(current ?? {}), [member]: value === '' ? undefined : value };
  return withPolicy(doc, owner, gate, ordered(gate, next));
}

export interface PolicyProblem {
  member: Member;
  text: string;
}

const isWhole = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value);

/** The most a limit may be: 10,000 for a concurrency limit, the window's ceiling for a rate limit (the largest without one). */
export function limitCeiling(gate: Gate, per: unknown): number {
  if (gate === 'concurrency') return MAX_CONCURRENCY_LIMIT;
  return isRatePeriod(per) ? RATE_CEILING[per] : Math.max(...Object.values(RATE_CEILING));
}

export function limitRangeWords(gate: Gate, per: unknown): string {
  const ceiling = limitCeiling(gate, per);
  return gate === 'rateLimit' && isRatePeriod(per) ? `from 1 to ${ceiling} (the most ${windowWords(per)} allows)` : `from 1 to ${ceiling}`;
}

const GROUP_WORDS = 'A group starts with a lowercase letter and holds lowercase letters, digits, dots or dashes (up to 64).';
const TIMEOUT_WORDS = `A queue timeout is from 1 ms to one day (${MAX_QUEUE_TIMEOUT_MS} ms).`;
const KEY_WORDS = `A key is an expression of 1 to ${MAX_KEY_EXPRESSION_BYTES} bytes.`;

/** What publication would refuse in a policy, member by member, in words. */
export function policyProblems(gate: Gate, policy: Record<string, unknown>): PolicyProblem[] {
  const problems: PolicyProblem[] = [];
  const per = policy['per'];
  const limit = policy['limit'];
  if (limit === undefined) {
    problems.push({ member: 'limit', text: gate === 'concurrency' ? 'Enter the most holders at once.' : 'Enter the most grants per window.' });
  } else if (!isWhole(limit) || limit < 1 || limit > limitCeiling(gate, per)) {
    problems.push({ member: 'limit', text: `Enter a whole number ${limitRangeWords(gate, per)}.` });
  }
  if (gate === 'rateLimit') {
    if (per === undefined) problems.push({ member: 'per', text: 'Choose a window: a second, a minute or an hour.' });
    else if (!isRatePeriod(per)) problems.push({ member: 'per', text: `The window "${clip(valueText(per), 20)}" is not one the platform accepts: choose a second, a minute or an hour.` });
  }
  const key = policy['key'];
  if (key !== undefined) {
    const bytes = typeof key === 'string' ? new TextEncoder().encode(key).length : 0;
    if (bytes < 1 || bytes > MAX_KEY_EXPRESSION_BYTES) problems.push({ member: 'key', text: KEY_WORDS });
  }
  const group = policy['group'];
  if (group !== undefined && (typeof group !== 'string' || !GROUP_NAME.test(group))) problems.push({ member: 'group', text: GROUP_WORDS });
  const timeout = policy['queueTimeoutMs'];
  if (timeout !== undefined && (!isWhole(timeout) || timeout < 1 || timeout > MAX_QUEUE_TIMEOUT_MS)) problems.push({ member: 'queueTimeoutMs', text: TIMEOUT_WORDS });
  return problems;
}

/** A value of a document as text, whatever it is (a newer platform's or a hand-written one). */
export function valueText(value: unknown): string {
  if (typeof value === 'string') return value;
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  return JSON.stringify(value) ?? '';
}

function clip(text: string, max: number): string {
  return text.length <= max ? text : `${text.slice(0, max - 1)}…`;
}
