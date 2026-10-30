# examples

## Overview

Executable client, worker and workflow-authoring usage examples. [start-run.ts:client] [worker.ts:host] [workflow.ts:document]

## Interface

- `start-run.ts` demonstrates start/wait/error handling; `worker.ts` demonstrates handler registration and stop; `workflow.ts` constructs a saga/event/timer document. [start-run.ts:SagatClient] [worker.ts:WorkerHost] [workflow.ts:workflow]

## Invariants

- The worker example passes idempotency/target keys and cancellation signals to its sample provider. The workflow example builds a local document without publishing it. [worker.ts:idempotencyKey] [worker.ts:targetIdempotencyKey] [worker.ts:signal] [workflow.ts:build]

## Side Effects

- The client example makes REST calls and writes console output. The worker example starts gRPC work, invokes its provider and installs SIGTERM handling. The workflow example prints JSON. [start-run.ts:console] [worker.ts:start] [worker.ts:SIGTERM] [workflow.ts:console.log]

## Verification

Run from the repository root:

```sh
cd sdks/typescript && npm test
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
