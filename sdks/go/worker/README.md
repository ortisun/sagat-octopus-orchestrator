# worker

## Overview

Activity, compensation and reconciliation worker host over gRPC. [host.go:Host]

## Interface

- Host construction/lifecycle, registration and options cover transport, inflight slots, languages/pools, TLS/retry/heartbeat, logging/events and dial customization. [host.go:New] [host.go:Start] [host.go:Stop] [host.go:RegisterActivity] [host.go:RegisterCompensation] [host.go:RegisterReconciler] [host.go:WithTransport] [host.go:WithDialOptions]
- Handler, ReconcilerHandler, Typed, TypedReconciler, Task, TaskFrom, Retryable, Permanent, OutcomeUnknown, Verdict and Void form the handler API. Task exposes attempt secrets and blob I/O through Secrets, BlobDescriptor, WriteBlobOptions and NewBlobOutput. [activity.go:Handler] [activity.go:Typed] [activity.go:TypedReconciler] [activity.go:Task] [activity.go:TaskFrom] [activity.go:Retryable] [activity.go:Permanent] [activity.go:OutcomeUnknown] [activity.go:Verdict] [activity.go:Void] [activity.go:Secrets] [blob.go:BlobDescriptor] [blob.go:WriteBlobOptions] [blob.go:NewBlobOutput]

## Invariants

- The host retains original task/attempt/generation/token identity and scoped instance identity from protocol 1.5 for reports. Heartbeats propagate cancellation/deadlines; stale or conflicting leases cancel with ErrLeaseLost and suppress results. [task.go:instanceOf] [task.go:execute] [task.go:complete] [task.go:beat]
- Reports use deterministic keys and at most five attempts for safe failures. Successful secret resolution is cached per attempt; copied/redacted buffers are wiped and invalidated after the handler. [host.go:reportAttempts] [task.go:report] [activity.go:Secrets] [activity.go:wipe]
- Inline output caps at 256 KiB. Blob streams enforce part order, chunk bounds, size/digest and clean EOF. Ordinary handler failures report type names, while deliberate failure codes/text are validated and bounded. [host.go:maxOutputBytes] [blob.go:maxBlobChunk] [blob.go:verifyingBlobReader] [task.go:failureOf] [task.go:safeMessage]

## Side Effects

- Creates gRPC channels and receive/heartbeat/handler goroutines; invokes handlers, emits concurrent lifecycle events and makes registration/task/report/secret/blob RPCs. Stop drains handlers then may cancel with ErrShutdown. [host.go:Start] [loop.go:run] [stream.go:receive] [task.go:execute] [task.go:beat] [host.go:emit] [host.go:Stop]

## Verification

Run from the repository root:

```sh
cd sdks/go && go test -race ./... && go vet ./...
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
