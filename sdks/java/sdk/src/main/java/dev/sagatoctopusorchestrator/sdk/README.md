# sdk

## Overview

REST client, configuration, errors, serialization and value contracts. [SagatClient.java:SagatClient]

## Interface

- `SagatClient` provides workflow resolution; run start/upload/get/list/operate/cancel/rerun/wait; approval listing/decisions; declared queries/updates; signals; chain inspection/commands/waits; event publication and journal reads. Async wrappers exist where declared; `resolveWorkflow` has no asynchronous counterpart. [SagatClient.java:SagatClient] [SagatClient.java:resolveWorkflow]
- Configuration and utilities: `ClientConfig`, `RetryPolicy`, `TlsOptions`, `TokenProvider`, `HttpTransport`, `JdkHttpTransport`, `Traceparents`, `Endpoints`, `Json`, `Text`, `Hooks`, `SagatSdk`, `SagatException`, `ErrorCode`, `ErrorCatalog`. [ClientConfig.java:ClientConfig] [RetryPolicy.java:RetryPolicy] [TlsOptions.java:TlsOptions] [TokenProvider.java:TokenProvider] [HttpTransport.java:HttpTransport] [JdkHttpTransport.java:JdkHttpTransport] [Traceparents.java:Traceparents] [Endpoints.java:Endpoints] [Json.java:Json] [Text.java:Text] [Hooks.java:Hooks] [SagatSdk.java:SagatSdk] [SagatException.java:SagatException] [ErrorCode.java:ErrorCode] [ErrorCatalog.java:ErrorCatalog]
- Call options: `ApprovalDecisionOptions`, `ApprovalListOptions`, `CallOptions`, `ChainCommandOptions`, `ChainQueryOptions`, `ChainRunListOptions`, `ChainUpdateOptions`, `EventInput`, `ListOptions`, `OperateOptions`, `StartOptions`, `UpdateOptions`, `WaitOptions`. [ApprovalDecisionOptions.java:ApprovalDecisionOptions] [ApprovalListOptions.java:ApprovalListOptions] [CallOptions.java:CallOptions] [ChainCommandOptions.java:ChainCommandOptions] [ChainQueryOptions.java:ChainQueryOptions] [ChainRunListOptions.java:ChainRunListOptions] [ChainUpdateOptions.java:ChainUpdateOptions] [EventInput.java:EventInput] [ListOptions.java:ListOptions] [OperateOptions.java:OperateOptions] [StartOptions.java:StartOptions] [UpdateOptions.java:UpdateOptions] [WaitOptions.java:WaitOptions]
- Value/result types: `ApprovalDecision`, `ApprovalDecisionReceipt`, `ApprovalListItem`, `ApprovalPage`, `ApprovalScope`, `ApprovalStatus`, `ChainInspection`, `ChainRun`, `ChainRunPage`, `ChainSegment`, `EventAccepted`, `JournalEvent`, `JournalPage`, `OperateAction`, `OperationResult`, `QueryResult`, `Run`, `RunInputBlobOptions`, `RunPage`, `RunStatus`, `SignalResult`, `UpdateReceipt`. [ApprovalDecision.java:ApprovalDecision] [ApprovalDecisionReceipt.java:ApprovalDecisionReceipt] [ApprovalListItem.java:ApprovalListItem] [ApprovalPage.java:ApprovalPage] [ApprovalScope.java:ApprovalScope] [ApprovalStatus.java:ApprovalStatus] [ChainInspection.java:ChainInspection] [ChainRun.java:ChainRun] [ChainRunPage.java:ChainRunPage] [ChainSegment.java:ChainSegment] [EventAccepted.java:EventAccepted] [JournalEvent.java:JournalEvent] [JournalPage.java:JournalPage] [OperateAction.java:OperateAction] [OperationResult.java:OperationResult] [QueryResult.java:QueryResult] [Run.java:Run] [RunInputBlobOptions.java:RunInputBlobOptions] [RunPage.java:RunPage] [RunStatus.java:RunStatus] [SignalResult.java:SignalResult] [UpdateReceipt.java:UpdateReceipt]

- [opentelemetry](./opentelemetry/README.md) — component contract.
- [worker](./worker/README.md) — component contract.
- [workflow](./workflow/README.md) — component contract.

## Invariants

- Path IDs/names, printable-ASCII idempotency keys and W3C version-00 traceparent are validated before requests. Safe calls retry only the implemented transport/unavailable/rate-limited/internal classifications and local token-provider failures. [SagatClient.java:runId] [SagatClient.java:pathName] [SagatClient.java:key] [Traceparents.java:isValid] [RestClient.java:request] [SagatException.java:isRetriable]
- HTTP responses are capped at 32 MiB; default transports refuse redirects. Error messages/reasons/waits are bounded and invalid trace IDs discarded. Custom transports remain responsible for their behavior. [HttpTransport.java:MAX_RESPONSE_BYTES] [RestClient.java:once] [SagatException.java:SagatException] [JdkHttpTransport.java:using]
- Canonical JSON sorts UTF-16 keys and rejects non-finite values, lone surrogates and unsafe whole numbers below exponent notation. TLS options accept system/custom roots and PEM PKCS#8 client identities. [Json.java:canonical] [Json.java:canonicalNumber] [Json.java:quote] [TlsOptions.java:withClientCertificate]

## Side Effects

- Performs authenticated REST calls, consumes upload streams, polls waits and creates daemon async threads unless supplied an executor. [RestClient.java:once] [SagatClient.java:uploadRunInput] [SagatClient.java:waitForCompletion] [SagatClient.java:SagatClient]
- HTTP transport buffers bounded bodies; TLS helpers read PEM files and create SSL/gRPC credentials. Endpoint and retry helpers may log warnings. [JdkHttpTransport.java:send] [TlsOptions.java:trusting] [TlsOptions.java:sslContext] [TlsOptions.java:channelCredentials] [Endpoints.java:warnIfPlainText] [RestClient.java:request]

## Verification

Run from the repository root:

```sh
cd sdks/java && ./mvnw -q verify
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
