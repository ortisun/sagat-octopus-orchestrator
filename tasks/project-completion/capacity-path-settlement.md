# Capacity scoped-path settlement — B1b.2a candidate

Status: reviewed evidence contract; no schema, writer or runtime activation.
One existing ScopedPath member Finish only; B1b.2 remains open.
Refs:T014-014..017/040, ADR-0030, A1, B0, B1a, B1b.1.

## Evidence decision

A ScopedPath membership binds immutably to tenant/namespace/runId/nonempty InstancePath.
Its owner remains that exact run/path across continuation; it never moves to a successor.
The immediate scope owner and iteration index must match the path and actual persisted admission.

Finish requires explicit persisted settlement evidence for that exact path, established from
committed execution facts by an authorized source writer. A caller supplies identity and expected
revisions, never settled=true. Missing or contradictory evidence refuses Finish.

The evidence means that the path's forward result/drain was durably consumed and every source
capable of creating further work in this same path has closed. All accepted work and effect
obligations belonging to this path reached recorded final dispositions. Closure includes deferred
compensation/reconciliation; absence of currently executable steps does not establish it.

Finishing this member changes only its A1 membership/header contribution. Independently retained
descendants keep membership and possession. A parent's satisfied/detached relationship cannot
settle a child. No Acquire/Vacate authority, quiescence or execution eligibility is established.

## Required persisted own-path facts

A proposed immutable settlement record must bind, without business values:

- Exact tenant/namespace/run/path, pinned definition version and admitted scope-owner/index.
- The committed source sequence for that iteration's consumed result/carry checkpoint or drain.
  A returned body awaiting checkpoint/reduction is open.
- The committed decision closing every applicable producer of future work in this path, including
  compensation not yet planned. No saga row is not a closure decision.
- Final disposition/source sequence of compensation, reconciliation and operator obligations.
  Preserve accepted PARTIAL/COMPENSATION_FAILED outcomes; unresolved operator waits are not settled.
- Versioned immutable settlement identity and source revisions/sequences used to establish it,
  retained for the required verification/replay window.

These semantics are absent from current storage. A separate source-writer contract must derive
these facts under the own-run guard and prevent every applicable later writer from adding work
to the settled path. Storing assertions with these names is insufficient.
For an initial supported leaf body, exact checkpoint/drain plus owning scope completed reduction
and final compensation decision can contribute evidence. Neither a terminal scope label nor an
absent saga suffices. Nested controllers, child relationships and broader bodies need separately
specified source-closure rules before they may produce settlement evidence.

## Finish guards and atomic boundary

Retain applicable own-chain → exact original own-run → selected G NOWAIT under B1a. Recheck
namespace/member binding/admitted path/settlement identity/expected revisions under G. Retain the
applicable own-run scheduler guard and sample DB time after its wait. A scoped original-run owner
is not the ChildRoot current-head identity; this unit does not substitute the existing root port.

Read exact scalar source facts or validated immutable settlement evidence, without run history,
sibling hydration, foreign business state or group registry scans. Permitted bounded same-run
ancestry and query bounds must be decided in the source-writer contract.
Valid evidence authorizes only selected A1 Finish, with header/member/receipt/audit/reference hint
atomic. Contention consumes TX; unknown COMMIT reconciles the original key/fingerprint in a fresh
TX. Replay retains current authority checks. Scoped binding/schema/fingerprint/receipt/retention
require separately reviewed extensions; changing CHILD_ROOT to another tag is insufficient.

## Required evidence before scoped storage

1. CHECKPOINTED/DRAINED followed by later compensation planning refuses Finish. Inactive ancestry,
   terminal run and CONTINUED alone refuse too.
2. Returned/unreduced results, pending compensation/reconciliation, unknown effects and unresolved
   operator waits refuse settlement; accepted adverse final outcomes remain visible.
3. Exact-path closure survives restart with the same immutable proof; another index/owner/namespace/
   run/successor cannot supply it.
4. Held-lock races between proof creation and actual work-producing writers prove closure in both
   orders, including later compensation planning.
5. Finish preserves retained descendants and another holder's epoch. Duplicate/stale/rollback/
   uncertain-COMMIT cases preserve B1a/A1.
6. Selected-path/ancestry reads stay bounded independently of historical iterations/foreign runs.

## Source observations and separate next units

PathExecutionState (crates/domain/src/execution/path_state.rs:517) holds exact-path state plus
ancestor eligibility fences; allows_forward_progress at506 is not settlement. path_read.rs:235
loads ancestor slot/active state and303 saga projections, without immutable closure evidence.
state.rs:4191-4274 plans compensation after iteration drain and adds derived steps in those paths;
primitives.rs:222 retains compensation reservations after drain. Collection completion checks
scope compensation (state.rs:2888), whereas loop success can close through checkpoint continuation
(state.rs:3248), so one generic finish_sequence predicate would be incorrect. Scope compensation
completion is checked atstate.rs:4291; persisted ended_sequence is atinfra-postgres/execution.rs:5326.
The dormant capacity schema20261210000000 explicitly permits only CHILD_ROOT.

Next units: choose/prove one leaf-shape settlement writer including latent compensation closure;
review scoped binding/receipt/retention extension; define nested/child relationship closure; define
quiescence/transition authority before Vacate/Acquire. This candidate closes no B1b.2/B1/B2 task
and permits no publication, production caller, schema migration or nested execution.


Independent fresh Astra/xhigh bounded review cycle1 found no concrete issue after cross-checking
all source observations. Root inspected the referenced source too. This is documentary evidence,
not a new test or implementation result; no external review CLI (existing user selection).
