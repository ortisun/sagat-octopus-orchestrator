import type { App } from '../src/app/context';
import { createApp } from '../src/app/context';
import type { GraphChanges, GraphEvent, JournalEvent, OperationRequest, OperationResult, Run, RuntimeEdge, RuntimeGraph, RuntimeNode } from '../src/api/types';
import { AUTH_CONTEXT, TOKEN, fakePlatform } from './render';
import type { Routes } from './render';

/** Fixtures and a scripted platform for the runs pages. Nothing here is imported by the Studio itself. */

export const RUN_ID = '019b5d6e-0000-7000-8000-000000000001';
export const WORKFLOW_ID = '019b5d6e-0000-7000-8000-0000000000aa';
export const VERSION_ID = '019b5d6e-0000-7000-8000-0000000000bb';
export const TRACE = 'a'.repeat(32);

export const at = (seconds: number): string => new Date(Date.UTC(2026, 9, 10, 10, 0, seconds)).toISOString();

export const node = (id: string, status: RuntimeNode['status'], over: Partial<RuntimeNode> = {}): RuntimeNode => ({ id, type: 'activity', status, attempt: 0, ...over });

export const graphOf = (nodes: RuntimeNode[], edges: RuntimeEdge[], over: Partial<RuntimeGraph> = {}): RuntimeGraph => ({ runId: RUN_ID, status: 'RUNNING', projectionSeq: 10, nodes, edges, ...over });

export const changesOf = (from: number, to: number, nodes: RuntimeNode[], over: Partial<GraphChanges> = {}): GraphChanges => ({ runId: RUN_ID, status: 'RUNNING', fromSeq: from, projectionSeq: to, reset: false, events: [], nodes, ...over });

export const graphEvent = (seq: number, eventType: string, stepId: string | null = null, attempt: number | null = null): GraphEvent => ({ seq, eventType, occurredAt: at(seq), stepId, attempt });

export const journalEvent = (seq: number, eventType: string, stepId: string | null = null, over: Partial<JournalEvent> = {}): JournalEvent => ({
  seq,
  eventType,
  occurredAt: at(seq),
  stepId,
  attempt: stepId === null ? null : 1,
  actor: 'worker:w1',
  traceId: TRACE,
  payloadRef: null,
  ...over,
});

export const runOf = (over: Partial<Run> = {}): Run => ({
  runId: RUN_ID,
  workflowId: WORKFLOW_ID,
  versionId: VERSION_ID,
  status: 'RUNNING',
  version: 10,
  paused: false,
  startedBy: 'user:alice',
  createdAt: at(0),
  updatedAt: at(9),
  finishedAt: null,
  deadline: null,
  traceId: TRACE,
  correlationId: 'order-1234',
  depth: 0,
  parent: null,
  ...over,
});

/** A saga that reserves stock and takes a payment, with the undo of each, and a condition in front. */
export function sagaRun(): { nodes: RuntimeNode[]; edges: RuntimeEdge[] } {
  const nodes = [
    node('check', 'SUCCEEDED', { type: 'condition', attempt: 1 }),
    node('shipping', 'SKIPPED', { type: 'activity' }),
    node('order', 'RUNNING', { type: 'saga', attempt: 1, saga: { phase: 'FORWARD', irreversible: [] } }),
    node('reserve', 'SUCCEEDED', { attempt: 1 }),
    node('pay', 'FAILED', { attempt: 3, failureCode: 'HTTP_500' }),
    node('reserve~c', 'WAITING', { kind: 'COMPENSATION', forward: 'reserve', waitingOn: 'operator', attempt: 2, failureCode: 'COMPENSATION_FAILED' }),
    node('pay~r', 'NOT_STARTED', { kind: 'RECONCILIATION', forward: 'pay' }),
    node('wait', 'WAITING', { type: 'timer', waitingOn: 'timer', dueAt: '2099-01-01T00:00:00Z' }),
    node('notify', 'NOT_STARTED'),
  ];
  const edges: RuntimeEdge[] = [
    { from: 'check', to: 'order', kind: 'true' },
    { from: 'check', to: 'shipping', kind: 'false' },
    { from: 'order', to: 'reserve', kind: 'next' },
    { from: 'reserve', to: 'pay', kind: 'next' },
    { from: 'pay', to: 'pay~r', kind: 'reconcile' },
    { from: 'pay', to: 'reserve~c', kind: 'error' },
    { from: 'reserve~c', to: 'wait', kind: 'compensation' },
    { from: 'wait', to: 'notify', kind: 'next' },
  ];
  return { nodes, edges };
}

/** A response whose body the test writes to: the event stream of a followed run. */
export class Pipe {
  readonly response: Response;
  #controller!: ReadableStreamDefaultController<Uint8Array>;
  constructor(status = 200, type = 'text/event-stream') {
    const body = new ReadableStream<Uint8Array>({ start: (c) => (this.#controller = c) });
    this.response = new Response(body, { status, headers: { 'content-type': type } });
  }
  write(text: string): void {
    this.#controller.enqueue(new TextEncoder().encode(text));
  }
  /** A change, framed as the platform sends it. */
  send(changes: GraphChanges): void {
    this.write(`event: graph\nid: ${changes.projectionSeq}\ndata: ${JSON.stringify(changes)}\n\n`);
  }
  end(): void {
    this.write('event: end\ndata: {}\n\n');
  }
  close(): void {
    this.#controller.close();
  }
}

export const jsonResponse = (body: unknown, status = 200, headers: Record<string, string> = {}): Response =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', ...headers } });

export const failure = (status: number, code: string, message: string, details?: Record<string, unknown>): Response =>
  jsonResponse({ code, message, retryable: false, traceId: 'b'.repeat(32), ...(details ? { details } : {}) }, status);

export interface RunPlatformInit {
  run?: Run;
  graph?: RuntimeGraph;
  journal?: JournalEvent[];
  pageSize?: number;
  /** What `POST /runs/{id}/operations` answers; by default 202 with the run's next version. */
  operate?: (body: OperationRequest, key: string) => Response;
  /** How the stream answers; by default a pipe for each call, kept in `pipes`. */
  stream?: (n: number) => Response;
  routes?: Routes;
}

/**
 * A platform with one run: the run, its graph, its journal, an event stream the test writes to and
 * the operations endpoint, all recorded in `calls`.
 */
export function runPlatform(init: RunPlatformInit = {}) {
  const base = sagaRun();
  const state = {
    run: init.run ?? runOf({ status: 'MANUAL_REVIEW' }),
    graph: init.graph ?? graphOf(base.nodes, base.edges, { status: 'MANUAL_REVIEW', projectionSeq: 10 }),
    journal: init.journal ?? [],
    graphReads: 0,
    streams: 0,
  };
  const pipes: Pipe[] = [];
  const operations: { body: OperationRequest; key: string }[] = [];
  const size = init.pageSize ?? 200;
  const routes: Routes = {
    [`GET /runs/${RUN_ID}`]: () => state.run,
    [`GET /runs/${RUN_ID}/graph`]: () => {
      state.graphReads += 1;
      return state.graph;
    },
    [`GET /runs/${RUN_ID}/graph/changes`]: () => changesOf(state.graph.projectionSeq, state.graph.projectionSeq, [], { status: state.graph.status ?? 'RUNNING' }),
    [`GET /runs/${RUN_ID}/stream`]: () => {
      state.streams += 1;
      if (init.stream) return init.stream(state.streams);
      const pipe = new Pipe();
      pipes.push(pipe);
      return pipe.response;
    },
    [`GET /runs/${RUN_ID}/events`]: (_init, url) => {
      const cursor = new URL(url, 'http://studio.test').searchParams.get('cursor');
      const from = cursor === null ? 0 : Number(cursor);
      const items = state.journal.slice(from, from + size);
      return { items, nextCursor: from + size < state.journal.length ? String(from + size) : null };
    },
    [`POST /runs/${RUN_ID}/operations`]: (request) => {
      const body = JSON.parse(request.body as string) as OperationRequest;
      const key = (request.headers as Record<string, string>)['Idempotency-Key'] ?? '';
      operations.push({ body, key });
      if (init.operate) return init.operate(body, key);
      return jsonResponse({ operationId: '019b5d6e-0000-7000-8000-0000000000cc', accepted: true, runStatus: state.run.status, runVersion: state.run.version + 1 } satisfies OperationResult, 202);
    },
    [`GET /workflows/${WORKFLOW_ID}`]: () => ({ id: WORKFLOW_ID, name: 'order-saga', draftRevision: 1, ir: {} }),
    [`GET /workflows/${WORKFLOW_ID}/versions`]: () => ({ items: [{ workflowId: WORKFLOW_ID, versionId: VERSION_ID, versionNumber: 3, digest: 'sha256:abc' }], nextCursor: null }),
    ...init.routes,
  };
  const platform = fakePlatform(routes);
  return { ...platform, state, pipes, operations, routes };
}

/** An app that is signed in with the permissions given (the platform's, not the Studio's, are what count), working in the namespace `dev`. */
export async function signedInApp(fetcher: typeof fetch, permissions: string[] = AUTH_CONTEXT.permissions): Promise<App> {
  const app = createApp({ fetch: fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, role: 'operator', permissions } as never));
  app.namespace.set('dev');
  return app;
}

export const OPERATOR = ['run:read', 'run:operate', 'definition:read'];
export const ADMIN = AUTH_CONTEXT.permissions;
export const VIEWER = ['run:read', 'definition:read'];
