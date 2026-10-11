import type { ApiClient } from './client';
import { path } from './client';
import { ApiError, isApiError } from './errors';
import { readSse } from './sse';
import type { GraphChanges } from './types';

/** Where a followed run is: how the changes arrive right now. */
export type FollowState =
  /** Opening the stream. */
  | 'connecting'
  /** Changes arrive as they happen. */
  | 'live'
  /** The stream is not available; the changes are asked for every few seconds. */
  | 'polling'
  /** The stream broke; it is being opened again. */
  | 'reconnecting'
  /** The run is over and everything it did has been delivered. */
  | 'ended'
  /** The token is not valid any more. */
  | 'unauthorized'
  /** The run cannot be followed (it does not exist, or the platform refuses). */
  | 'failed';

export interface FollowOptions {
  client: ApiClient;
  runId: string;
  /** The projection sequence the caller already has; absent, the first change is the whole graph. */
  since?: number | undefined;
  onChanges: (changes: GraphChanges) => void;
  onState: (state: FollowState, error?: ApiError) => void;
  /** How often to ask for changes while the stream is not available. */
  pollIntervalMs?: number;
  /** Stream failures in a row before giving up on the stream for a while. */
  streamFailuresBeforePolling?: number;
  /** How long to poll before trying the stream again. */
  retryStreamAfterMs?: number;
  /** For tests. */
  random?: () => number;
}

export interface Follow {
  close(): void;
}

const MIN_BACKOFF_MS = 500;
const MAX_BACKOFF_MS = 15_000;

/** A value no run has reached: asking for the changes after it gives the whole graph. */
const BEYOND = Number.MAX_SAFE_INTEGER;

function isChanges(value: unknown): value is GraphChanges {
  if (value === null || typeof value !== 'object') return false;
  const v = value as Record<string, unknown>;
  return typeof v['runId'] === 'string' && typeof v['projectionSeq'] === 'number' && typeof v['status'] === 'string' && Array.isArray(v['nodes']) && Array.isArray(v['events']) && typeof v['reset'] === 'boolean';
}

/**
 * Follows a run (D141, D142): the stream while it works, resuming from the last sequence it
 * delivered, and the same changes by polling while it does not. Nothing is lost between the two
 * because both are asked for "everything after sequence N". The caller applies each change to the
 * graph it holds; a `reset` replaces it.
 */
export function followRun(options: FollowOptions): Follow {
  const { client, runId, onChanges, onState } = options;
  const pollEvery = options.pollIntervalMs ?? 2000;
  const failuresBeforePolling = options.streamFailuresBeforePolling ?? 3;
  const retryStreamAfter = options.retryStreamAfterMs ?? 60_000;
  const random = options.random ?? Math.random;
  const stop = new AbortController();
  let cursor = options.since;
  let state: FollowState | undefined;

  const set = (next: FollowState, error?: ApiError) => {
    if (stop.signal.aborted) return;
    if (state === next && error === undefined) return;
    state = next;
    onState(next, error);
  };

  const deliver = (changes: GraphChanges) => {
    if (stop.signal.aborted) return;
    cursor = changes.projectionSeq;
    onChanges(changes);
  };

  const sleep = (ms: number) =>
    new Promise<void>((resolve) => {
      const timer = setTimeout(done, ms);
      function done() {
        clearTimeout(timer);
        stop.signal.removeEventListener('abort', done);
        resolve();
      }
      stop.signal.addEventListener('abort', done, { once: true });
    });

  /** One connection. Returns how it ended. */
  async function stream(): Promise<'end' | 'reconnect' | 'dropped' | 'dropped-empty' | 'closed' | 'unsupported'> {
    const headers: Record<string, string> = { Accept: 'text/event-stream' };
    if (cursor !== undefined) headers['Last-Event-ID'] = String(cursor);
    const response = await client.open(path`/runs/${runId}/stream`, { headers, signal: stop.signal });
    if (!response.ok) throw await client.failure(response);
    const type = response.headers.get('content-type') ?? '';
    if (!type.startsWith('text/event-stream') || response.body === null) {
      void response.body?.cancel().catch(() => undefined);
      return 'unsupported';
    }
    // The stream is open: changes will reach us as they happen, even if none has yet (a quiet run).
    set('live');
    let delivered = false;
    for await (const message of readSse(response.body, stop.signal)) {
      if (message.event === 'graph') {
        let data: unknown;
        try {
          data = JSON.parse(message.data);
        } catch {
          continue;
        }
        if (!isChanges(data)) continue;
        delivered = true;
        deliver(data);
      } else if (message.event === 'end') {
        return 'end';
      } else if (message.event === 'reconnect') {
        return 'reconnect';
      }
    }
    // Closed with no word: the network, a proxy or the server hanging up.
    if (stop.signal.aborted) return 'closed';
    return delivered ? 'dropped' : 'dropped-empty';
  }

  async function poll(): Promise<'ended' | 'continue'> {
    const changes = await client.get<GraphChanges>(path`/runs/${runId}/graph/changes`, { query: { since: cursor ?? BEYOND }, signal: stop.signal });
    deliver(changes);
    return isTerminal(changes.status) ? 'ended' : 'continue';
  }

  async function main() {
    let failures = 0;
    let backoff = MIN_BACKOFF_MS;
    set('connecting');
    for (;;) {
      if (stop.signal.aborted) return;
      try {
        const ended = await stream();
        if (ended === 'end') return set('ended');
        if (ended === 'closed') return;
        if (ended === 'unsupported') failures = failuresBeforePolling;
        else if (ended === 'reconnect') {
          // The server asked for it (it closes a stream after 15 minutes): come back at once.
          failures = 0;
          backoff = MIN_BACKOFF_MS;
          set('reconnecting');
          continue;
        } else {
          // A stream that delivered and then closed is not a failure; one that never delivered is. Neither
          // is tried again at once: a server that hangs up at every connection must not be hammered.
          failures = ended === 'dropped' ? 0 : failures + 1;
          if (ended === 'dropped') backoff = MIN_BACKOFF_MS;
          set('reconnecting');
        }
      } catch (cause) {
        if (stop.signal.aborted) return;
        const error = isApiError(cause) ? cause : new ApiError({ code: 'TRANSPORT', status: 0, message: 'The stream broke.', retryable: true });
        if (error.code === 'UNAUTHENTICATED') return set('unauthorized', error);
        if (error.code === 'NOT_FOUND' || error.code === 'FORBIDDEN') return set('failed', error);
        // No place left in the tenant's share of streams: ask for the changes instead.
        failures = error.status === 429 ? failuresBeforePolling : failures + 1;
        if (error.retryAfterMs !== undefined && failures < failuresBeforePolling) await sleep(error.retryAfterMs);
        set('reconnecting', error);
      }
      if (failures >= failuresBeforePolling) {
        const outcome = await pollingUntilStreamIsWorthTrying();
        if (outcome === 'ended' || outcome === 'stopped') return;
        failures = 0;
        backoff = MIN_BACKOFF_MS;
        continue;
      }
      await sleep(backoff * (0.8 + random() * 0.4));
      backoff = Math.min(backoff * 2, MAX_BACKOFF_MS);
    }
  }

  async function pollingUntilStreamIsWorthTrying(): Promise<'ended' | 'stopped' | 'retry'> {
    set('polling');
    const until = Date.now() + retryStreamAfter;
    while (Date.now() < until) {
      if (stop.signal.aborted) return 'stopped';
      try {
        if ((await poll()) === 'ended') {
          set('ended');
          return 'ended';
        }
      } catch (cause) {
        if (stop.signal.aborted) return 'stopped';
        const error = isApiError(cause) ? cause : undefined;
        if (error?.code === 'UNAUTHENTICATED') {
          set('unauthorized', error);
          return 'stopped';
        }
        if (error?.code === 'NOT_FOUND' || error?.code === 'FORBIDDEN') {
          set('failed', error);
          return 'stopped';
        }
        // A poll that failed is tried again at the next tick.
      }
      await sleep(pollEvery);
    }
    return 'retry';
  }

  void main();
  return {
    close() {
      stop.abort();
    },
  };
}

const TERMINAL = new Set(['SUCCEEDED', 'FAILED', 'CANCELLED', 'TIMED_OUT', 'COMPENSATED', 'PARTIAL', 'COMPENSATION_FAILED']);

/** Whether a run in this status will not change again. */
export function isTerminal(status: string): boolean {
  return TERMINAL.has(status);
}
