# workflows

## Overview

Workflow as code for TypeScript (design 044, Task 044-10): the determinism runtime that runs a workflow function inside
a `node:vm` context per run, the host side that drives its activations and computes the command chain, the replayer of
exported histories and the test environment. [context.ts:WorkflowContext] [run.ts:CodeRun] [replayer.ts:Replayer] [testing.ts:WorkflowTestEnvironment]

## Interface

- The workflow API (`activity`, `script`, `sleep`, `waitForEvent`, `child`, `approval`, `saga`, `requestCancellation`, `onSignal`, `signal`, `onUpdate`, `setState`, `patched`, `deprecatePatch`, `sideEffect`, `now`, `random`, `randomInt`, `uuid`, `continueAsNew`, `cancellation`, `acknowledgeCancellation`, `continueAsNewSuggested`, `historyLimitReached`) and the errors it throws, exported by `@sagat-octopus-orchestrator/sdk/workflows` and passed to each workflow function as its first argument. [context.ts:WorkflowContext] [index.ts]
- In a bundle the same import resolves, by the export condition `sagat-workflow`, to `context.ts` alone. [../../package.json]
- `CodeRun` replays completed activations against their records, runs the live one and prepares its completion (body digests, the replayed chain, the values that travel as blobs); `ThreadedRun` is its face on the workflow thread; `WorkflowRunCache` reuses a run only when the lease's window starts where it stopped. [run.ts:CodeRun] [threaded.ts:ThreadedRun] [cache.ts:WorkflowRunCache]
- `Replayer.replay` reads an export (a header line, then one event per line) and answers the verdict of the first difference; `WorkflowTestEnvironment` runs a function against an in-memory journal with stubbed activities, signals, updates and cancellation. [replayer.ts:Replayer] [testing.ts:WorkflowTestEnvironment]

## Invariants

- A context is created with `vm.constants.DONT_CONTEXTIFY`, `microtaskMode: 'afterEvaluate'` and no code generation from strings or WebAssembly; no host object is placed in it: one number before the bundle, one primitive string per call, one primitive string back. [sandbox.ts:WorkflowSandbox]
- Inside, `Date` and `Date.now` give the time of the activation's last `workflow_task.started`; `Math.random`, `random`, `randomInt` and `uuid` draw from xoshiro256** seeded by the run; timers are durable timer commands; the locale is `en-US` and the default time zone UTC; `WeakRef`, `FinalizationRegistry`, `SharedArrayBuffer` and `Atomics` are removed. A module-level read of the clock or the generator is refused. [context.ts:installGlobals]
- Each event of the window is applied in its own evaluation, so the context's microtasks drain completely between two events; an activation run that does not yield within its deadline (2 s by default, at least 100 ms) fails `ACTIVATION_DEADLINE` and its context is dropped. [run.ts:CodeRun]
- Contexts live on a worker thread started with no inherited command-line options, which ignores the unhandled rejections of workflow code. [thread.ts] [threaded.ts:WorkflowThread]
- A replay compares each command's kind and target with the record of its number, and a deferred terminal command with its deferral; a difference fails `NONDETERMINISM` with the command number and the actual kind and target, or answers `INCONCLUSIVE_REDACTED` once a withheld value was given. [context.ts:Run.issue]
- An activation's commands come in the order of `research.md` Decision 044-305; bodies follow the corpus table of Decision 044-232; the bounds of Requirement 044-012 and Non-functional requirement 044-006 throw `CommandBatchLimitExceeded` or `InlineValuesExceeded` at the same call live and in replay. [context.ts:Run.finishActivation] [context.ts:Run.admitInlineValue]
- Loading the host entry point checks Node 22.8 or later; registering workflow types needs the process time zone UTC. [preconditions.ts:checkWorkflowRuntime]

## Side Effects

- Creates one worker thread per `WorkflowThread` (one shared by the replayer and the test environment), `node:vm` contexts in it, and reads bundle files. [threaded.ts:WorkflowThread]

## Verification

Run from the repository root:

```sh
cd sdks/typescript && npm ci --ignore-scripts && npm test
```

The workflow tests run with `TZ=UTC` (`dist/test/workflows/*.test.js`).
