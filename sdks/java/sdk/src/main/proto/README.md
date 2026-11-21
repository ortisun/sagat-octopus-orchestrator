# proto

## Overview

Proto3 worker wire schema. [worker.proto:WorkerService]

## Interface

- `WorkerService` declares RegisterWorker, PollTasks, StreamTasks, Heartbeat, CompleteStep, FailStep, ResolveSecrets, ResolveDatabase, WriteBlob and ReadBlob. [worker.proto:WorkerService]
- Messages define registration/capabilities, polling/task leases, scope paths/instance identity, heartbeat, completion/failure, secrets, blob headers/chunks, step-command results and typed database configuration. DatabaseOperationKind and DatabaseTlsMode are enums. [worker.proto:WorkerCapabilities] [worker.proto:TaskLease] [worker.proto:ExecutionInstanceIdentity] [worker.proto:HeartbeatRequest] [worker.proto:CompleteStepRequest] [worker.proto:FailStepRequest] [worker.proto:ResolveSecretsRequest] [worker.proto:WriteBlobRequest] [worker.proto:ReadBlobResponse] [worker.proto:StepCommandResponse] [worker.proto:ResolvedDatabase] [worker.proto:DatabaseOperationKind] [worker.proto:DatabaseTlsMode]

## Invariants

- The schema fixes proto3, sagat_octopus_orchestrator.v1 and Java package dev.sagatoctopusorchestrator.protocol.v1. Blob streams use header/chunk oneofs; lease-bound requests carry attempt/generation/lease-token fields. The schema declares these fields; server enforcement is outside this directory. [worker.proto:syntax] [worker.proto:java_package] [worker.proto:WriteBlobRequest] [worker.proto:ReadBlobResponse] [worker.proto:CompleteStepRequest]

## Side Effects

- None

## Verification

Run from the repository root:

```sh
cd sdks/java && ./mvnw -q verify
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
