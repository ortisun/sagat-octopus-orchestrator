# src

## Overview

Crate facade for async REST, worker hosting, typed responses, capabilities, TLS/tokens, telemetry and workflow authoring. [lib.rs:Client] [lib.rs:WorkerHost] [lib.rs:workflow]

## Interface

- Client configuration/request types include RetryPolicy, RunDeadline, StartOptions, StepOutcome, OperationOptions, PageOptions, ListOptions, WaitOptions, UpdateRequest, PublishEvent, ClientOptions and CallOptions. Client provides construction, workflow resolution, run lifecycle, signals/list/journal/waits, declared queries/updates and events, extended by the client child modules. [client.rs:Client] [client.rs:RetryPolicy] [client.rs:CallOptions]
- Activity contracts include ExecutionInstanceIdentity, TaskKind, TaskInfo, CancellationReason, ActivityContext, ActivityFailure and Verdict; WorkerTransport, WorkerOptions and WorkerHost provide handler registration, lifecycle, protocol_version and wait_for_failure. [activity.rs:ActivityContext] [activity.rs:ActivityFailure] [activity.rs:Verdict] [worker.rs:WorkerTransport] [worker.rs:WorkerOptions] [worker.rs:WorkerHost]
- Typed wire results cover RunStatus/CompletionKind, run/scope/chain/parent/page/journal/query/update/event/approval contracts and BlobDescriptor. Additional public contracts are SagatError, TlsOptions/TlsIdentity, TokenProviderError/TokenSource, BlobOutput/WriteBlobOptions, SecretSet/SecretValue and EventKind/SdkTransport/SdkEvent/Telemetry. [types.rs:RunStatus] [types.rs:BlobDescriptor] [error.rs:SagatError] [tls.rs:TlsOptions] [tls.rs:TlsIdentity] [token.rs:TokenSource] [blob.rs:BlobOutput] [blob.rs:WriteBlobOptions] [capabilities.rs:SecretSet] [capabilities.rs:SecretValue] [telemetry.rs:Telemetry]
- The workflow module exposes Builder, base retry/step/saga options and canonical_json, with typed extensions in the workflow child directory. Internal boundaries include Rpc/error_of, Finite/encode, Capabilities/AttemptGuard and typed parsers. [workflow.rs:Builder] [workflow.rs:canonical_json] [worker_rpc.rs:Rpc] [json.rs:encode] [capabilities.rs:Capabilities] [capabilities.rs:AttemptGuard]

- [capabilities](./capabilities/README.md) — component contract.
- [client](./client/README.md) — component contract.
- [workflow](./workflow/README.md) — component contract.

## Invariants

- Only repeatable requests retry with stable serialized body and key; REST responses are bounded, redirects rejected and expected status/shape checked. Typed responses retain additive raw JSON and represent unknown run statuses. [client.rs:keyed] [client.rs:execute] [client.rs:consume_response] [types.rs:RunStatus]
- Ordinary hosts default to protocol 1.5; registration can understand additive minor 1.6, but no public ordinary-host database-runner method implements the schema's ResolveDatabase. Streams require minor 4 and scoped identity minor 5. [worker.rs:WorkerOptions] [worker.rs:register] [worker.rs:dispatch] [../proto/worker.proto:ResolveDatabase]
- Secrets require minor 2 and active attempts, cache successful resolution and invalidate after handlers; get returns an owned zeroizing copy. JSON handler output is finite and bounded to 256 KiB. [capabilities.rs:secrets] [capabilities.rs:SecretSet] [worker.rs:activity] [json.rs:encode]
- TLS retains chain/name checks with platform and configured roots. Telemetry hooks are panic-isolated and events contain bounded operation/code/kind/transport/duration fields. [tls.rs:configuration] [telemetry.rs:emit] [telemetry.rs:SdkEvent]

## Side Effects

- Clients perform authenticated HTTP requests and resolve dynamic tokens per attempt; successful name resolutions cache for 60 seconds. Non-loopback plaintext configuration emits tracing warnings. [client.rs:once] [client.rs:resolve_workflow] [client.rs:new]
- WorkerHost connects/registers over gRPC, receives leases, starts Tokio handler/heartbeat tasks, invokes handlers and reports results. [worker.rs:start] [worker.rs:report]

## Verification

Run from the repository root:

```sh
cd sdks/rust && cargo test --locked
```

Use a private `CARGO_TARGET_DIR` outside the repository, as required by [`sdks/AGENTS.md`](../../AGENTS.md).

These commands are documented from the existing build configuration; they were not executed during this documentation update.
