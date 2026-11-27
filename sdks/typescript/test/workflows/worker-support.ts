/*
 * The stub worker service and the leases of the tests that drive `WorkflowTasks` directly (security.test.ts,
 * worker.test.ts): the leases are those the fake workflow server issues for `code-run-lifecycle`.
 */
import { createHash } from 'node:crypto';
import { silentLogger, type Logger } from '../../src/config.js';
import { DEFAULT_RETRY } from '../../src/retry.js';
import { SagatError } from '../../src/errors.js';
import { WorkflowTasks, type WorkflowTaskEvent } from '../../src/worker/workflows.js';
import type { TaskLease, WireHistoryEvent, WorkerRpc } from '../../src/worker/grpc.js';
import { loadGoldenRun, type GoldenRun } from '../fake-workflow-server.js';
import { REFERENCE_RUNS } from './goldens.js';
import { bundleOf } from './support.js';

export const golden: GoldenRun = loadGoldenRun('code-run-lifecycle');
export const order = REFERENCE_RUNS.find((run) => run.scenario === 'code-run-lifecycle')!.versions['v1']!;
export const DIGEST = golden.commands.codeVersions['v1']!;

export interface Calls { method: string; request: Record<string, any> }

export interface StubOptions {
  /** The pages ReadHistory answers, from the first one whose events are at or after the request's `from_seq`. */
  pages?: WireHistoryEvent[][];
  lastPage?: boolean;
  blobs?: Map<string, Buffer>;
  header?: Record<string, unknown>;
  /** Thrown by the n-th ReadHistory call (1 for the first) after its first page, once each. */
  historyErrors?: Map<number, SagatError>;
  /** Thrown by the n-th ReadBlob call before its header, once each. */
  blobErrors?: Map<number, SagatError>;
  /** The answers of CompleteWorkflowTask in turn (an error is thrown); ACCEPTED once they run out. */
  completeAnswers?: Array<Record<string, unknown> | SagatError>;
  /** Called as each ReadHistory page is asked for, before it is given. */
  onPage?: (index: number) => void;
}

/** A stub of the worker service: the history pages it is given, recorded calls, and the answers of a fake that accepts. */
export function stubRpc(calls: Calls[], options: StubOptions = {}): WorkerRpc {
  let historyCalls = 0;
  let blobCalls = 0;
  let pageIndex = 0;
  const answers = [...(options.completeAnswers ?? [])];
  const rpc = {
    async *readHistory(request: Record<string, any>) {
      calls.push({ method: 'ReadHistory', request });
      const call = ++historyCalls;
      const pages = (options.pages ?? []).filter((page) => page.length === 0 || page[page.length - 1]!.seq >= request['from_seq'])
        .map((page) => page.filter((event) => event.seq >= request['from_seq']));
      for (let i = 0; i < pages.length; i++) {
        if (i === 1 && options.historyErrors?.has(call)) {
          const error = options.historyErrors.get(call)!;
          options.historyErrors.delete(call);
          throw error;
        }
        options.onPage?.(pageIndex++);
        yield { events: pages[i]!, last_page: (options.lastPage ?? true) && i === pages.length - 1, lease_expires_at_epoch_ms: 0 };
      }
    },
    async *readBlob(request: Record<string, any>) {
      calls.push({ method: 'ReadBlob', request });
      const call = ++blobCalls;
      if (options.blobErrors?.has(call)) throw options.blobErrors.get(call)!;
      const bytes = options.blobs?.get(request['blob_id']) ?? Buffer.alloc(0);
      yield { header: options.header ?? { blob_id: request['blob_id'], size: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex'), content_type: 'application/json' } };
      yield { chunk: bytes };
    },
    async writeBlob() { calls.push({ method: 'WriteBlob', request: {} }); throw new SagatError({ code: 'INTERNAL', message: 'not served' }); },
    async completeWorkflowTask(request: Record<string, any>) {
      calls.push({ method: 'CompleteWorkflowTask', request });
      const answer = answers.shift();
      if (answer instanceof SagatError) throw answer;
      return answer ?? { outcome: 'WORKFLOW_TASK_ACCEPTED', cause: 'WORKFLOW_TASK_FAILURE_CAUSE_UNSPECIFIED', already_applied: false };
    },
    async failWorkflowTask(request: Record<string, any>) { calls.push({ method: 'FailWorkflowTask', request }); return { already_applied: false }; },
  };
  return rpc as unknown as WorkerRpc;
}

export function tasksOf(rpc: WorkerRpc, events: WorkflowTaskEvent[], log: Logger = silentLogger): WorkflowTasks {
  return new WorkflowTasks({
    offers: [{ workflowType: 'order', codeDigest: DIGEST, bundleCode: bundleOf({ order }).code }],
    rpc: () => rpc, log, emit: (event) => events.push(event), retry: { ...DEFAULT_RETRY, initialDelayMs: 1, maxDelayMs: 2 },
  });
}

/** The lease of activation k of the golden run, as the fake issues it, with fields to change. */
export function leaseOf(k: number, change: Partial<Record<string, unknown>> = {}, history: WireHistoryEvent[] = golden.history as WireHistoryEvent[]): TaskLease {
  const activation = golden.commands.activations.find((a) => a.k === k)!;
  const seq = activation.window.throughSeq!;
  const visible = history.filter((event) => event.seq <= seq);
  const started = visible.find((event) => event.seq === seq)!;
  const workflowTask = {
    run_id: golden.commands.runId, attempt: 1, generation: k, lease_token: `lease-${k}`, expires_at_epoch_ms: Date.now() + 60_000,
    seed: Buffer.from(golden.commands.seedHex, 'hex'), started_at_us: started['at_us'], window_start_seq: activation.window.afterSeq,
    gated_entries: [], state_max_bytes: 0, first_page: visible, visible_count: visible.length, continue_as_new_suggested: false,
    history_limit_reached: false, traceparent: '', workflow_type: 'order', code_digest: DIGEST, ...change,
  };
  return {
    task_id: `wt-${k}-1`, run_id: golden.commands.runId, step_id: '~workflow', attempt: 1, generation: k, lease_token: `lease-${k}`,
    expires_at_epoch_ms: workflowTask.expires_at_epoch_ms, activity_name: '', input_json: '', input_blob_ref: '', traceparent: '',
    tenant_alias: '', idempotency_key: '', task_kind: 'WORKFLOW', target_node: '', target_idempotency_key: '', workflow_task: workflowTask as never,
  };
}

