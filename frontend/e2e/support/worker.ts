import { ActivityFailure, WorkerHost } from '@sagat-octopus-orchestrator/sdk';
import type { ActivityHandler } from '@sagat-octopus-orchestrator/sdk';
import { env } from './env';

export type Handlers = Record<string, ActivityHandler<any, unknown>>;

/** A worker of the TypeScript SDK (the one the platform's users run), serving `handlers` until stopped. */
export async function startWorker(name: string, handlers: Handlers, compensations: Handlers = {}): Promise<WorkerHost> {
  const host = new WorkerHost({
    grpcUrl: env.grpcUrl,
    namespace: env.namespace,
    token: env.tokens.worker,
    workerName: name,
    maxInflight: 8,
    heartbeatIntervalMs: 500,
  });
  for (const [activity, handler] of Object.entries(handlers)) host.registerActivity(activity, handler);
  for (const [activity, handler] of Object.entries(compensations)) host.registerCompensation(activity, handler);
  await host.start();
  return host;
}

export { ActivityFailure };
export const ok: ActivityHandler<any, unknown> = () => ({ done: true });
