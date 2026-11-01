# Runbook: sagas, compensations and reconciliation

For the people who run workflows that undo their own work. It covers what a saga does when something fails, what a person is asked to decide, and how to tell a provider that is down from a run that is stuck. The feature-037 delivery-1 behavior below is implementation in verification; the linked validation report, not this runbook, records acceptance. Design: [`docs/architecture/sagas.md`](../architecture/sagas.md); historical decisions: [`specs/003-sagas-compensations/research.md`](../../specs/003-sagas-compensations/research.md); the engine's own runbook: [`execution-engine.md`](execution-engine.md).

## 1. What happens when a step fails

A `saga` node wraps a part of the workflow. If a step inside it fails for good (and nothing inside handles it), or the run is cancelled or passes its deadline, the saga stops going forward and **undoes what completed, last first**:

1. The run becomes `COMPENSATING`. Each completed step that has a `compensateWith` gets a compensation step `<id>~c`; a compensation waits for the ones of the steps that came after it, and the branches of a `parallel` node do not wait for each other.
2. Workers receive the compensation as an ordinary task (`task_kind: COMPENSATION`, the activity named in `compensateWith`, the forward step's idempotency key in `target_idempotency_key`). It is retried (default five attempts, 2 s to 5 min).
3. When every compensation is done the saga ends and so does the run, with an outcome:

| Run status | Meaning |
|---|---|
| `COMPENSATED` | Everything that had to be undone was. |
| `PARTIAL` | Something known could not be undone: a step marked `irreversible` or a compensation an operator **skipped**. An unresolved unknown stays in review instead of being converted to `PARTIAL`. |
| `COMPENSATION_FAILED` | A known permanent/non-retryable compensation failed under explicit `onCompensationFailure: fail`; eligible independent compensations settled and blocked dependants were suppressed first. This is terminal retained evidence, not an active review queue. |
| `CANCELLED` / `TIMED_OUT` | The run was stopped and what it had done was undone first (the status is final only when the undoing is). |

A cancellation does **not** cancel a compensation that is already running or discard a review obligation: undoing and determining an uncertain effect are not work to abandon. What a stop does to known work depends on the saga's `cancelPolicy`: `compensate_completed` (default: stop what is in flight, undo what completed), `wait_in_flight_then_compensate` (let the attempts in flight finish, then undo everything known to have completed), `abort_no_compensation` (do not start compensation). None of these policies converts an unknown outcome into known failure.

## 2. A step whose outcome is unknown

Every leased `Activity` is external-capable, independently of `highRisk`. If an attempt ends without anyone knowing whether the effect happened — the worker died holding the lease (`LEASE_EXPIRED`), the attempt timed out (`ATTEMPT_TIMEOUT`), or the worker reported the reserved code `OUTCOME_UNKNOWN` — uncertainty takes precedence over retry, cancellation and explicit failure:

- An unknown original with `reconcileWith` waits (`waitingOn: reconciliation`) while lookup `<id>~r` receives the original idempotency key. `Applied` completes the original with the reported output. `NotApplied` is not proof of permanent absence: it holds review with `RECONCILIATION_NOT_FINAL`. `Unknown`, an invalid answer or exhausted lookup attempts also holds review. The platform does not issue a second original lease while this recovery is unresolved.
- An unknown original without `reconcileWith` goes directly to review. It is not defensively compensated; only an original known to have succeeded is a normal compensation candidate.
- An unknown compensation goes directly to review. It is not repeated automatically and explicit `fail` cannot terminalize it as though its outcome were known.

`CompleteStep` is the worker's assertion that the domain operation reached a final result. It is not provider proof created by the platform. A handler that receives accepted, queued or pending status reports `OUTCOME_UNKNOWN`, including for compensation.

## 3. Runs that wait for a person

`GET /runs?status=MANUAL_REVIEW` lists them. The runtime graph (`GET /runs/{id}/graph`) shows which step waits (`waitingOn: operator`, `failureCode` says why) — compensations appear as `<id>~c`, lookups as `<id>~r`.

Decide with `POST /runs/{id}/operations` (an `Idempotency-Key` header, a `reason` of 8 to 2000 characters after trimming that goes to the audit trail whole, with the outcome declared by a `RESOLVE_STEP`, optionally `expectedRunVersion` to refuse a decision taken on a stale view):

| Action | Body | Who | Effect |
|---|---|---|---|
| `RETRY_COMPENSATION` / `RETRY_STEP` | `stepId` | Operator or Admin with `run:operate` | Another attempt only when the previous outcome is known failed and the state remains retry-eligible, with the same idempotency key. An unknown original or compensation is refused with `409` even when the caller also has `saga:override`; resolve its outcome instead. |
| `SKIP_COMPENSATION` | `stepId` of a `…~c` | Admin with `run:operate` and `saga:override` | Give up on that compensation. The ones behind it go on. The saga ends `PARTIAL`. |
| `RESOLVE_STEP` | `stepId`, `outcome: SUCCEEDED` or `FAILED`, `errorCode` for `FAILED` | Admin with `run:operate` and `saga:override` | Record the authorized human conclusion after checking external evidence. The transaction retains actor, reason and declared outcome. Human `SUCCEEDED` may let compensation finish, but is not provider confirmation and does not qualify the integration. Resolution does not promise a new in-place attempt. |

A decision on a step that does not wait for a person, or that someone else already decided, is `409` and changes nothing. Every accepted decision is audited (`saga.step_retried`, `saga.step_skipped`, `saga.step_resolved`) with who, which step and why, and a run that waits for a person is not forgotten by a restart: it is a row.

Cancelling or timing out a run that waits for a person preserves the waiting step, its evidence and its decision authority across restart. It does not create a second lease, schedule defensive compensation for an unknown original, or turn an unknown compensation into terminal known failure.

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
| `403` on `SKIP_COMPENSATION` or `RESOLVE_STEP` | The caller lacks `saga:override` (and operation authority) | Use an authorized administrator; do not widen a worker/operator token |
| A run ends `CHILD_COMPENSATED` / `CHILD_PARTIAL` / `CHILD_COMPENSATION_FAILED` | Its child run ended in a saga outcome | Route on the code with an error edge, or look at the child run (its id is `child_run_id(parent, node)`) |
| A cancelled run is not `CANCELLED` yet | A compensation is running or waits for a person, or (when the run ended because *another branch* failed) a saga waits for one | Section 3 |
| `409` on retry or a decision | Someone decided first, the step is not eligible, or the effect remains unknown | Read the run again; resolve an unknown outcome before choosing any later action. A refused operation creates no success audit, idempotency success or scheduling outbox. |
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

### Stopping outbound work during an incident

Since feature 037, cancelling a run (or reaching its deadline) does not stop its pending
reconciliation lookups. A lookup is how an unknown effect gets resolved, so it survives the stop,
even when the run is also paused. To stop outbound calls to a provider you no longer trust, revoke
the worker binding for that activity, or the secret's access list. Both are checked when a lease
is granted. Then settle the waiting steps by hand.

## 7. Installation, rollback and retained evidence

Feature 037 delivery 1 upgrades **in place** from v0.2 (owner decision 2026-11-01, D1810). It adds no
migration and no wire change. Journals written by v0.1/v0.2 replay with their original meaning,
including a negative lookup that retried, an operator retry of an unknown outcome and a defensive
compensation plan. In-flight runs continue, and only new decisions follow the new rules
(`crates/domain/tests/legacy_replay.rs`). Upgrade as for any release (scaling runbook §3); do not
run old and new engines side by side.

**Rolling back to v0.2 is safe only before the new engine journals its first decision.** A v0.2
reducer refuses the new pairings (`not_applied` → review, `RECONCILIATION_NOT_FINAL`) with an
invariant error. After that point, stop new admission,
preserve the database and journal, keep operator ownership, and fix forward or perform an explicitly
reviewed reconciliation. Reverting code cannot undo a provider effect; replaying or re-triggering an
original request is not a rollback procedure.

The platform currently has no retention/partitioning control for the run journal (G28) and no
automatic retention or export for the audit trail (G182). Delivered outbox messages and some
ephemeral coordination records have their own cleanup, but that does not protect saga evidence.
Operators must size and monitor PostgreSQL, take and restore-test approved backups/PITR, and—when
policy requires external custody—page authorized records into an access-controlled archive and
document the transfer of responsibility. The platform does not claim that this satisfies a
regulator or supply an automatic evidence-export job.

For explicit `fail`, retained steps, journal and audit are the follow-up evidence. The terminal run
has no runnable or review-waiting compensation and there is no post-terminal queue; the integration
operator owns any external remediation and the retention period needed for it.

## 8. Not covered

The platform cannot prove a provider's finality, a compensation's idempotence or regulatory
retention from declarations and stable keys. No provider/capability is qualified in delivery 1;
US04 remains open and there is no qualification registry or exception path. Synthetic fixtures and
contract tests exercise engine behavior only. Historical design rationale remains in
[`specs/003`](../../specs/003-sagas-compensations/spec.md),
[`specs/014`](../../specs/014-workflow-primitives/spec.md), and
[ADR-0076](../adr/ADR-0076-compensation-failure-and-uncertain-effects.md), which remains **PROPOSED**.
The feature-037 delivery is implementation in verification: consult its
[validation report](../../specs/037-compensation-safety/validation-report.md) before claiming any gate
passed or using this runbook as evidence of provider safety.


Delivery-1 compatibility limit (037, owner decision 2026-11-01): ordinary `SECRET_IN_OUTPUT`
retains feature-009 final-failure behavior. Successful-result storage refusal now holds recovery,
but secret-bearing output can still take the existing failure/onError path without a review
queue. Follow up from safe retained evidence; never persist the prohibited secret for that
purpose. Existing managed-database safeguards are unchanged. See
[owner decisions](../owner-decisions.md).
