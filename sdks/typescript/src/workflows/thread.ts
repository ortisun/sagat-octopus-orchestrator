/*
 * The workflow thread: the `node:worker_threads` thread in which the contexts of workflow runs live.
 *
 * Why a thread of its own: an activation deadline stops a `vm` evaluation, which may be inside the context's microtask
 * checkpoint; on a thread where async hooks are enabled (the host's OpenTelemetry context manager, a test runner) that
 * termination corrupts the thread's async-hook stack and Node ends the process. Async hooks belong to each thread, and
 * this one runs only the SDK's runtime, started with no inherited command-line options, so no hook is enabled here.
 * It also keeps workflow code off the thread that runs activities and the worker's gRPC client.
 *
 * Only data crosses: requests and answers are structured-cloned messages of strings, numbers and byte arrays.
 */
import { parentPort } from 'node:worker_threads';
import type { HistoryEvent, RunInit } from './context.js';
import type { ReplayActivation } from './history.js';
import { CodeRun, type CompletionAnswer } from './run.js';
import { WorkflowBundle } from './sandbox.js';

export type ThreadRequest =
  | { op: 'bundle'; bundleKey: string; code: string; filename: string }
  | { op: 'forgetBundle'; bundleKey: string }
  | { op: 'create'; runKey: string; bundleKey: string; init: RunInit; activationDeadlineMs?: number }
  | { op: 'replay'; runKey: string; activation: ReplayActivation }
  | { op: 'activate'; runKey: string; window: HistoryEvent[]; started: ReplayActivation['started'] }
  | { op: 'commit'; runKey: string; answer: CompletionAnswer }
  | { op: 'abandon'; runKey: string }
  | { op: 'bodyDigests'; runKey: string }
  | { op: 'drop'; runKey: string };

export interface RunState {
  windowStart: number;
  acceptedCommands: number;
  chain: Uint8Array;
  broken: boolean;
  closed: boolean;
  deferredBodyDigest: Uint8Array | undefined;
}

const bundles = new Map<string, WorkflowBundle>();
const runs = new Map<string, CodeRun>();

function stateOf(run: CodeRun): RunState {
  return {
    windowStart: run.windowStart, acceptedCommands: run.acceptedCommands, chain: run.chain, broken: run.broken, closed: run.closed,
    deferredBodyDigest: run.deferredBodyDigest,
  };
}

function runOf(runKey: string): CodeRun {
  const run = runs.get(runKey);
  if (!run) throw new Error(`the workflow thread holds no run ${runKey}`);
  return run;
}

function handle(request: ThreadRequest): unknown {
  switch (request.op) {
    case 'bundle': {
      const bundle = bundles.get(request.bundleKey) ?? new WorkflowBundle(request.code, request.filename);
      bundles.set(request.bundleKey, bundle);
      return { workflowTypes: [...bundle.workflowTypes] };
    }
    case 'forgetBundle':
      bundles.delete(request.bundleKey);
      return {};
    case 'create': {
      const bundle = bundles.get(request.bundleKey);
      if (!bundle) throw new Error(`the workflow thread holds no bundle ${request.bundleKey}`);
      const options = request.activationDeadlineMs === undefined ? {} : { activationDeadlineMs: request.activationDeadlineMs };
      const run = new CodeRun(bundle, request.init, options);
      runs.set(request.runKey, run);
      return { state: stateOf(run) };
    }
    case 'replay': {
      const run = runOf(request.runKey);
      return { step: run.replay(request.activation), state: stateOf(run) };
    }
    case 'activate': {
      const run = runOf(request.runKey);
      return { answer: run.activate(request.window, request.started), state: stateOf(run) };
    }
    case 'commit': {
      const run = runOf(request.runKey);
      run.commit(request.answer);
      return { state: stateOf(run) };
    }
    case 'abandon': {
      const run = runOf(request.runKey);
      run.abandon();
      return { state: stateOf(run) };
    }
    case 'bodyDigests':
      return { digests: [...runOf(request.runKey).bodyDigests] };
    case 'drop':
      runs.delete(request.runKey);
      return {};
  }
}

if (parentPort) {
  const port = parentPort;
  // A promise that workflow code rejects and never handles (the losing branch of a race, a cancelled sleep) reaches
  // this thread's unhandled-rejection handler, whose default ends the thread with every run it holds. Only the
  // contexts' code runs here, and its unhandled rejections are part of its own control flow: they are ignored.
  process.on('unhandledRejection', () => undefined);
  port.on('message', (message: { id: number; request: ThreadRequest }) => {
    try {
      port.postMessage({ id: message.id, ok: handle(message.request) });
    } catch (error) {
      const name = error instanceof Error ? error.name : 'Error';
      const text = error instanceof Error ? error.message : 'the workflow thread failed';
      port.postMessage({ id: message.id, error: { name, message: text } });
    }
  });
}
