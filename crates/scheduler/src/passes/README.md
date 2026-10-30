# crates/scheduler/src/passes

## Overview

Private collection-transition passes for deterministic controls, scoped event waits and timers. They extend Scheduler through parent-module boundaries.

## Interface

- Control decisions use the parent-private `Scheduler::advance_collection_control` and `advance_collection_control_from_run` methods. [collection_controls.rs:Scheduler::advance_collection_control] [collection_controls.rs:Scheduler::advance_collection_control_from_run]
- Scoped event waits expose parent-private eligibility helpers `local_progress_candidate` and `deferred_failure`, plus `advance_collection_event_wait` and `advance_collection_event_wait_from_run`. [collection_event_waits.rs:local_progress_candidate] [collection_event_waits.rs:deferred_failure] [collection_event_waits.rs:Scheduler::advance_collection_event_wait] [collection_event_waits.rs:Scheduler::advance_collection_event_wait_from_run]
- Timer and deferred routing use parent-private `local_progress_candidate`, `advance_collection_local` and `advance_collection_local_from_run`. [collection_timers.rs:local_progress_candidate] [collection_timers.rs:Scheduler::advance_collection_local] [collection_timers.rs:Scheduler::advance_collection_local_from_run]

## Invariants

- Path advancement checks the expected run version, forward-progress permission, local step status, path revision and scheduler authority; revision conflicts return `Waiting`, while a missing fence is an error. [collection_controls.rs:Scheduler::advance_collection_control] [collection_event_waits.rs:Scheduler::advance_collection_event_wait] [collection_timers.rs:Scheduler::advance_collection_local]
- Conditions evaluate once; switches select the first true case or their default, and evaluator failure remains an error-valued resolution. [collection_controls.rs:Scheduler::control_answer]
- Scoped event candidates must be off the root path and be a valid pending wait or an unrouted terminal wait. Timer/routing candidates are pending timers or unrouted successful activity, condition, switch or timer instances. [collection_event_waits.rs:local_progress_candidate] [collection_timers.rs:local_progress_candidate]

## Side Effects

- Advances call store ports to load and begin transactions, check shard holds, persist addressed events and commit; domain calls mutate the transaction snapshot in memory. [collection_controls.rs:Scheduler::advance_collection_control] [collection_event_waits.rs:Scheduler::advance_collection_event_wait] [collection_timers.rs:Scheduler::advance_collection_local]
- Quota failures drop the current transaction before invoking collection-drain workflows. [collection_controls.rs:Scheduler::advance_collection_control] [collection_event_waits.rs:Scheduler::drain_event_wait_for_storage_quota]
- Advance methods emit `scheduler.action` spans with tenant, run, trace, instance and expected-version fields. [collection_controls.rs:tracing::instrument] [collection_event_waits.rs:tracing::instrument] [collection_timers.rs:tracing::instrument]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-scheduler --locked
```

These commands were not run in this documentation-only pass.
