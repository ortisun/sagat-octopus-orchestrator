import * as grpc from '@grpc/grpc-js';

/** A fault of the scenario file (tests/contract/sdk-conformance/README.md, "Faults"). */
export interface Fault {
  channel: 'http' | 'grpc';
  match: string;
  action: 'dropResponse' | 'dropStreamAfterFirst' | 'failBefore' | 'status';
  times?: number;
  after?: number;
  status?: number;
  code?: string;
  reason?: string;
  retryAfterMs?: number;
  body?: unknown;
}

const CODES: Record<string, number> = Object.fromEntries(
  Object.entries(grpc.status).filter(([, v]) => typeof v === 'number') as Array<[string, number]>,
);

/**
 * The faults of a scenario, one plan for every client and worker host of the run. A call is counted against
 * every fault that matches it, and the first fault in file order that is due answers it; `applied()` says,
 * per fault and in file order, how many calls it answered (the scenario proves a fault was reached by it).
 */
export class FaultPlan {
  private readonly seen: number[];
  private readonly answered: number[];

  constructor(private readonly faults: Fault[]) {
    this.seen = faults.map(() => 0);
    this.answered = faults.map(() => 0);
  }

  applied(): number[] {
    return [...this.answered];
  }

  private next(channel: 'http' | 'grpc', key: string): Fault | undefined {
    let due: number | undefined;
    this.faults.forEach((fault, i) => {
      if (fault.channel !== channel || !key.startsWith(fault.match)) return;
      const count = this.seen[i]!;
      this.seen[i] = count + 1;
      const after = fault.after ?? 0;
      const times = fault.times ?? Number.POSITIVE_INFINITY;
      if (due === undefined && count >= after && count < after + times) due = i;
    });
    if (due === undefined) return undefined;
    this.answered[due] = this.answered[due]! + 1;
    return this.faults[due];
  }

  /** A `fetch` for the REST client: what a fault says happens to the call, otherwise the real thing. */
  fetch(apiPath: string): typeof fetch {
    return async (input, init) => {
      const url = new URL(input instanceof Request ? input.url : String(input));
      const key = `${(init?.method ?? 'GET').toUpperCase()} ${url.pathname.replace(apiPath, '') || '/'}`;
      const fault = this.next('http', key);
      if (!fault) return fetch(input, init);
      if (fault.action === 'failBefore') throw new TypeError('fetch failed');
      if (fault.action === 'status') {
        const headers: Record<string, string> = { 'content-type': 'application/json' };
        if (fault.retryAfterMs !== undefined) headers['retry-after'] = String(Math.ceil(fault.retryAfterMs / 1000));
        return new Response(JSON.stringify(fault.body ?? {}), { status: fault.status ?? 503, headers });
      }
      // dropResponse: the call is made, the server applies it, the answer is lost.
      const response = await fetch(input, init);
      await response.arrayBuffer();
      throw new TypeError('fetch failed');
    };
  }

  /** The interceptor of the worker's gRPC channel. */
  interceptors(): grpc.Interceptor[] {
    if (!this.faults.some((fault) => fault.channel === 'grpc')) return [];
    const interceptor: grpc.Interceptor = (options, nextCall) => {
      const method = options.method_definition.path.split('/').pop() ?? '';
      const fault = this.next('grpc', method);
      const status = (code: number, reason?: string, retryAfterMs?: number): grpc.StatusObject => {
        const metadata = new grpc.Metadata();
        if (reason) metadata.set('x-sagat-error', reason);
        if (retryAfterMs !== undefined) metadata.set('retry-after-ms', String(retryAfterMs));
        return { code, details: 'injected', metadata };
      };
      if (!fault) return new grpc.InterceptingCall(nextCall(options));
      if (fault.action === 'failBefore' || fault.action === 'status') {
        const code = fault.action === 'failBefore' ? grpc.status.UNAVAILABLE : (CODES[fault.code ?? 'UNAVAILABLE'] ?? grpc.status.UNAVAILABLE);
        // The call never reaches the server: there is nothing underneath.
        const requester = {
          start(_metadata: grpc.Metadata, listener: grpc.InterceptingListener): void {
            queueMicrotask(() => {
              listener.onReceiveMetadata(new grpc.Metadata());
              listener.onReceiveStatus(status(code, fault.reason, fault.retryAfterMs));
            });
          },
          sendMessage(): void {},
          halfClose(): void {},
          cancel(): void {},
        };
        return new grpc.InterceptingCall(null as never, requester as unknown as grpc.Requester);
      }
      if (fault.action === 'dropStreamAfterFirst') {
        const underlying = nextCall(options);
        let delivered = false;
        return new grpc.InterceptingCall(underlying, {
          start(metadata, listener, next) {
            next(metadata, {
              onReceiveMetadata: (md, forward) => forward(md),
              onReceiveMessage: (message: unknown, forward) => {
                forward(message);
                const tasks = (message as { tasks?: unknown[] } | null)?.tasks;
                if (!delivered && Array.isArray(tasks) && tasks.length > 0) {
                  delivered = true;
                  queueMicrotask(() => underlying.cancelWithStatus(grpc.status.CANCELLED, 'injected'));
                }
              },
              onReceiveStatus: (received, forward) =>
                forward(delivered ? status(grpc.status.UNAVAILABLE) : received),
            });
            void listener;
          },
        });
      }
      // dropResponse: the server applies the call; its answer is replaced by a failure to deliver it.
      return new grpc.InterceptingCall(nextCall(options), {
        start(metadata, listener, next) {
          next(metadata, {
            onReceiveMetadata: (md, forward) => forward(md),
            onReceiveMessage: () => undefined,
            onReceiveStatus: (_status, forward) => forward(status(grpc.status.UNAVAILABLE)),
          });
          void listener;
        },
      });
    };
    return [interceptor];
  }
}

