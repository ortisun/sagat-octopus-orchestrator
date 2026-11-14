import type { Page } from '@playwright/test';
import { Api, IR_BASE } from './platform';

/** A name no other journey, and no other repetition of this one, uses: `<what>-<8 hex digits>`. */
export function unique(what: string): string {
  return `${what}-${crypto.randomUUID().slice(0, 8)}`;
}

/** A published workflow with nothing to do: its runs end at once, with no worker. */
export async function endWorkflow(api: Api, name: string, extra: Record<string, unknown> = {}): Promise<{ id: string; version: number }> {
  return api.publish(name, { ...IR_BASE, metadata: { name }, entrypoint: 'done', triggers: [{ kind: 'manual' }], nodes: [{ id: 'done', type: 'end' }], ...extra });
}

/**
 * A published workflow whose one step is an activity nobody serves: its runs stay open (the step waits for a worker that never
 * comes) until a person or a cancellation ends them. `activity` names the step's activity, and is bound to no worker.
 */
export async function openWorkflow(api: Api, name: string): Promise<{ id: string; version: number }> {
  return api.publish(name, {
    ...IR_BASE,
    metadata: { name },
    entrypoint: 'wait',
    triggers: [{ kind: 'manual' }],
    nodes: [
      { id: 'wait', type: 'activity', activity: 'e2e.nobody.serves.this', timeoutMs: 5000, next: 'done' },
      { id: 'done', type: 'end' },
    ],
  });
}

/** A namespace of its own, so a limit set on it (a quota of zero) cannot reach another journey. Returns its slug and its id. */
export async function ownNamespace(api: Api, what: string): Promise<{ slug: string; id: string }> {
  const slug = unique(what);
  const workspaces = await api.call<{ items: Array<{ id: string }> }>('GET', '/workspaces?limit=1', undefined, 200);
  const workspace = workspaces.body.items[0];
  if (!workspace) throw new Error('the journey needs a workspace');
  const created = await api.call<{ id: string }>('POST', `/workspaces/${workspace.id}/namespaces`, { slug, environment: 'dev' }, 201);
  return { slug, id: created.body.id };
}

/** The id of a namespace of the tenant, from its slug. */
export async function namespaceIdOf(api: Api, slug: string): Promise<string> {
  const found = await api.call<{ items: Array<{ id: string; slug: string }> }>('GET', '/namespaces?limit=200', undefined, 200);
  const namespace = found.body.items.find((item) => item.slug === slug);
  if (!namespace) throw new Error(`no namespace ${slug}`);
  return namespace.id;
}

/** Sets the quota of a resource in a namespace (`quota:manage`, audited). */
export async function setQuota(api: Api, namespaceId: string, resource: string, limit: number): Promise<void> {
  await api.call('PUT', `/namespaces/${namespaceId}/quotas/${resource}`, { limit }, 200);
}

/** Lets the worker identity of the harness serve `activities` in a namespace of the journey's own (the harness bound it in its own only). */
export async function bindWorker(admin: Api, namespaceId: string, activities: string[]): Promise<void> {
  const identity = await new Api('worker').call<{ subject: string }>('GET', '/auth/context', undefined, 200);
  await admin.call('PUT', `/namespaces/${namespaceId}/worker-bindings/${encodeURIComponent(identity.body.subject)}`, { activities }, 200);
}

/** An answer of 400 or more the page received: what the browser logged as a failed resource, with the status it had. */
export interface Refusal {
  status: number;
  method: string;
  url: string;
}

/** Records every answer of 400 or more the page gets, as they come (`withoutRefusals` reads them at the end of the journey). */
export function recordRefusals(page: Page): Refusal[] {
  const seen: Refusal[] = [];
  page.on('response', (response) => {
    if (response.status() >= 400) seen.push({ status: response.status(), method: response.request().method(), url: new URL(response.url()).pathname });
  });
  return seen;
}

/**
 * The problems of a journey that provokes refusals on purpose (a name in use, a quota of zero): the browser logs "Failed to load
 * resource" for every answer of 400 or more, and those lines are the refusals under test, not a Studio that logs errors. They are
 * dropped only for the statuses the journey `allowed`: an answer with any other status (a wrong route, a permission wall, a 5xx) is
 * a problem of its own, and so is everything else the page logged (a script error, a policy violation).
 */
export function withoutRefusals(problems: string[], seen: Refusal[], allowed: number[]): string[] {
  const logged = /^console\.error: Failed to load resource: the server responded with a status of [45]\d\d /;
  return [...problems.filter((problem) => !logged.test(problem)), ...seen.filter((refusal) => !allowed.includes(refusal.status)).map((refusal) => `refused ${refusal.status}: ${refusal.method} ${refusal.url}`)];
}

const leftovers: Array<{ api: Api; run: string }> = [];

/** A run the journey leaves open (its step waits for a worker nobody runs): it is cancelled when the journey is over, by `cancelLeftovers`. */
export function cancelAfterwards(api: Api, ...runs: string[]): void {
  for (const run of runs) leftovers.push({ api, run });
}

/**
 * The end of a journey (`test.afterEach`): the runs it left open are cancelled, so that the platform the next journeys share is
 * not left with runs that wait for a worker forever. Best effort: a run that has ended already refuses, and that is fine.
 */
export async function cancelLeftovers(): Promise<void> {
  const pending = leftovers.splice(0, leftovers.length);
  for (const { api, run } of pending) {
    await api.call('POST', `/runs/${run}/operations`, { action: 'CANCEL', reason: 'the journey is over and nothing waits on this run' }).catch(() => undefined);
  }
}
