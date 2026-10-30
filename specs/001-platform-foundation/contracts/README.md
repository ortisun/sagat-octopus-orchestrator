# Canonical design contracts

- `openapi.yaml`: REST endpoints of the control plane and runs.
- `worker.proto`: gRPC for worker registration, leases, heartbeat, complete/fail.
- `events.asyncapi.yaml`: NATS event envelopes and integrations.
- `workflow.schema.json`: declarative IR validated by JSON Schema 2020-12.
- `schema.sql`: **illustrative SQL model** to guide reviewed migrations; do not use as a production migration without reviewing composite foreign keys/tenant isolation, indexes, partitioning and policies.

## Rules

1. These contracts are design proposals; they do not present themselves as working endpoints.
2. If the contracts change, update all SDKs, conformance tests and `docs/traceability.md`.
3. `workflow.schema.json` validates the basic format; extra semantic validation checks node uniqueness, references, cycles, branches, Saga scopes and fanout limit.
4. `worker.proto` requires mTLS authentication/worker session scoped via interceptor in addition to payload fields.
5. `openapi.yaml` does not allow an arbitrarily supplied tenant_id in the request; authorization resolves the tenant.

<!-- EGM directory contract -->

## Interface

- `WorkerService` declares `RegisterWorker`, `PollTasks`, `StreamTasks`, `Heartbeat`, `CompleteStep`, `FailStep`, `ResolveSecrets`, `ResolveDatabase`, `WriteBlob` and `ReadBlob`. [worker.proto:WorkerService]
- Worker messages cover capabilities, leases, execution identity, heartbeat/results, secret resolution, blobs and typed managed-database values/configuration. [worker.proto:WorkerCapabilities] [worker.proto:TaskLease] [worker.proto:ExecutionInstanceIdentity] [worker.proto:DatabaseValue] [worker.proto:ResolvedDatabase]
- Illustrative identity/audit tables: `tenants`, `workspaces`, `namespaces`, `memberships`, `audit_records`. [schema.sql:tenants] [schema.sql:workspaces] [schema.sql:namespaces] [schema.sql:memberships] [schema.sql:audit_records]
- Illustrative workflow/execution tables: `workflows`, `workflow_versions`, `runs`, `run_events`, `steps`, `step_attempts`, `timers`, `wait_subscriptions`. [schema.sql:workflows] [schema.sql:workflow_versions] [schema.sql:runs] [schema.sql:run_events] [schema.sql:steps] [schema.sql:step_attempts] [schema.sql:timers] [schema.sql:wait_subscriptions]
- Illustrative integration/saga/dedupe/script tables: `event_inbox`, `event_outbox`, `saga_entries`, `command_dedupe`, `script_versions`. [schema.sql:event_inbox] [schema.sql:event_outbox] [schema.sql:saga_entries] [schema.sql:command_dedupe] [schema.sql:script_versions]

## Invariants

- The SQL explicitly says `MODEL DESIGN ONLY`; authoritative migrations supply production schema and triggers. Schema declarations do not prove implementation of an endpoint or guarantee. [schema.sql:MODEL DESIGN ONLY]
- The worker contract specifies at-least-once delivery and stable idempotency keys for lost responses, current lease-bound secret/database access, and bounded header/chunk blob streams. These are protocol requirements; implementation evidence belongs in the contract/conformance suites. [worker.proto:WorkerService] [worker.proto:WriteBlobRequest] [worker.proto:ReadBlobResponse]
- The illustrative namespace foreign key includes tenant/workspace identity, with namespace slugs unique per tenant. The audit comment delegates append-only enforcement to migration triggers. [schema.sql:namespaces] [schema.sql:audit_records]

## Side Effects

- Protocol/schema declarations have no runtime effects by themselves. Executing the illustrative SQL would create an extension, tables, constraints and indexes; do not use it as a production migration. [schema.sql:CREATE EXTENSION] [schema.sql:CREATE TABLE] [schema.sql:CREATE INDEX]

## Verification

Run from the repository root:

```sh
cargo test -p sagat-tests --test contract_platform_foundation --locked
cargo test -p sagat-tests --test contract_grpc_durable_execution --locked
```

Commands are verification entry points, not results from this documentation pass. Use the repository resource profile in [AGENTS.md](../../../AGENTS.md) for Cargo builds; integration/drill commands need their declared services and tools.
