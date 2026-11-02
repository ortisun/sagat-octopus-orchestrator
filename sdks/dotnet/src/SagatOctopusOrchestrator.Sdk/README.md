# SagatOctopusOrchestrator.Sdk

## Overview

Public .NET REST client, value contracts, configuration, canonical errors/JSON and child worker/builder integrations. [SagatClient.cs:SagatClient]

## Interface

- ISagatClient and SagatClient expose workflow resolution, input upload, run lifecycle/waits, approvals, declared queries/updates, run/chain signals/commands/paging/waits, event publication and journal reads. RunInputBlobOptions configures uploads; MeasuredBlobContent is internal. [ISagatClient.cs:ISagatClient] [SagatClient.cs:SagatClient] [RunInputBlobs.cs:RunInputBlobOptions] [RunInputBlobs.cs:MeasuredBlobContent]
- Models cover statuses, parent/run/chain/page/action/outcome/operation/approval/query/update/signal/event/journal contracts and request/start/list/operate/wait options. CanonicalJson.Write, RetryPolicy/Backoff, SagatErrorCode/SagatErrorCodes/SagatException and Traceparent supply shared behavior. [Models.cs:Run] [Models.cs:RequestOptions] [CanonicalJson.cs:Write] [RetryPolicy.cs:Backoff] [SagatErrorCode.cs:SagatErrorCodes] [SagatException.cs:SagatException] [Traceparent.cs:Traceparent]
- SagatOptions, WorkerOptions, WorkerTransport and SagatTlsOptions configure the SDK; SdkInfo declares SDK/protocol/language/tracing constants. The project targets net8.0/net10.0 and generates internal protobuf clients. [SagatOptions.cs:SagatOptions] [SdkInfo.cs:SdkInfo] [SagatOctopusOrchestrator.Sdk.csproj:TargetFrameworks] [SagatOctopusOrchestrator.Sdk.csproj:Access="Internal"]

- [DependencyInjection](./DependencyInjection/README.md) — component contract.
- [Internal](./Internal/README.md) — component contract.
- [Workers](./Workers/README.md) — component contract.
- [Workflows](./Workflows/README.md) — component contract.

## Invariants

- The host offers protocol 1.5; copied schema ResolveDatabase is additive 1.6 and has no ordinary WorkerRpc wrapper. Retry classification accepts eligible Transport/Unavailable/RateLimited/Internal; caller cancellation remains OperationCanceledException. [SdkInfo.cs:ProtocolVersion] [Workers/WorkerRpc.cs:WorkerRpc] [SagatException.cs:SagatException]
- Canonical JSON sorts keys ordinally and rejects lone surrogates, non-finite values and digit-form integers beyond 2^53-1. Client paths are validated locally and result models retain raw JSON; server authorization/durability is outside these local checks. [CanonicalJson.cs:Write] [SagatClient.cs:RunPath] [Models.cs:Run]

## Side Effects

- Performs REST I/O, consumes upload streams once, caches workflow resolutions, generates request IDs and polls with jitter. Child packages add worker I/O, tasks, DI, logs, metrics and traces. [SagatClient.cs:ResolveWorkflowAsync] [RunInputBlobs.cs:UploadRunInputAsync] [Workers/WorkerHost.cs:WorkerHost] [DependencyInjection/ServiceCollectionExtensions.cs:AddSagatOctopusOrchestrator]

## Verification

Run from the repository root:

```sh
cd sdks/dotnet && dotnet test && dotnet format --verify-no-changes
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
