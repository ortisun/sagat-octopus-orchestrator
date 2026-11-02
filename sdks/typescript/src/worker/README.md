# worker

## Overview

Activity contracts, authenticated gRPC transport and worker lifecycle. [activity.ts:ActivityContext] [grpc.ts:WorkerRpc] [host.ts:WorkerHost]

## Interface

- Activity APIs include TaskKind, BlobDescriptor, BlobOutput, ActivityFailure, Verdict, reconcile, Secrets, ExecutionInstanceIdentity, ActivityContext, ActivityHandler and cancellation detection. [activity.ts:ActivityContext] [activity.ts:ActivityFailure] [activity.ts:reconcile] [activity.ts:isCancellation]
- WorkerConfig, WorkerEvent and WorkerHost configure identity, namespace, inflight limits, TLS, protocol/transport, logs and events; the host registers activities, compensations and reconcilers and exposes start, stop and protocolVersion. [host.ts:WorkerConfig] [host.ts:WorkerEvent] [host.ts:WorkerHost]
- WorkerRpc is the intra-SDK adapter for registration, polling/streaming, heartbeat, completion/failure, secrets and blobs; it is not exported by the package entry point. [grpc.ts:WorkerRpc] [../index.ts:WorkerHost]

## Invariants

- The host requires 1–1000 inflight slots, at least one handler and unique registrations made before start. [host.ts:WorkerHost]
- The host offers protocol 1.5; task kind/secrets require 1.2, blobs 1.3, streams 1.4 and scoped identity 1.5. The original lease identity remains the identity used for reports and capabilities. [../config.ts:PROTOCOL_VERSION] [host.ts:minorOf] [host.ts:instanceOf] [host.ts:inflightKey]
- Heartbeats start before handlers and continue through reporting. Lost leases suppress reports; completion/failure retries use stable done-/failed- keys and at most five attempts. Cancellation remains cooperative. [host.ts:execute] [host.ts:beat] [host.ts:withRetries] [host.ts:complete] [host.ts:fail]
- Secret buffers are invalidated and wiped after handlers. Blob streams are attempt-scoped, use chunks at most 256 KiB and check ordering, size and SHA-256 before acceptance. Ordinary handler exceptions expose their type rather than their text; purposeful failure text is bounded. [host.ts:resolveSecrets] [host.ts:writeBlob] [host.ts:readBlob] [host.ts:safeMessage]
- gRPC URLs accept http/https only; HTTPS to a literal IP requires tls.serverName. Each RPC resolves its bearer token and applies a deadline. [grpc.ts:checkGrpcConfig] [grpc.ts:WorkerRpc]

## Side Effects

- WorkerRpc reads the packaged proto, manages a gRPC channel, calls token providers, sends unary/streaming RPCs and cancels pending calls. [grpc.ts:loadService] [grpc.ts:WorkerRpc]
- The host invokes application handlers, schedules receive/backoff/heartbeat timers, emits logs/events and buffers then wipes secrets. [host.ts:run] [host.ts:serve] [host.ts:beat] [host.ts:resolveSecrets]

## Verification

Run from the repository root:

```sh
cd sdks/typescript && npm test
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
