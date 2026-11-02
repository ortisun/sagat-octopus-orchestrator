# crates/scheduler

## Overview

sagat-scheduler runs background passes over authoritative application storage ports.

## Interface

- Scheduler::tick, primitive advances, event/outbound delivery, outbox dispatch, shard placement, monitoring and telemetry projection compose background work. Private passes advance collection controls/waits/timers. [src/passes.rs:Scheduler::tick] [src/events.rs:EventDispatcher] [src/outbox.rs:OutboxDispatcher] [src/shards.rs:Keeper] [src/telemetry.rs:TelemetryProjector] [src/passes/collection_controls.rs:Scheduler::advance_collection_control] [src/passes/collection_event_waits.rs:Scheduler::advance_collection_event_wait] [src/passes/collection_timers.rs:Scheduler::advance_collection_local]
- [src](./src/README.md) — Background orchestration over authoritative storage ports: fair candidate selection, fenced transitions, event/outbound delivery, outbox publication, shard placement, monitoring, child coordination and journal-backed telemetry. Scheduler::tick refreshes placement before each ordered pass; it does not add a capacity-transfer runtime loop. [src/deliveries.rs:DeliverySettings]

## Invariants

- Placed mutations recheck exact shard generation and storage authority; discovery candidates are hints. tick includes active collection/loop/continuation/wait/timer/retry/child passes, with no capacity-transfer runtime loop; dormant vocabulary does not establish general nesting/workload acceptance. [src/passes.rs:View::holds] [src/passes.rs:Scheduler::apply_if] [src/passes.rs:Scheduler::tick]

## Side Effects

- Passes mutate authority through transaction ports, publish outbox/events, send external effects and emit metrics. [src/passes.rs:Scheduler::tick] [src/outbox.rs:OutboxDispatcher::run_once] [src/deliveries.rs:DeliveryWorker::tick]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-scheduler --locked
```

These commands were not run in this documentation-only pass.
