import { ActivityFailure, WorkerHost } from '@sagat-octopus-orchestrator/sdk';
import type { ActivityContext, ActivityHandler } from '@sagat-octopus-orchestrator/sdk';
import { env } from './env';

export type Handlers = Record<string, ActivityHandler<any, unknown>>;

/** A worker of the TypeScript SDK (the one the platform's users run), serving `handlers` until stopped. */
export async function startWorker(name: string, handlers: Handlers, compensations: Handlers = {}, namespace: string = env.namespace): Promise<WorkerHost> {
  const host = new WorkerHost({
    grpcUrl: env.grpcUrl,
    namespace,
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

/**
 * A handler that fails the first attempt the platform gives of each run it serves, and succeeds on a later one (the retry an operator
 * asks for, say). The outcome follows the run and the attempt the platform reports, `ctx.runId` and `ctx.attempt`, not the number of
 * times the handler ran: a worker serves every run of its namespace that waits for the activity, a run an earlier journey left
 * included, and a count shared by all of them hands the failure to whichever run comes first (the fixture of US04 failed that way:
 * the run of the journey was `COMPENSATED`, not `MANUAL_REVIEW`).
 */
export function failsFirstAttempt(failure: () => Error, result: unknown): ActivityHandler<any, unknown> {
  const first = new Map<string, number>();
  return (_input: unknown, ctx: ActivityContext) => {
    const firstSeen = first.get(ctx.runId) ?? ctx.attempt;
    first.set(ctx.runId, firstSeen);
    if (ctx.attempt === firstSeen) throw failure();
    return result;
  };
}
