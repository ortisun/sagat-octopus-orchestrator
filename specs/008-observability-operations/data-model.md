# Data Model: Distributed observability and operations

## Context

Diagnose and operate runs, steps, external integration, resources and Sagas from reliable data.

Data ownership follows the bounded contexts of `docs/architecture/overview.md`, with the reference physical schema in `specs/001-platform-foundation/data-model.md` and `specs/002-durable-execution/data-model.md`. The physical schema of this feature is the four additive migrations `20261015000000_trace_links`, `20261015000100_telemetry_projection`, `20261015000200_operational_alerts` and `20261015000300_operator_actions` in `crates/infra-postgres/migrations/`, each with a tested `down`.

The central decision (ADR-0017): **telemetry is derived from the journal and never produced on the write path.** The journal (`run_events`) is the truth about what happened; the spans and measurements are a projection of it, and the only thing the write path adds is a note of how far the journal has got.

## Entities and observable state

| Entity | Table | Notes |
|---|---|---|
| Projection note | `telemetry_pending` | One row per run: `head_seq` (the last journal sequence, as the write path left it), `applied_seq` (the last one the projector has taken), `behind_since` (null while caught up). Composite foreign key to `runs`, deleted with the run. Written by an upsert in the transaction that appends to the journal; read and advanced by the projector. The age of the oldest row with `applied_seq < head_seq` is the lag, exactly. Check constraints: `head_seq >= applied_seq >= 0` and `behind_since IS NULL` exactly when caught up. Partial index on `behind_since` where behind: only those rows are ever asked for. |
| Engine lease | `engine_leases` | `(name, holder, expires_at)`: a singleton loop (the projector `telemetry-projector`, the monitor) runs in one process at a time. The expiry is the **database's clock**, so processes with skewed clocks agree, and a process that dies is replaced when its lease runs out, with no session-level lock to leak through a pool. |
| Trace links | `runs.trace_parent_span`, `inbox_events.trace_id`, `inbox_events.trace_parent_span` | The span a run hangs from (the request that started it, or for a run started by an event, the span of the event's request) and the trace an event was published in. Format-checked (`^[0-9a-f]{16}$`, `^[0-9a-f]{32}$`), nullable: a run from before the feature hangs from nothing (G176). |
| Operational alert | `operational_alerts` | One row per `(tenant, namespace, kind, subject)` that toggles between `FIRING` and `RESOLVED`: counts and ages and nothing else, never a payload, a run's data or a secret. `kind` is one of `OUTBOX_STUCK`, `LEASE_NOT_REAPED`, `STEP_UNCLAIMED`, `REVIEW_PENDING`, `DEAD_ITEMS`; `subject` is bounded (120 chars: an activity name or the kind of dead item). A state change is written in the transaction that records the obligation to tell the namespace's webhook subscribers. Composite foreign key to `namespaces`. |
| Alert subscriptions | `webhook_subscriptions.kinds` | The constraint now allows `ops.alert.fired` and `ops.alert.resolved` besides `run.finished`. |
| Dead and live indexes | `inbox_events_dead_idx`, `runs_live_by_status_idx` | Partial indexes for what the monitor asks about, so asking is a range scan over a small index and never a scan of a table that grows with history. |
| Re-run link | `runs.rerun_of` | The run an ended run was started again from (`RERUN_AS_NEW`); composite foreign key to `runs`; the new run is otherwise ordinary (its journal, its trace, its quota). |
| Worker drain | `worker_drains` | `(tenant, namespace, subject)`: a worker **identity** whose sessions in the namespace are given no new work. `reason` (8 to 2000 chars), `drained_by`, `drained_at`. The mark is on the identity and not on a session, so a worker that registers again (a restart) is still drained until someone says otherwise. Composite foreign key to `namespaces`. |
| Audit | `audit_records` (existing) | New actions: `run.marked_for_review`, `run.rerun`, `audit.read`, `worker.drained`, `worker.undrained`; new resource type `worker`. `audit.read` collapses repeats. |
| Journal | `run_events` (existing) | **Not changed.** The cause shown next to a transition (`failure`, `retry`, `review`, `abort`, `unknown_outcome`, `reconciliation`, `stop`, `timeout`, `cancel`, `decision`, `request`, `outcome`) is computed from the event by `RunEvent::cause()`, and so is the span id of an event (`event_span_id(run, seq)`): both are derived, never stored. |

Two things are derived and not stored, on purpose: **span ids** are computed from the run and the step (`run_span_id`, `attempt_span_id`, `event_span_id`), so exporting the same journal twice produces the same spans and a backend deduplicates them; and the worker's lease carries the attempt's `traceparent`, computed the same way.

## Invariants

- Every row of every table above carries `tenant_id`, and every foreign key into `namespaces` or `runs` is the composite `(tenant_id, id)`: the database refuses a note, alert, drain or re-run link that points into another tenant.
- A note is written only in the transaction that appends to the journal, and `head_seq` never moves backwards (`GREATEST` in the upsert), so a note cannot be ahead of the journal at the time it is read. If one is anyway (a restore, a journal trimmed by retention), the projector settles it to what it claimed, with a warning, instead of asking for it forever.
- The projector records a run's progress **only after** the collector has taken its spans (or refused them for good), so a crash re-exports and never loses; the spans have computed ids, so a re-export is idempotent for the backend. Measurements are counted after the progress is recorded, so a crash between the two loses a count and never doubles one (G171).
- Nothing the projector or the monitor does takes a lock on a run. The projector reads an append-only journal at one point in time; the monitor reads aggregates.
- A condition the monitor could not look into this round leaves its alerts as they are: not seeing is not the same as being fine. At most 25 alerts of a kind per namespace per look.
- An operator action is a command: the domain checks the transition, the journal records it, the audit trail records who and why, in one transaction. The scheduler's own retry and an operator's race on the run's row; the first commit wins and the second gets an illegal-transition (`409`).
- A drain takes effect in the same statement that decides whether a poll is handed work (`draining` is computed in the session update), so a poll and a drain at the same moment never hand out work after the drain returned.
- The audit read returns the tenant's own records, scrubbed of anything with the shape of a credential on the way out, **without** `access.denied.cross_tenant` (it exists only when the identifier belongs to another tenant, so listing it would be an existence oracle), and records that it was read.
- Metric labels are closed sets: never a tenant, a namespace, a run, a node, an activity or a subject (checked in the sources by `no_metric_is_labelled_with_something_that_grows_with_the_data` and in the exposition of the engine by `the_gauges_follow_the_queues_and_never_carry_a_tenant_a_namespace_or_an_activity`).

## Migrations

Additive, with a tested `down` (`tests/integration/observability-operations/migration.rs`: rolled back to the migration before this feature's with a run, a note, a firing alert, a drain and subscriptions that ask for alerts in the tables, then forward again; the down removes what older code cannot understand: subscriptions that asked for nothing but alerts, and the notifications of alerts that were not delivered). A run created before the migration has no note and no parent span: its telemetry state is `UNTRACKED` until it changes. The runtime role `sagat_app` receives `SELECT, INSERT, UPDATE, DELETE` on `telemetry_pending`, `engine_leases`, `operational_alerts` and `worker_drains`, and nothing else. No migration rewrites a table: the new columns are nullable and the new indexes are small partial ones.
