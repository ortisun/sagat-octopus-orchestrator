# ADR-0136: A failure handler runs in a phase between the stop and the terminal commit, the namespace handler is an obligation admitted once, and an early result is set once before the run is terminal

**Status:** Proposed
**Decided by:** agent proposal (feature 022, design of the rest of release 0.3.0, 2026-11-24) under the owner's delegation (OD-2026-11-24-4, "Tomem a melhor decisão"); the coordinating session accepts or rejects it after the independent architecture, correctness and security reviews (task T022-200)
**Refines / supersedes:** refines ADR-0056 decision 2 (the namespace failure-handler policy row is read without a lock, amended 2026-11-24) and ADR-0050 decision 7 (where the dispatch row and the failed run sit in the merged admission order, amended 2026-11-24); builds on ADR-0010 (compensation first, derived `~` nodes), ADR-0012 and ADR-0025 (obligations), ADR-0039 (source identity guard), ADR-0094 (evaluation rounds before `persist`; its decision 4 is amended for the finish paths) and ADR-0050 decision 9 (fleet activation). Supersedes none.
**Enforced by:** proposed checks, built by the slices of US03 and US04 in `specs/022-flow-control-step-policies/tasks.md`: `crates/domain/tests/failure_handler.rs` and `crates/domain/tests/early_return.rs` (the planner never emits `run.finished` for a run in its handler phase; the status after the phase equals the pending status for every handler outcome; a result is set at most once and never after it is set); `verify_run` (invariants 3, 4 and 5 of `data-model.md`); `integration_flow_control_step_policies::{failure_handlers, early_return, handler_dispatch}`; `fault_flow_control_step_policies::handlers` (a crash at each commit of the phase and of the dispatch, a duplicate dispatcher, exactly one handler run per failure); the unique index on `runs (tenant_id, handler_for_run_id)`; the admission slot-order references of `tests/fault/schedules-bulk-operations/slot_order.rs` extended to the dispatch (task T022-214).

## Context

Feature 022 US03 (FR-022-012 to FR-022-016) adds a workflow `onFailure` handler (a region of nodes or a child workflow)
and a namespace default handler; US04 (FR-022-008 to FR-022-011) adds `earlyReturn`, which sets the run's result before
trailing steps finish. Both act at the end of a run, the busiest commit of the engine. The slice built since and the code
on the tree constrain them:

- The run's end is decided by the planner and written by `persist`, except on two paths: the collection finish
  (`crates/infra-postgres/src/primitives.rs`, near lines 3482 to 3510) and the loop finish (`loop_progress.rs`, near
  lines 764 to 785) write `run.finished` themselves (gap G1197). A handler phase, a namespace dispatch, a start permit's
  release and the `result_already_returned` mark would all be skipped there.
- A run that declares a start gate keeps its permit until its terminal commit (T022-022, D022-049); a run that never
  acquired it must not run its workflow handler outside the key's limit (D1197).
- Every run creator must admit through one port, `Engine::start` (ADR-0056 decision 3, D022-049); a handler run and a
  derived handler child are run creators.
- The approved plan locks the namespace handler policy row `FOR SHARE` on the terminal path; that is one lock more on the
  busiest commit, for every failing run of a namespace.
- A handler that may turn a failure into success misstates what happened after compensation (O1161).

## Decision

1. **The handler phase sits between the stop and the terminal commit.** Where the planner would emit `run.finished {P}`
   for a run whose version declares `onFailure` with `P` in its `on` set, and which held its start's grants (or declares
   no start gate), it MUST first finish the existing stop (unstarted work cancelled, in-flight attempts settled,
   compensation and reconciliation ended; a run in `MANUAL_REVIEW` waits), then emit `failure_handler.started {P}` instead
   of `run.finished`. The run stays live (`RUNNING` or `WAITING`, `runs.failure_handler_state = RUNNING`,
   `runs.pending_terminal_status = P`) and keeps its start's permit. `run.finished {P}` MUST be emitted only in the commit
   that emits `failure_handler.finished {outcome}`, with the start's `concurrency.released` before it.
2. **The two forms reuse what exists.** The region form enters a region of the root graph (decision D022-210 of the
   feature's research): its nodes are root nodes, decided by the same lookup and gate rounds. The child form is the
   derived child node `onFailure~h` (the ADR-0010 `~` precedent), created through `Evaluation::Child` and admitted through
   `Engine::start`. The handler receives the closed, payload-free failure context `{status, code, node, attempt, kind,
   occurredAt}`.
3. **A handler never changes the outcome.** Whatever the handler's outcome (succeeded, failed, timed out at `timeoutMs`
   from `failure_handler.started` by a timer, cancelled), the run MUST finish with `P`. A handler's failure MUST NOT
   invoke it again. A cancellation during the phase stops the handler's work and finishes with `P`; a `CANCELLED` run
   never enters the phase. Recovery is a handler that starts a reset or a rerun as new: an audited new run under the
   handler's authority (decision D022-201).
4. **No workflow handler without the start's grants.** A run that ends before it was ever granted its start's gates
   (queue timeout, key error at admission) MUST finish directly; only the namespace handler may follow.
5. **The namespace handler is an obligation admitted once.** The terminal commit of a run that qualifies (its status is in
   the configuration's statuses, the configuration is enabled, its version declares no `onFailure` or its handler did not
   succeed, it is not a handler run, not a test tree, not cancelled) MUST read the policy row without a lock and insert one
   dispatch row keyed by `(tenant_id, source_run_id)`, recording the `config_revision` it read and the failure context.
   A shard-fenced dispatcher MUST admit the handler run through `Engine::start` (initiator `system:engine`, authority of
   the handler version's publisher, provenance `handlerFor`) and mark the row `ADMITTED` with the run's id in the same
   transaction, taking its locks in ADR-0050's slots: the dispatch row (slot 3), the source run's identity guard
   (`KEY SHARE`, slot 3R), the handler workflow's head (slot 4b), the new run (slot 6), its gates (slot 7), usage rows
   (slot 8b). A refused admission is recorded in a separate transaction (attempts, backoff, `DEAD` after 12, alerted and
   redrivable). The unique index on `runs (tenant_id, handler_for_run_id)` makes a second handler run of one failure
   impossible.
6. **Every end of a run goes through the reducer.** The handler phase, the namespace dispatch and the early-result mark
   MUST be written by `persist` from the planner's events; the collection- and loop-finish paths MUST be routed through
   the reducer (decision D022-207) before the capabilities `failure-handler-v1` and `early-return-v1` can be activated.
   Until then publication refuses `onFailure` and `earlyReturn` in a definition with a `forEach`, `while` or
   `repeatUntil` node (`UNSUPPORTED_NODE`, dependency `COLLECTION_FINISH_ROUTING`).
7. **An early result is set once, before the run is terminal.** Reaching an `earlyReturn` node, the planner MUST ask
   `Evaluation::EarlyResult`; in that commit `persist` writes the result's payload, `runs.result_digest`,
   `result_source = EARLY_RETURN`, `result_available_at`, the journal event `result.returned` and the payload-free
   `run.result_returned` outbox (none for a test tree), and the planner schedules `next`. At most one of `earlyReturn` or
   `end.outputExpr` sets a run's result; a later `end` records `result_already_returned`. Nothing after it (a failure, the
   handler phase, a cancellation, a deadline) changes the result; the status is whatever the rest of the run produces.
   Placement is the root graph only, never in a saga body, a parallel branch, a body or a handler region.
8. **Reset rules.** Every reset point inside a handler phase (after `failure_handler.started`) is refused; `result.returned`
   is re-materialized; a reset run never inherits `handler_for` (`contracts/journal-events.md`).
9. **Fleet activation.** `failure-handler-v1` and `early-return-v1` are registry capabilities required of control-plane
   and engine processes; until activation their use is refused (`FLOW_CONTROL_ACTIVATION_PENDING`).

## Considered options

1. **A phase in the run's own state machine, a dispatch obligation, a result set once** — chosen. The run stays one
   state machine: replay and `verify_run` need three events and two columns; pause, cancellation, the rounds and the
   permit's lifetime apply to the phase unchanged; exactly one handler run per failure follows from one transaction and a
   unique index.
2. **Handlers as outbound subscriptions (a webhook on `run.finished`)** — rejected: an at-least-once external send would
   model an internal admission; there is no exactly-one handler run, no closed context and no provenance.
3. **A handler that may recover the run** (Windmill's `recover: true`) — rejected (O1161): after compensation it misstates
   what happened, and it needs a transition from a stopped run to a status other than the pending one that no reset rule
   or replay covers.
4. **The handler inside the stop, before compensation** — rejected: the handler would see an unsettled run, and a slow or
   failing handler would delay compensation, which ADR-0010 puts first.
5. **The namespace handler run admitted in the failed run's terminal commit** — rejected: one commit would create a second
   run and take its admission locks (slots 1 to 6) after the failed run's own lock, the cross-run pattern ADR-0056 option
   2 rejected, on the busiest commit.
6. **The policy row `FOR SHARE` on the terminal path**, as the approved plan says — rejected (decision D022-208 of the
   feature's research): a lock for every failing run of the namespace that every configuration write waits for; the
   statement's snapshot and the recorded revision give the same answer, auditable.
7. **A second, "early" result beside the final one** — rejected (D1169): two disclosure paths for one answer.
8. **A handler region as a named body** — rejected (decision D022-210): it would run under 014's exact-path
   transactions, where cache and gates are refused in 0.3.0, for a region that has one instance.
9. **Each finish path taught the new rules** instead of routing them through the reducer — rejected: three copies of the
   terminal logic, the "guard each caller" shape ADR-0089 rejected.

## Consequences

- Good: a failed run's handler starts after compensation, sees a closed context and never changes the outcome; exactly
  one namespace handler run per failure whatever the dispatcher crashes; an early result is disclosed while the run runs
  and survives its later failure.
- Bad: a failing run in a namespace with a default handler costs the terminal commit one more statement (the plain read
  of the policy row) and one dispatch row insert; observers see a failed run as `RUNNING` with a pending status for the
  length of its handler phase; a handler step behind a busy key can time out before it runs (gap G022-027).
- Harder: the finish-path routing must land first (decision D022-207); every future path that ends a run must go through
  the reducer.
- Concurrency of configuration: a change that commits while a terminal commit is in flight applies to that failure or
  not, by commit order; the dispatch records which revision (gap G022-024).
- Compatibility and rollback: additive IR (`onFailure`, `earlyReturn`), events, columns and tables; activation through the
  registry. Disabling a capability stops new publications and starts; runs already in a phase finish it with compatible
  readers; the dispatcher keeps running until pending dispatches drain.

## Evidence

None yet: the tests named under **Enforced by** are built by tasks T022-030 to T022-038, T022-207, T022-208 and T022-213
to T022-215. Spec: FR-022-008 to FR-022-016 and their amendments of 2026-11-24; research: D1169 to D1177, D1195, D1197,
D022-201, D022-207, D022-208, D022-210, D022-215, D022-216; data model: the handler tables, the `runs` columns and
invariants 3 to 5.
