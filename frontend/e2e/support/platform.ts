import { env } from './env';

/** The platform's REST API as a test uses it to set the world up and to check what the Studio did (not through the Studio). */
export type Who = keyof typeof env.tokens;

export class Api {
  constructor(private readonly who: Who = 'admin') {}

  async call<T = Record<string, any>>(method: string, path: string, body?: unknown, expect?: number): Promise<{ status: number; body: T }> {
    const response = await fetch(`${env.origin}/v1${path}`, {
      method,
      headers: {
        Authorization: `Bearer ${env.tokens[this.who]}`,
        'X-Octopus-Namespace': env.namespace,
        'Idempotency-Key': crypto.randomUUID(),
        ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
      },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    const text = await response.text();
    const parsed = (text === '' ? undefined : JSON.parse(text)) as T;
    if (expect !== undefined && response.status !== expect) throw new Error(`${method} ${path} answered ${response.status}, not ${expect}: ${text}`);
    return { status: response.status, body: parsed };
  }

  async publish(name: string, ir: Record<string, unknown>): Promise<{ id: string; version: number }> {
    const created = await this.call<{ id: string }>('POST', '/workflows', { name, ir }, 201);
    const published = await this.call<{ versionNumber: number }>('POST', `/workflows/${created.body.id}/publish`, { draftRevision: 1 }, 201);
    return { id: created.body.id, version: published.body.versionNumber };
  }

  async start(workflowId: string, input: unknown = {}, extra: Record<string, unknown> = {}): Promise<string> {
    const run = await this.call<{ runId: string }>('POST', '/runs', { workflowId, input, ...extra }, 202);
    return run.body.runId;
  }

  async run(runId: string) {
    return (await this.call<{ status: string; paused?: boolean; version: number }>('GET', `/runs/${runId}`, undefined, 200)).body;
  }

  async graph(runId: string) {
    return (await this.call<{ status: string; projectionSeq: number; nodes: Array<{ id: string; status: string; attempt?: number; failureCode?: string | null; waitingOn?: string | null }> }>('GET', `/runs/${runId}/graph`, undefined, 200)).body;
  }

  async journal(runId: string) {
    return (await this.call<{ items: Array<{ seq: number; eventType: string; stepId?: string | null; attempt?: number | null; actor: string }> }>('GET', `/runs/${runId}/events?limit=500`, undefined, 200)).body.items;
  }

  /** Polls until a run has the status (the engine's loops move it along). */
  async until(runId: string, status: string | string[], timeoutMs = 60_000): Promise<string> {
    const wanted = Array.isArray(status) ? status : [status];
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const run = await this.run(runId);
      if (wanted.includes(run.status)) return run.status;
      if (Date.now() > deadline) throw new Error(`run ${runId} is ${run.status}, not ${wanted.join('|')} after ${timeoutMs} ms`);
      await new Promise((resolve) => setTimeout(resolve, 150));
    }
  }
}

export const IR_BASE = {
  apiVersion: 'sagat-octopus-orchestrator.io/v1',
  kind: 'Workflow',
};

/** The payment saga the repository ships, with small retry delays so a journey does not wait. */
export function paymentSaga(name: string): Record<string, unknown> {
  return {
    ...IR_BASE,
    metadata: { name, description: 'Reserves a limit, submits a payment, undoes the reservation if the payment is declined' },
    entrypoint: 'validate',
    triggers: [{ kind: 'manual' }],
    inputSchema: { type: 'object', required: ['paymentId', 'amount'], properties: { paymentId: { type: 'string' }, amount: { type: 'number' } } },
    nodes: [
      { id: 'validate', type: 'activity', activity: 'payment.validate', timeoutMs: 5000, retry: { maxAttempts: 3, strategy: 'fixed', initialDelayMs: 300, maxDelayMs: 300, jitter: false }, next: 'check' },
      { id: 'check', type: 'condition', expression: 'input.amount > 1000', ifTrue: 'review', ifFalse: 'settlement' },
      { id: 'review', type: 'activity', activity: 'payment.review', timeoutMs: 5000, next: 'settlement' },
      { id: 'settlement', type: 'saga', begin: 'reserve', next: 'done', onCompensationFailure: 'manual_review', cancelPolicy: 'compensate_completed' },
      {
        id: 'reserve', type: 'activity', activity: 'limit.reserve', timeoutMs: 5000, next: 'submit', compensateWith: 'limit.release',
        compensationRetry: { maxAttempts: 1, strategy: 'fixed', initialDelayMs: 100, maxDelayMs: 100, jitter: false },
      },
      { id: 'submit', type: 'activity', activity: 'payment.submit', timeoutMs: 5000, compensateWith: 'payment.return', retry: { maxAttempts: 1 } },
      { id: 'done', type: 'end' },
    ],
  };
}
