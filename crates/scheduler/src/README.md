# crates/scheduler/src

## Overview

Background orchestration over authoritative storage ports: fair candidate selection, fenced transitions, event/outbound delivery, outbox publication, shard placement, monitoring, child coordination and journal-backed telemetry. Scheduler::tick refreshes placement before each ordered pass; it does not add a capacity-transfer runtime loop.

## Interface

- `DeliverySettings`, `DeliveryRound` and `DeliveryWorker` configure/tick/clean outbound delivery using imported sender/publisher/store ports. [deliveries.rs:DeliverySettings] [deliveries.rs:DeliveryRound] [deliveries.rs:DeliveryWorker]
- `EventDispatchSettings`, `EventRound` and `EventDispatcher` configure clock/cron-driven event dispatch and housekeeping. [events.rs:EventDispatchSettings] [events.rs:EventRound] [events.rs:EventDispatcher]
- Fairness exposes `Weights`, `Selection`, `share`, `select`, `rest`, `rest_after`, `ROUNDS` and `DEFAULT_WEIGHT`. [fairness.rs:Weights] [fairness.rs:Selection] [fairness.rs:share] [fairness.rs:select] [fairness.rs:rest] [fairness.rs:rest_after] [fairness.rs:ROUNDS] [fairness.rs:DEFAULT_WEIGHT]
- `MonitorSettings`/`MonitorRound`/`Monitor` provide operational sampling; `DispatchSettings`/`Dispatched`/`OutboxDispatcher` provide placed outbox publication. [monitor.rs:MonitorSettings] [monitor.rs:MonitorRound] [monitor.rs:Monitor] [outbox.rs:DispatchSettings] [outbox.rs:Dispatched] [outbox.rs:OutboxDispatcher]
- `SchedulerSettings`, `Report` and `Scheduler` configure the engine/store, item-source, placement and child-service dependencies, then expose tick/housekeeping. [passes.rs:SchedulerSettings] [passes.rs:Report] [passes.rs:Scheduler] [passes.rs:Scheduler::tick] [passes.rs:Scheduler::housekeeping]
- Public primitive advances cover collection/loop full-run entry points and continuation handoff, with typed CollectionAdvance/LoopAdvance and error results. [passes.rs:Scheduler::advance_collection] [passes.rs:Scheduler::advance_collection_from_run] [passes.rs:CollectionAdvanceError] [passes.rs:Scheduler::advance_loop] [passes.rs:Scheduler::advance_loop_from_run] [passes.rs:LoopAdvanceError] [passes.rs:Scheduler::advance_continuation] [passes.rs:ContinuationAdvanceError]
- Shard placement exposes `KeeperSettings`, `Snapshot`, `Placement`, `Round` and `Keeper` with round/leave and current shard/lease/weight views. [shards.rs:KeeperSettings] [shards.rs:Snapshot] [shards.rs:Placement] [shards.rs:Round] [shards.rs:Keeper]
- Journal telemetry exposes `ProjectorSettings`, `ProjectionRound`, `TelemetryProjector` run/status operations and projection `state`. [telemetry.rs:ProjectorSettings] [telemetry.rs:ProjectionRound] [telemetry.rs:TelemetryProjector] [telemetry.rs:state]
- [passes](./passes/README.md) — Private collection-transition passes for deterministic controls, scoped event waits and timers. They extend Scheduler through parent-module boundaries. [passes/collection_controls.rs:Scheduler::advance_collection_control]

## Invariants

- Placed scheduling selects unexpired owned shards and rechecks exact lease generation inside mutating transactions. Expired snapshots self-fence when storage cannot be reached. [passes.rs:View::holds] [passes.rs:View::holds_path] [shards.rs:Snapshot::lease_of] [shards.rs:Snapshot::shards]
- Root actions lock the run, check authority and stale-candidate conditions before commands/commit. Quota recovery opens a new transaction guarded by the original version/fence/condition. Scoped actions use exact-path revision checks, authority and due clocks; conflicts become skips. [passes.rs:Scheduler::apply_if] [passes.rs:Scheduler::stop_for_storage_quota] [passes.rs:Scheduler::apply_path_if]
- Weighted max-min selection is bounded by ROUNDS/MAX_EXCLUDED and preserves encounter order; later discovery failure retains already discovered work. [fairness.rs:select] [fairness.rs:share] [fairness.rs:ROUNDS] [fairness.rs:MAX_EXCLUDED]
- Trigger delivery is recorded in the new-run transaction; wait delivery/output/state/audit and commit share the run transaction, while scoped wake uses native receipts and addressed persistence. [events.rs:EventDispatcher::start] [events.rs:EventDispatcher::wake] [events.rs:EventDispatcher::wake_scoped]
- Outbox delivery is at-least-once: publication precedes marking delivered, failures get durable backoff and lost confirmation can republish the same message ID. [outbox.rs:OutboxDispatcher::run_once] [outbox.rs:OutboxDispatcher::backoff]
- Continuation evaluates outside the initial head lock, then commits against captured chain/head/run revision and scheduler authority. Collection/loop advances require a real placement lease; child creation retains the parent shard fence in its creation transaction. [passes.rs:Scheduler::advance_continuation_in_view] [passes.rs:Scheduler::advance_collection_from_run_in_view] [passes.rs:Scheduler::advance_loop_from_run_in_view] [passes.rs:Scheduler::start_child] [passes.rs:View::holds_child_start]
- Journal projection advances after collector acceptance or permanent-rejection handling, and records derived observations only after mark_projected wins. Unavailable collectors leave work pending. [telemetry.rs:TelemetryProjector::flush] [telemetry.rs:record]
- The tick sequence calls collections, loops, continuations, approvals, timers, waits, retries, lost leases, deadlines, stopped-run settlement, hint renewal and child passes. Dormant capacity vocabulary adds no creator/claim permission or general nesting/workload acceptance. [passes.rs:Scheduler::tick]

## Side Effects

- Delivery claims work, resolves signing secrets/broker credentials, sends through HTTP/broker ports and records attempts/audits/metrics; failed attempt persistence leaves its lease to expire. [deliveries.rs:DeliveryWorker::tick] [deliveries.rs:DeliveryWorker::attempt_traced] [deliveries.rs:DeliveryWorker::settle]
- Event dispatch evaluates cron/filters, creates runs, wakes waits and updates inbox disposition; outbox dispatch publishes tenant-scoped messages and stores success/backoff. [events.rs:EventDispatcher::tick] [events.rs:EventDispatcher::cron_pass] [outbox.rs:OutboxDispatcher::run_once] [outbox.rs:subject_of]
- Tick mutates independent candidates through transaction ports and emits action/fence/quota metrics. Cleanup removes expired idempotency/session/script/inbox/delivered-outbox records in bounded work. [passes.rs:Scheduler::tick] [passes.rs:Scheduler::each] [passes.rs:Scheduler::housekeeping] [events.rs:EventDispatcher::housekeeping]
- Monitoring acquires a singleton lease, samples queues, reconciles/purges alerts and emits/zeros gauges. Keeper rounds announce replicas and renew/adopt/release/acquire leases; leave first empties local placement, then attempts durable cleanup. [monitor.rs:Monitor::run_once] [shards.rs:Keeper::round] [shards.rs:Placement::publish] [shards.rs:Keeper::leave]
- Telemetry projection reads journal batches, performs OTLP exports and advances projection records/metrics. Collection entry may read blob material through CollectionItemSource; transient unavailability waits, while permanent source failures request durable stops/audits. [telemetry.rs:TelemetryProjector::run_once] [telemetry.rs:TelemetryProjector::flush] [passes.rs:Scheduler::with_item_source] [passes.rs:Scheduler::stop_collection_for_source]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-scheduler --locked
```

These commands were not run in this documentation-only pass.
