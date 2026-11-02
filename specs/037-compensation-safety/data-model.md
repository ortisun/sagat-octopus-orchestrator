# Compensation safety — proposed state model

Status: delivery-1 design; no database migration or runtime change has been applied.
Source types: `crates/domain/src/saga.rs`, `execution/{event,state,state_effects,instance}.rs`.

## Existing entities retained

| Entity | Identity / constraints | Change |
|---|---|---|
| Workflow version | Immutable published version/digest | Existing explicit `onCompensationFailure` only; no scope field or qualification property |
| Execution instance | Run + full InstancePath + node | Never collapse collection items/loop iterations into a root node |
| Attempt | Instance + attempt + generation/fence | Only a current lease/report may settle it; no identity regenerated for uncertainty |
| Effect / compensation | Existing original/target idempotency keys and derived companion identity | No new effect registry; review keeps those identities |
| Step | Existing statuses, failure code and WaitingOn | Use Operator/Reconciliation waiting; no new public status |
| Resolution | Existing StepResolved/StepSkipped plus authorized command audit | Actor and justification retained; human success is not provider confirmation |
| Run/saga outcome | Existing COMPENSATED, PARTIAL, COMPENSATION_FAILED, cancelled/timed-out paths | Terminal only after recovery obligations settle; no post-terminal scheduling |

The only proposed new data value is `RECONCILIATION_NOT_FINAL`, a FailureCode for a negative lookup
that cannot establish final absence. Existing constraint: **`[A-Za-z0-9_.:-]`, 1–64 characters**.
It belongs in `leaves_outcome_unknown`; it contains no provider payload. Existing event shapes and
serialization carry it. Replay must accept NotApplied + Review, including root and path reducers.

## Transition table (delivery 1)

| Input / context | Result | Forbidden fallback |
|---|---|---|
| Declared Activity, unknown outcome, reconciler present | WaitingOn::Reconciliation | Blind retry / defensive compensation |
| Declared Activity, unknown, no reconciler | WaitingOn::Operator | Inferring purity from risk flag |
| Compensation, unknown/accepted/pending | Operator review before retry/fail/cancel | Automatic or operator-requested retry before resolution |
| Reconciliation Applied | Original succeeded, using existing validated output/fence path | Treating provider acceptance as Applied |
| Reconciliation NotApplied | Operator review, RECONCILIATION_NOT_FINAL | RetryAt or failed-with-final-absence inference |
| Reconciliation Unknown/failure | Original review, retained cause | Recursive lookup or silent absence |
| Known compensation failure, default | Existing retry policy then operator review | Cancelling review on deadline |
| Known compensation failure, explicit fail | Existing retry policy then failed step; independent work continues | Early terminal while compensation is runnable/active/waiting |
| Compensation plan candidate | Original Succeeded, including audited manual success | Unknown Failed/Cancelled candidate |
| Human Retry on any unknown original/compensation | Typed state conflict; uncertainty retained | Treating privilege alone as an outcome resolution |
| Human resolve/skip | Existing authority and atomic audit; honest succeeded/failed/skipped evidence | Financial/provider-finality claim |
| Stop with unresolved effect/recovery review | Preserve obligation and exact instance | Cancelling the wait or continuing a new segment past it |

Commands and reducers must agree. For every unknown original or compensation, reject StepRetryRequested in the shared
reducer as well as entry commands. Known-failure retry remains available. Resolve records an
outcome; it is not a new retry-after-resolution operation. Planners and plan reducers both reject unknown compensation
candidates. Stop guards apply to root and path-local execution. Full replay and bounded path loading
must agree; no event may carry a policy result that the reducer treats differently.

No qualification entity is implemented in delivery 1. Eventual immutable qualification content and
current scoped authority require the US04 evidence gate in [plan](plan.md). Do not persist a fake
approved row or accept an undeclared qualification/purity property as an escape hatch.
