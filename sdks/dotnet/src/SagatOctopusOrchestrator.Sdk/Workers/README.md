# Workers

## Overview

Worker registration, lease reception, handler lifecycle, capabilities and authenticated gRPC transport. [WorkerHost.cs:WorkerHost] [WorkerRpc.cs:WorkerRpc]

## Interface

- IWorkerHost and WorkerHost expose registration/lifecycle; HeartbeatIntervalFor computes pacing. ActivityExecutionContext, TaskKind, CancellationReasons, ExecutionScope and ExecutionInstanceIdentity describe handler context. [IWorkerHost.cs:IWorkerHost] [WorkerHost.cs:HeartbeatIntervalFor] [ActivityExecutionContext.cs:ActivityExecutionContext]
- ActivityFailure provides Retryable/Permanent/OutcomeUnknown; ActivitySecrets supplies attempt secrets; BlobDescriptor/BlobWriteOptions/BlobOutput model blob I/O; ReconciliationEffect/Verdict model reconciliation. [ActivityFailure.cs:ActivityFailure] [ActivitySecrets.cs:ActivitySecrets] [Blobs.cs:BlobDescriptor] [Blobs.cs:BlobWriteOptions] [Blobs.cs:BlobOutput] [Verdict.cs:Verdict]
- Internal boundaries include registrations/result variants, TaskRuntime, gRPC error mapping, WorkerRpc and blob integrity helpers. [Registrations.cs:ActivityRegistration] [TaskRuntime.cs:TaskRuntime] [GrpcErrors.cs:Map] [WorkerRpc.cs:WorkerRpc] [Blobs.cs:Blobs]

## Invariants

- The host advertises 1.5; scoped identity requires minor 5, task kinds/secrets minor 2 and blobs minor 3. Stale/conflicting heartbeats cancel the handler and suppress reporting. [../SdkInfo.cs:ProtocolVersion] [WorkerHost.cs:ExecuteInAsync] [WorkerHost.cs:BeatAsync]
- Reports use task-derived keys and at most five safe retries; stale attempts stop reporting. Secret bytes cache successful resolution, are copied to callers and wiped on handler completion. Blob reads require a matching initial header, bounded chunks, exact size and SHA-256 before clean exhaustion. [WorkerHost.cs:WithRetriesAsync] [WorkerHost.cs:SecretsLoader] [ActivitySecrets.cs:Dispose] [Blobs.cs:ReadAsync]

## Side Effects

- Creates TLS-configured gRPC channels, sends unary/stream RPCs and starts polling/heartbeat/handler tasks. Invokes user handlers, logs/traces/meters, consumes/yields blob streams and clears secret buffers. [WorkerRpc.cs:WorkerRpc] [WorkerHost.cs:RunLoopAsync] [WorkerHost.cs:RunAndReportAsync] [Blobs.cs:WriteAsync] [ActivitySecrets.cs:Dispose]

## Verification

Run from the repository root:

```sh
cd sdks/dotnet && dotnet test && dotnet format --verify-no-changes
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
