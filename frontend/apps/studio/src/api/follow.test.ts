import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from './client';
import { followRun, isTerminal } from './follow';
import type { FollowState } from './follow';
import type { GraphChanges } from './types';

const RUN = '019b5d6e-0000-7000-8000-000000000001';

function changes(seq: number, over: Partial<GraphChanges> = {}): GraphChanges {
  return { runId: RUN, status: 'RUNNING', fromSeq: seq - 1, projectionSeq: seq, reset: false, events: [], nodes: [], ...over };
}

const frame = (seq: number, over: Partial<GraphChanges> = {}) => `event: graph\nid: ${seq}\ndata: ${JSON.stringify(changes(seq, over))}\n\n`;

/** A response whose body the test writes to. */
class Pipe {
  readonly response: Response;
  #controller!: ReadableStreamDefaultController<Uint8Array>;
  constructor(status = 200, type = 'text/event-stream') {
    const body = new ReadableStream<Uint8Array>({ start: (c) => (this.#controller = c) });
    this.response = new Response(body, { status, headers: { 'content-type': type } });
  }
  write(text: string) {
    this.#controller.enqueue(new TextEncoder().encode(text));
  }
  close() {
    this.#controller.close();
  }
}

interface Call {
  url: string;
  headers: Record<string, string>;
  signal: AbortSignal | undefined;
}

function harness(script: (call: Call, n: number) => Response | Promise<Response>) {
  const calls: Call[] = [];
  const unauthenticated = vi.fn();
  const client = new ApiClient({
    baseUrl: '/v1',
    token: () => 't',
    namespace: () => undefined,
    onUnauthenticated: unauthenticated,
    fetch: ((url: string, init: RequestInit) => {
      const call = { url, headers: init.headers as Record<string, string>, signal: init.signal ?? undefined };
      calls.push(call);
      return Promise.resolve(script(call, calls.length));
    }) as typeof fetch,
  });
  const delivered: GraphChanges[] = [];
  const states: FollowState[] = [];
  const start = (over: Partial<Parameters<typeof followRun>[0]> = {}) =>
    followRun({ client, runId: RUN, onChanges: (c) => delivered.push(c), onState: (s) => states.push(s), random: () => 0.5, ...over });
  return { calls, delivered, states, start, unauthenticated };
}

const json = (body: unknown, status = 200, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', ...headers } });

const tick = () => vi.advanceTimersByTimeAsync(0);

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('followRun', () => {
  it('delivers the changes in order, is live after the first, and asks for the whole graph when it has none', async () => {
    const pipe = new Pipe();
    const h = harness(() => pipe.response);
    const follow = h.start();
    await tick();
    expect(h.calls[0]!.url).toBe(`/v1/runs/${RUN}/stream`);
    expect(h.calls[0]!.headers['Last-Event-ID']).toBeUndefined();
    expect(h.calls[0]!.headers['Accept']).toBe('text/event-stream');
    // Live as soon as the stream is open, before anything arrives: a quiet run is not "connecting".
    expect(h.states).toEqual(['connecting', 'live']);
    pipe.write(frame(5, { reset: true }));
    pipe.write(frame(7));
    await tick();
    expect(h.delivered.map((c) => c.projectionSeq)).toEqual([5, 7]);
    expect(h.states).toEqual(['connecting', 'live']);
    follow.close();
  });

  it('resumes from the sequence it was given', async () => {
    const pipe = new Pipe();
    const h = harness(() => pipe.response);
    h.start({ since: 41 });
    await tick();
    expect(h.calls[0]!.headers['Last-Event-ID']).toBe('41');
  });

  it('stops when the run is over and does not open the stream again', async () => {
    const pipe = new Pipe();
    const h = harness(() => pipe.response);
    h.start();
    await tick();
    pipe.write(frame(9, { status: 'SUCCEEDED' }));
    pipe.write('event: end\ndata: {}\n\n');
    await tick();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(h.states.at(-1)).toBe('ended');
    expect(h.calls).toHaveLength(1);
  });

  it('opens the stream again with the last sequence it delivered when the connection drops', async () => {
    const first = new Pipe();
    const second = new Pipe();
    const h = harness((_c, n) => (n === 1 ? first.response : second.response));
    h.start();
    await tick();
    first.write(frame(3));
    await tick();
    first.close(); // the network went away; no `end`
    await tick();
    expect(h.states.at(-1)).toBe('reconnecting');
    await vi.advanceTimersByTimeAsync(600);
    expect(h.calls).toHaveLength(2);
    expect(h.calls[1]!.headers['Last-Event-ID']).toBe('3');
    second.write(frame(4));
    await tick();
    expect(h.delivered.map((c) => c.projectionSeq)).toEqual([3, 4]);
    expect(h.states.at(-1)).toBe('live');
  });

  it('does not hammer a server that hangs up at every connection: it waits, longer each time, and then polls', async () => {
    const h = harness(() => new Response(new ReadableStream({ start: (c) => c.close() }), { status: 200, headers: { 'content-type': 'text/event-stream' } }));
    h.start({ streamFailuresBeforePolling: 3, pollIntervalMs: 60_000 });
    await vi.advanceTimersByTimeAsync(0);
    expect(h.calls).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(400);
    expect(h.calls).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(200);
    expect(h.calls).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(5000);
    expect(h.calls.filter((c) => c.url.includes('/stream')).length).toBeLessThanOrEqual(4);
    expect(h.states).toContain('polling');
  });

  it('comes back at once when the server says so (it closes a stream after 15 minutes)', async () => {
    const first = new Pipe();
    const second = new Pipe();
    const h = harness((_c, n) => (n === 1 ? first.response : second.response));
    h.start();
    await tick();
    first.write(frame(3));
    first.write('event: reconnect\ndata: {}\n\n');
    await tick();
    expect(h.calls).toHaveLength(2);
    expect(h.calls[1]!.headers['Last-Event-ID']).toBe('3');
  });

  it('ignores an event it cannot read instead of stopping', async () => {
    const pipe = new Pipe();
    const h = harness(() => pipe.response);
    h.start();
    await tick();
    pipe.write('event: graph\ndata: not json\n\n');
    pipe.write('event: graph\ndata: {"nodes": 1}\n\n');
    pipe.write('event: surprise\ndata: {}\n\n');
    pipe.write(frame(2));
    await tick();
    expect(h.delivered.map((c) => c.projectionSeq)).toEqual([2]);
  });

  it('stops for good when the token is not valid, and says so to the session', async () => {
    const h = harness(() => json({ code: 'UNAUTHENTICATED', message: 'expired' }, 401));
    h.start();
    await tick();
    expect(h.states.at(-1)).toBe('unauthorized');
    await vi.advanceTimersByTimeAsync(120_000);
    expect(h.calls).toHaveLength(1);
    expect(h.unauthenticated).toHaveBeenCalledOnce();
  });

  it('stops for good when the run does not exist or may not be read', async () => {
    for (const [status, code] of [
      [404, 'NOT_FOUND'],
      [403, 'FORBIDDEN'],
    ] as const) {
      const h = harness(() => json({ code, message: 'no' }, status));
      h.start();
      await tick();
      expect(h.states.at(-1)).toBe('failed');
      await vi.advanceTimersByTimeAsync(60_000);
      expect(h.calls).toHaveLength(1);
    }
  });

  it('asks for the changes instead when the tenant has no place left for another stream', async () => {
    const h = harness((c) =>
      c.url.includes('/stream')
        ? json({ code: 'RATE_LIMITED', message: 'full', details: { reason: 'STREAMS_PER_TENANT' } }, 429, { 'retry-after': '5' })
        : json(changes(12, { status: 'RUNNING' })),
    );
    h.start({ since: 10, pollIntervalMs: 2000 });
    await tick();
    await vi.advanceTimersByTimeAsync(0);
    expect(h.states).toContain('polling');
    const polled = h.calls.filter((c) => c.url.includes('/graph/changes'));
    expect(polled[0]!.url).toBe(`/v1/runs/${RUN}/graph/changes?since=10`);
    await vi.advanceTimersByTimeAsync(2000);
    expect(h.calls.filter((c) => c.url.includes('/graph/changes'))[1]!.url).toContain('since=12');
    expect(h.delivered.length).toBeGreaterThanOrEqual(2);
  });

  it('stops polling when the run is over', async () => {
    let n = 0;
    const h = harness((c) => {
      if (c.url.includes('/stream')) return json({ code: 'RATE_LIMITED', message: 'full' }, 429);
      n += 1;
      return json(changes(10 + n, { status: n < 3 ? 'RUNNING' : 'SUCCEEDED' }));
    });
    h.start({ pollIntervalMs: 1000 });
    await vi.advanceTimersByTimeAsync(10_000);
    expect(h.states.at(-1)).toBe('ended');
    const after = h.calls.length;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(h.calls).toHaveLength(after);
  });

  it('gives up on the stream after repeated failures, polls, and tries the stream again later', async () => {
    let streams = 0;
    const pipe = new Pipe();
    const h = harness((c) => {
      if (c.url.includes('/stream')) {
        streams += 1;
        return streams <= 3 ? json({ code: 'UNAVAILABLE', message: 'down' }, 503) : pipe.response;
      }
      return json(changes(20));
    });
    h.start({ streamFailuresBeforePolling: 3, retryStreamAfterMs: 5000, pollIntervalMs: 1000 });
    await vi.advanceTimersByTimeAsync(10_000);
    expect(h.states).toContain('polling');
    expect(streams).toBeGreaterThanOrEqual(4);
    pipe.write(frame(21));
    await tick();
    expect(h.states.at(-1)).toBe('live');
  });

  it('polls when the answer is not an event stream (a proxy that does not stream)', async () => {
    const h = harness((c) => (c.url.includes('/stream') ? json({ nothing: true }) : json(changes(2))));
    h.start({ pollIntervalMs: 1000 });
    await vi.advanceTimersByTimeAsync(1500);
    expect(h.states).toContain('polling');
    expect(h.delivered.length).toBeGreaterThan(0);
  });

  it('backs off between failed attempts, with a ceiling', async () => {
    const h = harness(() => json({ code: 'UNAVAILABLE', message: 'down' }, 503));
    h.start({ streamFailuresBeforePolling: 50 });
    await tick();
    const times: number[] = [];
    let last = 0;
    for (let ms = 0; ms < 120_000 && times.length < 8; ms += 100) {
      await vi.advanceTimersByTimeAsync(100);
      if (h.calls.length > times.length + 1) {
        times.push(ms - last);
        last = ms;
      }
    }
    expect(times[1]!).toBeGreaterThan(times[0]!);
    expect(Math.max(...times)).toBeLessThanOrEqual(15_500);
  });

  it('closes everything it opened', async () => {
    const pipe = new Pipe();
    const h = harness(() => pipe.response);
    const follow = h.start();
    await tick();
    follow.close();
    await tick();
    expect(h.calls[0]!.signal?.aborted).toBe(true);
    await vi.advanceTimersByTimeAsync(120_000);
    expect(h.calls).toHaveLength(1);
    // The body was cancelled from our side: the server's writes have nowhere to go.
    expect(() => pipe.write(frame(99))).toThrow();
    await tick();
    expect(h.delivered).toEqual([]);
  });
});

describe('isTerminal', () => {
  it('knows the statuses a run does not leave', () => {
    for (const s of ['SUCCEEDED', 'FAILED', 'CANCELLED', 'TIMED_OUT', 'COMPENSATED', 'PARTIAL', 'COMPENSATION_FAILED']) expect(isTerminal(s)).toBe(true);
    for (const s of ['RUNNING', 'WAITING', 'PAUSED', 'CANCELLING', 'COMPENSATING', 'MANUAL_REVIEW']) expect(isTerminal(s)).toBe(false);
  });
});
