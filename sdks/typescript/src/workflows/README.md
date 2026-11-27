# workflows

## Overview

Workflow as code for TypeScript (design 044, Task 044-10): the determinism runtime that runs a workflow function inside
a `node:vm` context per run, the host side that drives its activations and computes the command chain, the replayer of
exported histories and the test environment. [context.ts:WorkflowContext] [run.ts:CodeRun] [replayer.ts:Replayer] [testing.ts:WorkflowTestEnvironment]

## Interface

- The workflow API (`activity`, `script`, `sleep`, `waitForEvent`, `child`, `approval`, `saga`, `requestCancellation`, `onSignal`, `signal`, `onUpdate`, `setState`, `patched`, `deprecatePatch`, `sideEffect`, `now`, `random`, `randomInt`, `uuid`, `continueAsNew`, `cancellation`, `acknowledgeCancellation`, `continueAsNewSuggested`, `historyLimitReached`) and the errors it throws, exported by `@sagat-octopus-orchestrator/sdk/workflows` and passed to each workflow function as its first argument. [context.ts:WorkflowContext] [index.ts]
- In a bundle the same import resolves, by the export condition `sagat-workflow`, to `context.ts` alone. [../../package.json]
- `CodeRun` replays completed activations against their records, runs the live one and prepares its completion (body digests, the replayed chain, the values that travel as blobs); `ThreadedRun` is its face on the workflow thread; `WorkflowRunCache` reuses a run only when the lease's window starts where it stopped. [run.ts:CodeRun] [threaded.ts:ThreadedRun] [cache.ts:WorkflowRunCache]
- `protocol.ts` is the one adapter between the worker protocol (`worker.proto` 1.10) and the runtime's model: history events from the gRPC loader's form and from the proto3 JSON mapping of an export line, back to that mapping, and the commands and failure reports a workflow task answers. [protocol.ts:eventFromWire] [protocol.ts:eventFromProto3Json] [protocol.ts:commandToWire] [protocol.ts:failureToWire]
- `Replayer.replay` reads an export (a header line, then one event per line) and answers the verdict of the first difference; `WorkflowTestEnvironment` runs a function against an in-memory journal with stubbed activities, signals, updates and cancellation, starts a run delayed or with a signal or an update, cancels at once only a step whose attempt has not begun (`beginStep` marks one that a worker took), applies a completion's cancellations before any stub runs, and settles each kind of step with its own event (`completeStep`, `failStep`, `decideApproval`). [replayer.ts:Replayer] [testing.ts:WorkflowTestEnvironment]

## Invariants

- A context is created with `vm.constants.DONT_CONTEXTIFY`, `microtaskMode: 'afterEvaluate'` and no code generation from strings or WebAssembly; no host object is placed in it: one number before the bundle, one primitive string per call, one primitive string back. [sandbox.ts:WorkflowSandbox]
- Inside, `Date` and `Date.now` give the time of the activation's last `workflow_task.started`; `Math.random`, `random`, `randomInt` and `uuid` draw from xoshiro256** seeded by the run; timers are durable timer commands; the locale is `en-US` and the default time zone UTC; `WeakRef`, `FinalizationRegistry`, `SharedArrayBuffer` and `Atomics` are removed. A module-level read of the clock or the generator is refused. [context.ts:installGlobals]
- Each event of the window is applied in its own evaluation, so the context's microtasks drain completely between two events; an activation run that does not yield within its deadline (2 s by default, at least 100 ms) fails `ACTIVATION_DEADLINE` and its context is dropped. [run.ts:CodeRun]
- Contexts live on a worker thread started with `--experimental-vm-modules` only (no inherited option), no environment, a heap bound and a bound of its memory outside the heap checked after every evaluation (`ExternalMemoryExceeded`, research Decision 044-323); it ignores the unhandled rejections of workflow code, and each request has a watchdog that ends the thread rather than hang. [thread.ts] [threaded.ts:WorkflowThread]
- A dynamic `import()` gets a primitive string, never an error of the host's realm; the two properties the host writes are non-configurable data properties; a host error carries a checked error name, never a message of the context; stack traces show the bundle's frames only. [sandbox.ts:WorkflowSandbox] [context.ts:installStackTraces]
- Every value is taken at its call as canonical text; the host checks every field of the context's answer and the bounds again, and fails the workflow task `NONDETERMINISM_RISK` on an answer it refuses. [context.ts:snapshot] [run.ts:checkResult] [run.ts:prepareChecked]
- A replay compares each command's kind and target with the record of its number (and, for an activity or a script, the saga scope the record carries), and a deferred terminal command with its deferral; a difference fails `NONDETERMINISM` with the command number and the actual kind, target and saga scope, or answers `INCONCLUSIVE_REDACTED` once a withheld value was given. [context.ts:Run.issue]
- A scheduled activity or script hashes its saga scope in its body (`"scope": "s-<n>"`); a cancelled activity, script, wait or approval rejects with `CommandCancelled` only from its outcome in the history, a cancelled sleep with `TimerCancelled` once its `timer_cancel` is made. [chain.ts:hashedBody] [context.ts:stepOutcome]
- An activation's commands come in the order of `research.md` Decision 044-305; bodies follow the corpus table of Decision 044-232; the bounds of Requirement 044-012 and Non-functional requirement 044-006 throw `CommandBatchLimitExceeded` or `InlineValuesExceeded` at the same call live and in replay. [context.ts:Run.finishActivation] [context.ts:Run.admitInlineValue]
- A side effect's computation runs live only, makes no command, and leaves the run's generator where a replay finds it; a handler, a validator and a nested scope follow `research.md` Decision 044-316. [context.ts:sideEffect]
- Loading the host entry point checks Node 22.8 or later; registering workflow types needs the process time zone UTC. [preconditions.ts:checkWorkflowRuntime]

## Side Effects

- Creates one worker thread per `WorkflowThread` (one shared by the replayer and the test environment), `node:vm` contexts in it, and reads bundle files. [threaded.ts:WorkflowThread]

## Verification

Run from the repository root:

```sh
cd sdks/typescript && npm ci --ignore-scripts && npm test
```

The workflow tests run with `TZ=UTC` and `--experimental-vm-modules` (`dist/test/workflows/*.test.js`).
