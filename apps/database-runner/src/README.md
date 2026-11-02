# database-runner source

## Overview

Isolated managed-PostgreSQL executor with a narrow configuration/runner API and crate-private lease, capability, session and reporting machinery.

## Interface

- Public `RunnerError` and re-exported `Runner::{start, serve}` expose process startup and cancellation. [lib.rs:RunnerError] [lib.rs:Runner] [runner.rs:Runner]
- `config` exposes `Config`, `DeploymentPolicy`, `NamespacePolicy`, `Limits` and parsing/policy/network/deadline helpers. [config.rs:Config] [config.rs:DeploymentPolicy] [config.rs:NamespacePolicy] [config.rs:Limits]
- Internal attempt execution, capability decoding, authenticated RPC and per-connection session slots remain crate-private. The executable requires `SAGAT_DATABASE_RUNNER_CONFIG` and optionally configures metrics/OTLP. [attempt.rs:run] [capability.rs:decode] [rpc.rs:Rpc] [slots.rs:Slots] [main.rs:SAGAT_DATABASE_RUNNER_CONFIG]

## Invariants

- Configuration rejects unknown fields, bounds file/settings/namespace sizes, requires HTTPS except explicit numeric-loopback development mode, and requires deployment deny CIDRs outside that mode. [config.rs:Config] [config.rs:bounded_file]
- Each attempt rereads operator configuration, preserves startup development mode and intersects deployment/namespace capability limits. Decoding validates identity, revisions, credentials, TLS, operation/parameters and lease deadline; credential bytes are zeroized on exit. [attempt.rs:execute] [config.rs:Config::policy] [capability.rs:decode] [capability.rs:Guard]
- Each RPC rereads a bounded regular token file, validates its nonblank whitespace-free token and uses sensitive Bearer metadata. Sessions are bounded per namespace/connection by the capability pool size. [rpc.rs:Rpc] [slots.rs:Slots] [slots.rs:Permit]
- Only managed-database leases without instance paths are accepted. SQL execution is invoked once per attempt while terminal reports retry stable task/generation keys. Lost blob acknowledgements may allocate another blob; cleanup relies on TTL. [attempt.rs:run] [attempt.rs:execute] [attempt.rs:report_until_ack] [attempt.rs:upload]
- Registration advertises only the managed-database activity with protocol 1.6 and validates the returned version, worker identity and session token. [runner.rs:Runner::register]

## Side Effects

- Reads configuration/token/CA files, registers and polls worker sessions, resolves credentials, heartbeats, uploads blobs and reports terminal outcomes over gRPC. [config.rs:Config::read] [rpc.rs:Rpc] [runner.rs:Runner::serve] [attempt.rs:run]
- Executes the capability query/statement through `PostgresExecutor` against its approved destination scope. Emits runner telemetry and exposes an optional metrics-only HTTP listener. [attempt.rs:execute] [runner.rs:Runner::serve] [main.rs:run]

## Verification

From the repository root, with a private Cargo target directory outside the checkout:

```sh
cargo test -p sagat-database-runner --locked
```

This is the manifest-derived package check; it was not run during this documentation pass.
