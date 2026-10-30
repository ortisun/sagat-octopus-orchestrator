# execution-engine source

## Overview

Worker gRPC serving and durable scheduling/event/delivery/projection loops assembled over PostgreSQL with optional hint transports and connectors.

## Interface

- `serve` accepts shared/engine configuration, pre-bound worker/admin listeners and shutdown; `probe` and `shutdown_signal` are re-exported process helpers. [lib.rs:serve] [lib.rs:probe] [lib.rs:shutdown_signal]
- `EngineConfig`, `TelemetrySettings` and `MonitorConfig` parse scheduler, shard, transport, secret, blob, listener and OTel settings. [settings.rs:EngineConfig] [settings.rs:TelemetrySettings] [settings.rs:MonitorConfig]
- `loops` exposes concrete engine aliases, `HintPublisher`, `Health` and scheduler/keeper/housekeeping/dispatcher/event/delivery/telemetry/monitor loop functions. CLI helpers define serve/healthcheck/help/version. [loops.rs:EngineScheduler] [loops.rs:HintPublisher] [loops.rs:Health] [loops.rs:run_scheduler] [loops.rs:run_keeper] [loops.rs:run_housekeeping] [loops.rs:run_dispatcher] [loops.rs:run_events] [loops.rs:run_deliveries] [loops.rs:run_telemetry] [loops.rs:run_monitor] [cli.rs:Command]
- Crate-private `CriticalTasks` / `supervise` coordinate critical-task shutdown and bounded draining. [supervision.rs:CriticalTasks] [supervision.rs:supervise]

## Invariants

- Outside development, startup rejects insecure JWKS settings and verifies least-privilege audit protection. Configuration selects one internal event transport and requires its URL; plaintext secret-bearing worker transport needs its explicit allowance. [lib.rs:serve] [settings.rs:EngineConfig]
- Scheduler concurrency reserves pool capacity for the keeper; shard leases and scheduler batch/concurrency settings are bounded. [lib.rs:serve] [settings.rs:MIN_SHARD_LEASE] [settings.rs:MAX_SHARD_LEASE] [settings.rs:MAX_BATCH]
- Critical-task failures make health sticky-unhealthy and trigger shared shutdown. Readiness also checks fresh scheduler/keeper progress and bounded database readiness; draining has a finite grace period. [loops.rs:Health] [supervision.rs:supervise] [supervision.rs:CriticalTasks] [lib.rs:SHUTDOWN_GRACE]
- Transport dispatch, connector supervision and blob cleanup are conditional on their configured backends; their presence is not implied by starting the engine. [lib.rs:serve]

## Side Effects

- Uses PostgreSQL, holds/releases shard leases and starts scheduler, event, delivery, housekeeping, monitoring and journal-projection tasks. [lib.rs:serve] [loops.rs:run_keeper]
- Serves the worker gRPC endpoint and optional admin metrics/liveness/readiness endpoints; configured integrations may connect to NATS/Redis, OpenBao, blob storage and external connector destinations. [lib.rs:serve]
- Reads configured TLS files, runs metrics upkeep and optional OTLP export, and drains/aborts tasks on shutdown before closing storage. [settings.rs:pem_file] [lib.rs:serve] [supervision.rs:supervise]

## Verification

From the repository root, with a private Cargo target directory outside the checkout:

```sh
cargo test -p sagat-execution-engine --locked
```

This is the manifest-derived package check; it was not run during this documentation pass.
