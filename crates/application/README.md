# crates/application

## Overview

sagat-application defines use cases and storage/provider ports over domain types for adapters to implement.

## Interface

- RunService, WorkerService, Engine and governance/provider service families expose application workflows; their associated ports define durable authority and external-effect boundaries. [src/runs.rs:RunService] [src/worker.rs:WorkerService] [src/execution.rs:Engine] [src/quotas.rs:QuotaService] [src/secrets.rs:SecretService] [src/primitives.rs:CollectionStore]
- [src](./src/README.md) — Application use cases, typed commands/results and storage/provider ports. Services enforce local authorization and validation and coordinate effects; durable atomicity, current-authority rechecks, receipt/journal/outbox semantics and reconciliation are adapter responsibilities where specified by ports. [src/api_admission.rs:ApiOperation]

## Invariants

- Worker reports recheck receipts, lease fences and current authority around provider work. The private collection planner and ChildRoot/leaf certification/scoped-finish APIs remain caller-dormant; local checks do not establish general nesting/workload acceptance. [src/worker/output.rs:WorkerService::complete_inner] [src/worker/secrets.rs:WorkerService::resolve_secrets] [src/execution/collection_plan.rs:plan_collection_entry] [src/capacity_acquire.rs:CurrentChildRootAcquireStore] [src/leaf_certification.rs:LeafCollectionSuccessStore] [src/scoped_leaf_finish.rs:ScopedLeafFinishStore]

## Side Effects

- Use cases call storage, provider, validator and audit ports, generate credentials and emit spans/metrics. Concrete database/network I/O belongs to configured adapters; Engine persistence remains in its caller's transaction. [src/worker.rs:WorkerService] [src/worker/database.rs:WorkerService::resolve_database_inner] [src/runs/ad_hoc.rs:RunService::start_private] [src/execution.rs:Engine::run]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-application --locked
```

These commands were not run in this documentation-only pass.
