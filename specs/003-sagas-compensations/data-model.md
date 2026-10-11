# Data Model: Native sagas and compensations

## Context

Orchestrate long-running business transactions, with compensations, controlled reprocessing and reconciliation.

Data ownership follows the bounded contexts of `docs/architecture/overview.md`, with the reference physical schema in `specs/001-platform-foundation/data-model.md`. This feature adds one migration, `crates/infra-postgres/migrations/20261011000000_sagas.up.sql` (with its `.down.sql`); it only adds: a column, wider `CHECK` lists (added `NOT VALID`: they bind every new row and do not scan the table under its lock), four tables for sagas, one for the circuit breaker, and a split of the ready index (forward work, and compensations on an index of their own so that the ones an open breaker holds back are never walked by anything else).

## The definition (IR)

| Field | Where | Meaning |
|---|---|---|
| `type: saga`, `begin`, `onCompensationFailure` (`manual_review` default, `fail`), `cancelPolicy` (`compensate_completed` default, `wait_in_flight_then_compensate`, `abort_no_compensation`) | a node | A closed region: everything reachable from `begin` up to a node with no `next` is its body. At most 32 per workflow; they nest. |
| `compensateWith` | an `activity` | The activity that undoes it. Derives the node `<id>~c`. Must be idempotent and tolerate an effect that never happened. |
| `reconcileWith` | an `activity` | The activity that finds out whether it happened. Derives `<id>~r`. |
| `compensationRetry`, `compensationTimeoutMs` | an `activity` | Retry budget and attempt timeout of the compensation (default 5 attempts, exponential 2 s to 5 min, jitter). |
| `irreversible` | an `activity` | The effect cannot be undone; a saga that compensates past it ends `PARTIAL`. Cannot be combined with `compensateWith`. |
| `highRisk` | an `activity` | An attempt with an unknown outcome is never retried blindly: it is looked up (`reconcileWith`) or waits for an operator. |

Refused when the version is published: a body entered from outside or leaving the region (`SagaBoundary`), an `end` inside a body, more than 32 sagas, `irreversible` with `compensateWith`, effect fields on a node that is not an activity, an id containing `~`. Accepted with a warning: `compensateWith` outside any saga (it can never run).

**Derived nodes.** `<id>~c` and `<id>~r` are nodes of the parsed definition (`NodeOrigin::Compensation` / `Reconciliation`) that nothing routes to. They carry the activity, retry policy, timeout and idempotency template of their forward node (so secrets, bindings and templates apply to them like to it). `WorkflowDefinition::nodes()` is what the author wrote, `all_nodes()` what the engine can run.

## Entities and observable state

- **Definition/Version:** immutable after publication; tenant and namespace required. A version carries its **compensation plans**.
- **`saga_plans`** (tenant, version, saga node): `on_compensation_failure`, `cancel_policy`, `irreversible[]`. Immutable by trigger (also against `TRUNCATE`); a composite foreign key to `workflow_versions (tenant_id, id)` keeps a plan inside its tenant.
- **`saga_plan_entries`** (tenant, version, saga node, node): `position` (a valid order to follow one by one), `compensation_id` (stable for the version and node), `compensation_activity`, `after[]` (the forward nodes whose compensations must finish first). Immutable like the plans.
- **Step (`steps`):** gains `kind` (`FORWARD`, `COMPENSATION`, `RECONCILIATION`, checked against the node id by `steps_kind_matches_node`), the status `SKIPPED`, and the waiting reasons `SAGA` (a saga node while its body works, compensates or waits), `RECONCILIATION` (the lookup is finding out) and `OPERATOR` (a person has to decide; `failure_code` then says why, enforced by `steps_operator_has_a_reason`).
- **`run_sagas`** (tenant, run, node): `phase` (`FORWARD`, `ABORTING`, `COMPENSATING`, `DONE`), `cause_code`, `outcome` (`COMPENSATED`, `PARTIAL`, `COMPENSATION_FAILED`; only when `DONE`), `during_stop`, `irreversible[]`. A projection of the journal, written with it.
- **`run_saga_entries`** (tenant, run, saga node, node): the compensations a saga planned for this run, with the ones each waits for and their `compensation_id`. Written once, when the saga stops going forward.
- **Run (`runs`):** the statuses `COMPENSATING` and `MANUAL_REVIEW` (live) and `COMPENSATED`, `PARTIAL`, `COMPENSATION_FAILED` (terminal; `finished_at` is set for them), the stop reasons that name them, and a partial index for the runs that wait for a decision.
- **JournalEvent:** append-only, as before. New events: `saga.entered`, `saga.completed`, `saga.aborting`, `compensation.planned`, `saga.compensated`, `reconciliation.requested`, `reconciliation.concluded`, `step.needs_review`, `step.retry_requested`, `step.skipped`, `step.resolved`.
- **`compensation_breakers`** (tenant, namespace, activity): `failures`, `opened_until` (NULL closed; future open or probing; past half open), `probing` (the lease granted when the window was over is the probe), `level` (failed probes in a row, which doubles the window), `updated_at`. Written in the transaction that records an attempt's end; deleted by a success.
- **Audit:** `saga.step_retried`, `saga.step_skipped`, `saga.step_resolved`, with the actor, `run/node` as resource and the reason (at most 512 characters), in the transaction of the decision.

## State machines

**Saga (`run_sagas.phase`)**

```
FORWARD ──(unhandled failure in the body | the run is stopped | the branch lost its join)──▶ ABORTING
FORWARD ──(the body ran to its end)──▶ DONE (completed; the run goes on)
ABORTING ──(the body is wound down, reconciliations answered)──▶ COMPENSATING   [compensation.planned]
COMPENSATING ──(every planned compensation is terminal)──▶ DONE (COMPENSATED | PARTIAL | COMPENSATION_FAILED)
```

**Compensation step (`<id>~c`):** `PENDING` (waits for the compensations it depends on) → `READY` → `LEASED` → `RUNNING` → `SUCCEEDED`; a failure with attempts left → `RETRY_WAIT` → `READY`; no attempts left → `WAITING` on `OPERATOR` (policy `manual_review`) or `FAILED` (policy `fail`). From `WAITING` an operator takes it to `READY` (retry), `SKIPPED`, or `SUCCEEDED` / `FAILED` (resolve). A compensation whose dependency `FAILED` or was `CANCELLED` is `CANCELLED`: what waits for a compensation that will not happen cannot be released either.

**Forward step with an unknown outcome:** `LEASED/RUNNING` → (lease lost, timeout, `OUTCOME_UNKNOWN`) → `WAITING` on `RECONCILIATION` (its `<id>~r` is `READY`) → verdict *applied* `SUCCEEDED`; *not applied* `RETRY_WAIT` or `FAILED`; *unknown*, an invalid answer or a lookup out of attempts `WAITING` on `OPERATOR`. A `highRisk` step with no lookup goes to `WAITING` on `OPERATOR` directly.

**Run status** is a function of the steps, the sagas and the stop: `MANUAL_REVIEW` when a step waits on an operator, else `COMPENSATING` when a saga is `ABORTING` or `COMPENSATING`, else as before; at the end `final_status_for(reason)` gives `SUCCEEDED`, `FAILED`, `CANCELLED`, `TIMED_OUT`, or, if a saga ended while the run was being stopped, the worse of its outcome and the stop's.

## Invariants

- Every tenant-scoped record validates the security context; every new table carries `tenant_id` in its key and in its foreign keys, and every statement filters by it.
- No in-place update of a published version; its plans cannot be changed or removed by anyone, the table owner included.
- Every state change records a sequenced event, and the projections equal a replay of the journal (`verify_run` after every scenario).
- A compensation is planned once per saga, only for steps whose effect happened or may have happened, in an order that never starts one before those it waits for.
- A compensation and a lookup carry idempotency keys that are the same on every attempt and every restart (`compensation:{id}:{run}`, `reconciliation:{run}:{forward}`), and the key of the forward step they are about.
- A decision of an operator is applied once: the second of two racing decisions on the same step is refused and writes nothing, and the same `Idempotency-Key` replays the first.
- A run that waits for a person holds no lease, no connection and no timer.
- The breaker's probe is exactly one: of all the callers that find it half open one wins the update. A failure that arrives while it is open changes only the count.
- A saga's `onError` route runs only for a failure that was undone (`COMPENSATED`); otherwise the failure is unhandled and the run ends with the saga's outcome.
- A run's final status is the worst of the stop reason and of every saga that ended while the run was ending.
- Stopping a run supersedes its pause.

## Lock order

A command locks the run row (`SELECT … FOR UPDATE`) first. The circuit breaker row is read or written only while that lock is held and after nothing else is locked, and the claim query reads it without locking, so the breaker adds no edge to the lock order of feature 009 (binding, then quota).

## Migrations

Additive and reversible: `…_sagas.up.sql` widens the `CHECK` lists of `steps.status`, `steps.waiting_kind`, `runs.status`, `runs.stop_reason` and `runs.finished_consistent`, adds `steps.kind` (default `FORWARD`, so existing rows are valid) with its constraints, and creates the five tables; `…_sagas.down.sql` restores the previous lists and drops what was added. Runs in flight when it is applied have no derived nodes and no saga rows, and continue unchanged. The privileges of the runtime role are granted only when it exists, as before (`GRANT SELECT, INSERT, UPDATE` on `run_sagas`, `SELECT, INSERT` on the plan and entry tables, `SELECT, INSERT, UPDATE, DELETE` on `compensation_breakers`).
