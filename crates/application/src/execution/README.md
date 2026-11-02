# crates/application/src/execution

## Overview

Dormant pure planner for bounded collection entry, return checkpointing and capacity-limited admission. The private module is not invoked by Engine.

## Interface

- Parent-private `CollectionSnapshot`, `AdmissionWindow`, `CollectionContinuation`, `CollectionPlan`, `CollectionReturn` and `CollectionPlanError` describe bounded snapshots, ordered events, continuation and typed failure. [collection_plan.rs:CollectionSnapshot] [collection_plan.rs:AdmissionWindow] [collection_plan.rs:CollectionContinuation] [collection_plan.rs:CollectionPlan] [collection_plan.rs:CollectionReturn] [collection_plan.rs:CollectionPlanError]
- Parent-private `plan_collection_entry` and `plan_collection_return` produce ordered addressed-event plans. [collection_plan.rs:plan_collection_entry] [collection_plan.rs:plan_collection_return]

## Invariants

- Admission windows are 1..=256; entry rejects excess max-items, and open planning matches item count, snapshot digest and every prior item digest. [collection_plan.rs:AdmissionWindow::new] [collection_plan.rs:plan_collection_entry] [collection_plan.rs:plan_open_collection]
- Admission is bounded by free concurrency slots, remaining items and the requested window, using contiguous indexes and exact child paths. Returns must name declared Return nodes and are checkpointed before replacement admission. [collection_plan.rs:plan_open_collection] [collection_plan.rs:plan_collection_return]
- Every proposed sequence is applied to cloned state before return. The private dead-code-allowed module remains dormant; its planner is not evidence of runtime enablement. [collection_plan.rs:validate_events] [collection_plan.rs:allow(dead_code)] [../execution.rs:mod collection_plan]

## Side Effects

- Planning clones state and applies events to shadow in-memory projections; it performs no storage or external I/O. [collection_plan.rs:plan_collection_entry] [collection_plan.rs:validate_events]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-application --locked
```

These commands were not run in this documentation-only pass.
