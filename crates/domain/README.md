# crates/domain

## Overview

sagat-domain contains framework-free business types, validation and reducers without web/database/broker adapters.

## Interface

- Public modules cover identity, audit, workflow, execution, policies, integrations, scripts, secrets, quotas and sagas. Execution exports journal vocabulary, reducers, commands, planners and exact-instance identities; collection shape proof is deliberately narrow. [src/lib.rs:pub mod] [src/execution/event.rs:RunEvent] [src/execution/instance.rs:ExecutionInstanceId] [src/workflow/collection_shape.rs:LeafCollectionSuccessShape]
- [src](./src/README.md) — Framework-free domain boundary for identity, audit, workflow/execution models, policies, integration vocabulary and planners/validators. Storage, transport, process and serialization adapters live elsewhere; local identifier generation is explicitly nondeterministic. [src/lib.rs:pub mod]

## Invariants

- Dependencies contain no internal Sagat adapter. Dormant capacity-transfer gates grant no execution permission; shape/path/limit checks do not enable general nesting or certify large workloads. [Cargo.toml:dependencies] [src/execution/capacity_transfer.rs:acquire_gate] [src/execution/capacity_transfer.rs:vacate_gate] [src/workflow/collection_shape.rs:LeafCollectionSuccessShape::check] [src/execution/plan.rs:start_pending]

## Side Effects

- Most operations validate, plan or mutate memory. UUID/audit constructors use local nondeterministic generation and secret values zeroize on drop. [src/macros.rs:uuid_id] [src/audit.rs:AuditRecord::by] [src/secrets.rs:SecretValue]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-domain --locked
```

These commands were not run in this documentation-only pass.
