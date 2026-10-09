# Runbook: sagas, compensations and reconciliation (feature 003)

For the people who run workflows that undo their own work. It covers what a saga does when something fails, what a person is asked to decide, and how to tell a provider that is down from a run that is stuck. Design: [`docs/architecture/sagas.md`](../architecture/sagas.md); decisions: [`specs/003-sagas-compensations/research.md`](../../specs/003-sagas-compensations/research.md); the engine's own runbook: [`execution-engine.md`](execution-engine.md).

## 1. What happens when a step fails

A `saga` node wraps a part of the workflow. If a step inside it fails for good (and nothing inside handles it), or the run is cancelled or passes its deadline, the saga stops going forward and **undoes what completed, last first**:

1. The run becomes `COMPENSATING`. Each completed step that has a `compensateWith` gets a compensation step `<id>~c`; a compensation waits for the ones of the steps that came after it, and the branches of a `parallel` node do not wait for each other.
2. Workers receive the compensation as an ordinary task (`task_kind: COMPENSATION`, the activity named in `compensateWith`, the forward step's idempotency key in `target_idempotency_key`). It is retried (default five attempts, 2 s to 5 min).
3. When every compensation is done the saga ends and so does the run, with an outcome:

| Run status | Meaning |
|---|---|
| `COMPENSATED` | Everything that had to be undone was. |
| `PARTIAL` | Something could not be: a step marked `irreversible`, a compensation an operator **skipped**, or an effect of unknown outcome that nothing can undo. The run must be handled by a person afterwards. |
| `COMPENSATION_FAILED` | A compensation failed for good and the saga is configured `onCompensationFailure: fail`, or the run was cancelled while a person was being waited for. |
| `CANCELLED` / `TIMED_OUT` | The run was stopped and what it had done was undone first (the status is final only when the undoing is). |

A cancellation does **not** cancel a compensation that is already running: undoing is not work to abandon. What a stop does to the rest depends on the saga's `cancelPolicy`: `compensate_completed` (default: stop what is in flight, undo what completed), `wait_in_flight_then_compensate` (let the attempts in flight finish, then undo everything), `abort_no_compensation` (just stop).

## 2. A step whose outcome is unknown

If an attempt ends without anyone knowing whether the effect happened — the worker died holding the lease (`LEASE_EXPIRED`), the attempt timed out (`ATTEMPT_TIMEOUT`), or the worker reported the reserved code `OUTCOME_UNKNOWN` — the platform does **not** retry blindly:

- With `reconcileWith`, the step waits (`waitingOn: reconciliation`) and a lookup `<id>~r` is handed to a worker with the forward step's idempotency key. The worker answers `{"effect": "applied" | "not_applied" | "unknown", "output": …}`. *Applied*: the step succeeded, with that output, and nothing is submitted again. *Not applied*: the step is retried (same key) or, with no attempt left, failed and the saga compensates. *Unknown*, an answer of another shape, or ten failed lookups: the step goes to a person.
- Without `reconcileWith`, an ordinary step is retried as always; a step marked `highRisk` waits for a person instead.

## 3. Runs that wait for a person

`GET /runs?status=MANUAL_REVIEW` lists them. The runtime graph (`GET /runs/{id}/graph`) shows which step waits (`waitingOn: operator`, `failureCode` says why) — compensations appear as `<id>~c`, lookups as `<id>~r`.

Decide with `POST /runs/{id}/operations` (an `Idempotency-Key` header, a `reason` of 8 to 2000 characters after trimming that goes to the audit trail whole, with the outcome declared by a `RESOLVE_STEP`, optionally `expectedRunVersion` to refuse a decision taken on a stale view):

| Action | Body | Who | Effect |
|---|---|---|---|
| `RETRY_COMPENSATION` / `RETRY_STEP` | `stepId` | Operator or Admin (`run:operate`); **Admin only** (`saga:override`) for a step whose outcome may have happened (`waitingOn: operator` with `LEASE_EXPIRED`, `ATTEMPT_TIMEOUT`, `OUTCOME_UNKNOWN`, `RECONCILIATION_FAILED` or `RECONCILIATION_UNKNOWN`) | Another attempt of the same step, with the same idempotency key. The old attempts stay in the journal. Refused (409) on a step of a saga that stopped going forward: decide whether it happened instead. |
| `SKIP_COMPENSATION` | `stepId` of a `…~c` | Admin only (`saga:override`) | Give up on that compensation. The ones behind it go on. The saga ends `PARTIAL`. |
| `RESOLVE_STEP` | `stepId`, `outcome: SUCCEEDED` or `FAILED`, `errorCode` for `FAILED` | Admin only (`saga:override`) | Declare that the step happened (it is complete, later steps go on) or did not (it failed and the saga compensates). Use it after checking with the provider. |

A decision on a step that does not wait for a person, or that someone else already decided, is `409` and changes nothing. Every decision is audited (`saga.step_retried`, `saga.step_skipped`, `saga.step_resolved`) with who, which step and why, and a run that waits for a person is not forgotten by a restart: it is a row.

Cancelling a run that waits for a person gives up on the waiting: a forward step is cancelled and compensated like what completed; a compensation that was waiting is cancelled, and the run ends `COMPENSATION_FAILED` because an effect was left in place.

## 4. The circuit breaker

When the compensations of one activity fail several times in a row (`SAGAT_COMPENSATION_BREAKER_THRESHOLD`, default 5, counted per tenant, namespace and activity name: a failing `dev` does not hold back `prod`) the platform stops handing out compensations of that activity for `SAGAT_COMPENSATION_BREAKER_OPEN_SECS` (30). They stay `READY`, lose nothing, and the runs stay `COMPENSATING`. Then exactly one is handed out as a probe: if it succeeds the breaker closes; if it fails it reopens for twice as long, up to `SAGAT_COMPENSATION_BREAKER_MAX_OPEN_SECS` (300). Failures of attempts that were already running when it opened are counted and change nothing else. `0` turns the breaker off.

```sql
-- which providers are being left alone, and until when
SELECT namespace_id, activity, failures, level, opened_until FROM compensation_breakers WHERE tenant_id = '…' AND opened_until > now();
```

A compensation that reports "already released" as a **failure** counts toward the breaker. Report the idempotent answer as a success.

## 5. Symptoms

| Symptom | Likely cause | What to do |
|---|---|---|
| Runs pile up in `COMPENSATING`, `compensation_breaker_opened_total` rises | The provider of the compensation is down; its breaker is open | Fix or wait for the provider; the probe closes the breaker by itself. `compensation_attempts_total{result}` shows what the attempts do |
| Runs in `MANUAL_REVIEW` | A compensation ran out of attempts, or a lookup could not say | Section 3. `operator_decisions_total{action="requested"}` counts how often a person is asked |
| A run ended `PARTIAL` | Something could not be undone (see the graph: the saga node's `irreversible`, a `SKIPPED` compensation) | Handle it outside the platform; the audit trail has the decisions |
| A step is `WAITING` on `reconciliation` for long | No worker serves the lookup activity (`reconcileWith`), or its bindings do not cover it | Bind and start a worker for it; the lookup is retried ten times and then goes to a person |
| `RECONCILIATION_ANSWER_INVALID` on a `…~r` | The lookup answered something that is not `{"effect": …}` | Fix the activity; decide the step by hand meanwhile |
| `403` on `SKIP_COMPENSATION`, `RESOLVE_STEP` or a retry of a step whose outcome may have happened | Only administrators hold `saga:override` | An administrator decides; operators may `RETRY_COMPENSATION` |
| A run ends `CHILD_COMPENSATED` / `CHILD_PARTIAL` / `CHILD_COMPENSATION_FAILED` | Its child run ended in a saga outcome | Route on the code with an error edge, or look at the child run (its id is `child_run_id(parent, node)`) |
| A cancelled run is not `CANCELLED` yet | A compensation is running or waits for a person, or (when the run ended because *another branch* failed) a saga waits for one | Section 3 |
| `409` on a decision | Someone decided first, or the step does not wait for a person | Read the run again |
| `422` on `RETRY_COMPENSATION` | `stepId` is not a compensation (it does not end in `~c`) | Use the id shown by the graph |

## 6. Metrics

| Metric | Meaning |
|---|---|
| `saga_transitions_total{event}` | `entered`, `completed`, `aborting`, `planned` |
| `saga_outcomes_total{outcome}` | `compensated`, `partial`, `compensation_failed` |
| `compensation_attempts_total{result}` | `succeeded`, `retry_scheduled`, `failed`, `needs_review` |
| `reconciliations_total{stage}` | `requested`, `applied`, `not_applied`, `unknown` |
| `operator_decisions_total{action}` | `requested`, `retry`, `skip`, `resolve_succeeded`, `resolve_failed` |
| `compensation_breaker_opened_total` | Times a breaker opened or reopened |

They are counted after the transaction commits, and no label names a tenant, run, node or payment. A dashboard and alerts on them belong to feature 008.

## 7. Upgrading

The migration of this feature adds constraints to `steps` and `runs` as `NOT VALID` (they apply to every new row and do not scan the table under its lock), and replaces the ready index of `steps` by two partial ones. On a database with a very large `steps` table apply it in a quiet period, or create the two indexes by hand first with `CREATE INDEX CONCURRENTLY` and the same definitions (`steps_ready_idx` for forward work and lookups, `steps_ready_compensation_idx` for compensations; see the migration). `ALTER TABLE … VALIDATE CONSTRAINT` can mark the new constraints validated at leisure; it does not block writes.

## 8. Not covered

Listed with the reason in [`specs/003-sagas-compensations/research.md`](../../specs/003-sagas-compensations/research.md) (G51–G61): the platform cannot check that a compensation is idempotent (it is the author's contract, and the key helps); the breaker is per activity name and does not cover lookups; Studio screens and dashboards are features 007 and 008; the SDK DSL is feature 006; compensations written as inline scripts wait for the sandbox (005).
