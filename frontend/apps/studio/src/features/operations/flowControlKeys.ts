import type { ApiClient } from '../../api/client';
import { path } from '../../api/client';
import type { components } from '../../api/schema';

/**
 * The flow-control keys of a namespace (feature 022): the live concurrency keys (FR-022-022) and the rate-limit keys
 * (FR-022-048), oldest wait first, by key label only, to a caller who may reveal personal data in the namespace; one row
 * per group to every other caller (`view: GROUPS`, decision D022-086), each group read whole on one page and never across
 * pages (decision D022-131: `spansPages` is always false, deprecated, and not shown). The cursor is opaque and sealed by
 * the platform: it is sent back as it came and never read.
 */

type Schemas = components['schemas'];

export type ConcurrencyKey = Schemas['ConcurrencyKey'];
export type ConcurrencyKeyGroup = Schemas['ConcurrencyKeyGroup'];
export type ConcurrencyKeyPage = Schemas['ConcurrencyKeyPage'];
export type RateLimitKey = Schemas['RateLimitKey'];
export type RateLimitKeyGroup = Schemas['RateLimitKeyGroup'];
export type RateLimitKeyPage = Schemas['RateLimitKeyPage'];

export interface KeyQuery {
  group?: string;
  cursor?: string;
  limit?: number;
}

export function flowControlKeysApi(client: ApiClient, namespaceId: string) {
  const base = path`/namespaces/${namespaceId}`;
  return {
    concurrencyKeys: (query: KeyQuery = {}, signal?: AbortSignal) => client.get<ConcurrencyKeyPage>(`${base}/concurrency-keys`, { query: { ...query }, signal }),
    rateLimitKeys: (query: KeyQuery = {}, signal?: AbortSignal) => client.get<RateLimitKeyPage>(`${base}/rate-limit-keys`, { query: { ...query }, signal }),
  };
}

export type FlowControlKeysApi = ReturnType<typeof flowControlKeysApi>;

const MAX_VALUE = 40;

function shown(value: unknown): string {
  const text = typeof value === 'string' ? value : (JSON.stringify(value) ?? '');
  return text.length > MAX_VALUE ? `${text.slice(0, MAX_VALUE)}…` : text;
}

/** A `null` label is one the platform withheld from this caller (FR-022-057); the reason is said once beside the table. */
export function keyLabelWords(label: unknown): string {
  if (label === null) return 'Hidden';
  // a label the platform did not send at all breaks the contract: shown as unsupported, as the run view does (FR-007-010)
  return typeof label === 'string' ? shown(label) : 'Unsupported (no label sent)';
}

const SCOPES: Record<string, string> = { RUN: 'Run starts', STEP: 'Step attempts' };
const POOLS: Record<string, string> = { ORDINARY: 'Ordinary', DRAFT_TEST: 'Draft test' };

export function scopeWords(scope: unknown): string {
  return (typeof scope === 'string' ? SCOPES[scope] : undefined) ?? `Unsupported (${shown(scope)})`;
}

export function poolWords(pool: unknown): string {
  return (typeof pool === 'string' ? POOLS[pool] : undefined) ?? `Unsupported (${shown(pool)})`;
}

/** The namespace's queued runs, counted as the `queued_runs` ceiling counts them. */
export function queuedWords(queued: number): string {
  if (queued === 0) return 'No run of this namespace is queued.';
  return `${queued} ${queued === 1 ? 'run of this namespace is' : 'runs of this namespace are'} queued (waiting for a permit or a slot, each run counted once).`;
}

/** Why a rate-limit key, or group, has no instant for its next slot, in the contract's words. */
export function noNextSlotWords(key: { waiters: number; unleasedGrants: number }): string {
  if (key.waiters === 0) return 'Nobody waits';
  if (key.unleasedGrants > 0) return 'Not by time: a granted step frees it when a worker leases it or it is given back';
  return 'There is room: the next grant pass grants';
}
