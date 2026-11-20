# src

## Overview

REST client, workflow/database authoring, shared transport utilities and optional telemetry. [index.ts:SagatClient] [workflow.ts:WorkflowBuilder] [opentelemetry.ts:OpenTelemetryBridge]

## Interface

- SagatClient provides workflow resolution; run start/blob/get/list/operations/waits; approvals; declared run/chain queries and updates; chain inspection/paging/signals/cancellation/waits; event publication and journal reads, with typed result contracts. [client.ts:SagatClient]
- The package entry point exports client/worker/activity APIs, error/retry/trace/JSON helpers, workflow factories and database types. ClientConfig, TlsOptions and TokenSource define caller configuration; SagatError and CATALOG define canonical failures. [index.ts:SagatClient] [config.ts:ClientConfig] [config.ts:TlsOptions] [config.ts:TokenSource] [errors.ts:SagatError] [catalog.ts:CATALOG]
- Flow, Body, WorkflowBuilder and workflow/flow/body factories author activities, scripts, databases, timers, events, children, decisions, collections, loops, approvals, continuations, parallel/saga, terminal nodes, state operations and chain policies. [workflow.ts:Flow] [workflow.ts:Body] [workflow.ts:WorkflowBuilder] [database.ts:databaseNode]
- The separate OpenTelemetryBridge API adapts traceContext and onEvent; trace helpers validate or generate W3C traceparent values. [opentelemetry.ts:OpenTelemetryBridge] [traceparent.ts:isTraceparent] [traceparent.ts:checkTraceparent] [traceparent.ts:newTraceparent]

- [worker](./worker/README.md) — component contract.

## Invariants

- HTTP attempts attach authorization, namespace and SDK user-agent, refuse redirects, enforce timeout/cancellation and cap response bodies at 32 MiB. [http.ts:HttpTransport] [http.ts:readCapped]
- Only safeToRepeat requests with retriable mapped errors repeat. One-pass uploads are not retried and must reach EOF with matching measured size/SHA-256 before returning their descriptor. [http.ts:safeToRepeat] [http.ts:request] [client.ts:uploadRunInput]
- Path IDs, keys and selected response shapes are checked locally; errors bound messages, reasons, trace IDs and retry waits. JSON rejects non-finite numbers, bigint and unsafe integers; canonical output sorts keys without insignificant whitespace. [client.ts:runIdOf] [client.ts:namespaceIdOf] [errors.ts:SagatError] [json.ts:canonicalJson] [json.ts:encodeJson] [json.ts:parseJson]
- Workflow and database builders validate locally visible shape/reference/bound constraints and emit IR; they do not publish workflows or prove server acceptance. [workflow.ts:WorkflowBuilder] [workflow.ts:copyJson] [database.ts:databaseNode]
- TLS transport uses configured roots/client identity/server name without disabling verification. Telemetry contains observer failures and caps open task spans at 1000. [tlsfetch.ts:tlsFetch] [opentelemetry.ts:MAX_OPEN_TASKS] [opentelemetry.ts:onEvent]

## Side Effects

- The client performs REST I/O, consumes upload streams, caches workflow-name resolution for 60 seconds and polls waits. [client.ts:SagatClient] [http.ts:HttpTransport]
- Transport helpers emit retry/plaintext warnings and create a keep-alive HTTP(S) agent. Workflow/database builders mutate local builder state and return documents without platform I/O. [http.ts:request] [config.ts:warnIfNotSecure] [tlsfetch.ts:tlsFetch] [workflow.ts:Flow] [database.ts:databaseNode]
- The optional bridge records metrics and spans through application-supplied or global OpenTelemetry providers. [opentelemetry.ts:OpenTelemetryBridge]

## Verification

Run from the repository root:

```sh
cd sdks/typescript && npm test
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
