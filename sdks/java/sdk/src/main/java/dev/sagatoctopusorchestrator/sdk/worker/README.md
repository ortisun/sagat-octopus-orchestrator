# worker

## Overview

Worker host, handler contracts and gRPC capabilities. [WorkerHost.java:WorkerHost]

## Interface

- `WorkerHost` registers activities, compensations and reconcilers and exposes `start`, `stop`, `close`, and `protocolVersion`. `WorkerConfig.Builder` configures endpoint, identity, capabilities, transport, TLS, retry/deadline, heartbeat, interceptors, events and wait caps; `WorkerTransport` is AUTO, POLL or STREAM. [WorkerHost.java:WorkerHost] [WorkerConfig.java:WorkerConfig] [WorkerTransport.java:WorkerTransport]
- Handler contracts: `ActivityHandler`, `ActivityContext`, `ActivityLogger`, `ActivityFailure`, `CancellationSignal`, `CancelReason`, `TaskKind`, `Secrets`, `Verdict`. [ActivityHandler.java:ActivityHandler] [ActivityContext.java:ActivityContext] [ActivityLogger.java:ActivityLogger] [ActivityFailure.java:ActivityFailure] [CancellationSignal.java:CancellationSignal] [CancelReason.java:CancelReason] [TaskKind.java:TaskKind] [Secrets.java:Secrets] [Verdict.java:Verdict]
- Blob and identity contracts: `BlobDescriptor`, `BlobOutput`, `BlobWriteOptions`, `ExecutionInstanceIdentity`, `ExecutionScope`. [BlobDescriptor.java:BlobDescriptor] [BlobOutput.java:BlobOutput] [BlobWriteOptions.java:BlobWriteOptions] [ExecutionInstanceIdentity.java:ExecutionInstanceIdentity] [ExecutionScope.java:ExecutionScope]
- `WorkerEvent` variants cover Registered, Poll, TaskTaken, TaskCompleted, TaskFailed, TaskStale, TaskDropped, Heartbeat, ReceiveFailed and Reconnect. [WorkerEvent.java:WorkerEvent]

## Invariants

- Handler names are unique and registered before start; the host requires 1–256 handlers and maxInflight of 1–1000. [WorkerHost.java:add] [WorkerHost.java:start] [WorkerConfig.java:build]
- Inflight identity includes task ID and negotiated scope path; heartbeat starts before the handler and continues through reporting. Stale/lost leases suppress reports; reports reuse deterministic done-/failed- keys and at most five attempts. Inline output is limited to 256 KiB. [WorkerHost.java:inflightKey] [WorkerHost.java:execute] [WorkerHost.java:Beat] [WorkerHost.java:withRetries] [WorkerHost.java:MAX_REPORT_ATTEMPTS] [WorkerHost.java:MAX_OUTPUT_BYTES]
- Secrets resolve lazily and are cleared after the handler. Blob reads require a header, chunks no larger than 256 KiB, and exact size/SHA-256 before clean EOF. [WorkerHost.java:dropSecrets] [Secrets.java:drop] [BlobInputStream.java:read] [BlobInputStream.java:verifyEnd]

## Side Effects

- The host opens gRPC channels, creates poll/task/heartbeat threads, invokes handlers and callbacks, emits logs, heartbeats and reports. [WorkerHost.java:start] [WorkerHost.java:emit] [WorkerHost.java:complete] [WorkerHost.java:fail]
- Activity capabilities resolve remote secrets and stream blobs; writes consume their source once and closing a read can cancel its RPC. Handler logger messages are caller-owned and must not contain sensitive values. [WorkerHost.java:resolveSecrets] [WorkerHost.java:writeBlob] [WorkerHost.java:readBlob] [WorkerHost.java:logger] [BlobInputStream.java:close]

## Verification

Run from the repository root:

```sh
cd sdks/java && ./mvnw -q verify
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
