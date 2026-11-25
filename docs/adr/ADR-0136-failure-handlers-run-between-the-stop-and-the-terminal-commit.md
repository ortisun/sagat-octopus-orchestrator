# ADR-0136: A failure handler runs in a phase between the stop and the terminal commit, the namespace handler is an obligation admitted once, and an early result is set once before the run is terminal

**Status:** Proposed (revised 2026-11-25 after the reviews of `14b9e40b`; `specs/022-flow-control-step-policies/reviews/2026-11-25/`)
**Decided by:** agent proposal (feature 022, design of the rest of release 0.3.0, 2026-11-24) under the owner's delegation (OD-2026-11-24-4, "Tomem a melhor decisão"), revised with the coordinating session's rulings of 2026-11-25 (decisions D022-222 to D022-240); accepted once the three reviewers find no Blocker in a re-review (task T022-200)
**Refines / supersedes:** refines ADR-0056 decision 2 (the namespace failure-handler policy row is not locked; amended 2026-11-24 and 2026-11-25) and ADR-0050 decision 7 (the dispatch row's slot and the recorded deviation of the terminal commit's insert, amended 2026-11-25); depends on ADR-0146 (every end of a run goes through the reducer); builds on ADR-0010 (compensation first, derived `~` nodes), ADR-0012 and ADR-0025 (obligations), ADR-0039 (source identity guard), ADR-0094 (evaluation rounds before `persist`) and ADR-0050 decision 9 (fleet activation). Supersedes none.
**Enforced by:** proposed checks, built by the slices of US03 and US04 in `specs/022-flow-control-step-policies/tasks.md`: `crates/domain/tests/failure_handler.rs` (one test per case: a gated region step, a region step failing without `onError`, a cancellation, the run deadline, the timeout with a leased attempt, a stop request during the phase, no second phase; the status after the phase equals the pending status for every outcome) and `crates/domain/tests/early_return.rs` (a result set at most once and never after it is set); `verify_run` (invariants 3, 4, 5 and 16 of `data-model.md`); `integration_flow_control_step_policies::{failure_handlers, early_return, handler_dispatch}`; `fault_flow_control_step_policies::handlers` (a crash at each commit of the phase and of the dispatch, a duplicate dispatcher, a crash inside the admission still reaching `DEAD`, exactly one handler run per failure and per lineage); the unique indexes on `runs (tenant_id, handler_for_run_id)` and on the dispatch rows' lineage; the rule of `scripts/check-architecture.py` that only the terminal path of `persist` inserts a dispatch row and only the dispatcher and the redrive lock one (task T022-225); the slot-order references of `tests/fault/schedules-bulk-operations/slot_order.rs` extended to the dispatch (task T022-224).

## Context

Feature 022 US03 (FR-022-012 to FR-022-016) adds a workflow `onFailure` handler (a region of nodes or a child workflow)
and a namespace default handler; US04 (FR-022-008 to FR-022-011) adds `earlyReturn`, which sets the run's result before
trailing steps finish. Both act at the end of a run, the busiest commit of the engine. The tree constrains them:

- A stopped run drains (`crates/domain/src/execution/plan.rs:306-308`, `:523`, `:602-606`), a second stop is refused
  (`state.rs:4636-4639`; `409` at `crates/application/src/runs.rs:3344`), a stopped run reads `CANCELLING`
  (`state.rs:433-434`), and `check_finish` refuses `run.finished` while a step is active, or with a status other than
  `SUCCEEDED` without a stop (`state.rs:5133-5163`). No handler work can run under the stop as it is.
- Several paths end a run without the reducer (gap G1197; ADR-0146 lists them). A handler phase, a namespace dispatch, a
  start permit's release and the `result_already_returned` mark would be skipped there.
- A run that declares a start gate keeps its permit until its terminal commit (T022-022, D022-049); a run that never
  acquired it must not run its workflow handler outside the key's limit (D1197).
- Every run creator must admit through one port, `Engine::start` (ADR-0056 decision 3, D022-049); a handler run and a
  derived handler child are run creators.
- A run-creating aggregate (slot 3 of ADR-0050 decision 7) may not be written by a terminal transition, and
  publication locks a workflow row before the namespace's secrets (`crates/application/src/definitions.rs:618-642`).
- A handler that may turn a failure into success misstates what happened after compensation (O1161), and a handler that
  reruns its failed run under an ambient authority could loop without bound.

## Decision

1. **The handler phase is its own reducer state.** Where the planner would emit `run.finished {P}` for a run whose
   version declares `onFailure` with `P` in its `on` set, and which was granted every gate of its start (or declares
   none), it MUST first finish the existing stop (unstarted work cancelled, in-flight attempts settled, compensation and
   reconciliation ended; a run in `MANUAL_REVIEW` waits), then emit `failure_handler.started {P}`. That event MUST move
   the stop into `runs.pending_terminal_status = P` and clear the stop, so the run reads `RUNNING` or `WAITING`
   (`runs.failure_handler_state = RUNNING`) and the planner plans the handler's work, and only it; the run keeps its
   start's permit. `run.finished {P}` MUST be emitted only in the commit that emits `failure_handler.finished
   {outcome}`, with the start's `concurrency.released` before it; `check_finish` accepts it there and nowhere else.
2. **The phase ends once.** A region step that fails without an `onError` route ends the phase `FAILED`; a phase never
   opens a second phase. A cancellation request or the run deadline during the phase is a handler cancellation, and the
   handler's own timer (`timeoutMs` from `failure_handler.started`, existing only during the phase and cancelled by
   `failure_handler.finished`) a handler timeout: a draining sub-state (`failure_handler_state` `CANCELLING` or
   `TIMING_OUT`) asks the region's leased attempts and the `onFailure~h` child to stop, and its last settle commits
   `failure_handler.finished` (`CANCELLED` or `FAILURE_HANDLER_TIMEOUT`, the column reading `CANCELLED` or `TIMED_OUT`),
   the permit's release and `run.finished {P}`. A stop request during the phase is accepted as that cancellation, never
   refused.
3. **The two forms reuse what exists.** The region form enters a region of the root graph (decision D022-210): its nodes
   are root nodes, decided by the same lookup and gate rounds; publication refuses `compensateWith` in it (no new
   compensable work after the stop). The child form is the derived child node `onFailure~h` (the ADR-0010 `~`
   precedent), created through `Evaluation::Child` and admitted through `Engine::start`; publication refuses a child
   form whose workflow's start permit shares the parent's group and a key that can coincide, which could never be
   granted while the parent holds its permit (`FAILURE_HANDLER_SELF_GATE`). The handler receives the closed,
   payload-free failure context `{status, code, node, attempt, kind, occurredAt}`.
4. **A handler never changes the outcome and holds no authority of its own.** Whatever the handler's outcome, the run
   MUST finish with `P`. A `CANCELLED` run never enters the phase. Recovery is a new run: a handler that wants the work
   done again calls the public API with a machine identity it is given as a secret, audited as that identity. No run is
   granted its publisher's operator rights.
5. **Handler runs are a tree of their own.** A run carries `handler_tree`, set for a dispatched handler run, an
   `onFailure~h` child, every child of a handler-tree run, and every reset or rerun whose source or initiator is a
   handler-tree run; `handler_kind` (`DISPATCHED`, `REGION_CHILD`) says which kind of handler run it is. A handler-tree
   run MUST NOT be dispatched to the namespace handler. `handler_for_run_id` and its unique index apply to dispatched
   runs only, and a rerun or reset of a handler run inherits neither.
6. **No workflow handler without the start's grants.** A run that ends before it was ever granted its start's gates
   (queue timeout, key error at admission) MUST finish directly; only the namespace handler may follow.
7. **The namespace handler is an obligation admitted once per failure and per lineage.** A run qualifies when its status
   is in the configuration's statuses, the configuration is enabled, no workflow handler phase of the run ended
   `SUCCEEDED`, it is not a handler-tree run, not a test tree, and it was not cancelled (a run cancelled during its
   handler phase is not dispatched: the operator decided). The configuration's revision and enabled flag are read with
   the run's definition, in the statement that loads it, without a lock. The terminal commit of a qualifying run MUST
   insert one dispatch row keyed by `(tenant_id, namespace_id, source_run_id)`, recording the `config_revision` read and
   the failure context, and always does: the pending-dispatch ceiling refuses a new or re-enabled configuration and
   raises an alert, never a terminal commit. This insert is a recorded deviation of ADR-0050 decision 7 (a terminal
   transition writes a slot-3 row): the row is invisible until the commit, its key is the failed run, whose lock
   serializes every writer of it, and the dispatcher locks only committed rows `SKIP LOCKED`, so no cycle can pass
   through it. A namespace handler is dispatched at most once per lineage (the source chain of resets and reruns,
   `lineage_run_id`), by a unique index on the dispatch rows; a reset or rerun extends its source's lineage only when its initiator is a machine identity or a handler-tree run; a reset or rerun a person (a member) starts begins a lineage of its own (O1176 answered (b), decision D022-241).
8. **The dispatcher counts first, then admits.** A shard-fenced pass counts each attempt (attempts, `next_attempt_at`)
   in its own committed transaction, then admits the handler run through `Engine::start` (initiator `system:engine`; its steps run under the
   handler version's publisher as a triggered run's do, D1177, with no operator right; provenance `handlerFor`) and marks the row
   `ADMITTED` in that transaction, locking the row inside `RunStore::create` through a `NewRun` field (the
   `pending_reset` precedent), in ADR-0050's slots: the dispatch row (slot 3), the source run's identity guard
   (`KEY SHARE`, slot 3R), the handler workflow's head read without a lock, the new run (slot 6), its gates (slot 7), the
   row's update (8a), usage rows (8b). After 12 counted attempts the row is `DEAD`, alerted and redrivable. A crash
   inside the admission still reaches `DEAD`.
9. **Every end of a run goes through the reducer.** The handler phase, the namespace dispatch and the early-result mark
   MUST be written by `persist` from the planner's events; the capabilities `failure-handler-v1` and `early-return-v1`
   are listed by a binary only once ADR-0146's routing and every slice of their story are in it. Until the routing
   lands, publication refuses `onFailure` and `earlyReturn` in a definition with a `forEach`, `while` or
   `repeatUntil` node (`UNSUPPORTED_NODE`, dependency `COLLECTION_FINISH_ROUTING`).
10. **An early result is set once, before the run is terminal.** Reaching an `earlyReturn` node, the planner MUST ask
    `Evaluation::EarlyResult`; in that commit `persist` writes the result's payload, `runs.result_digest`,
    `result_source = EARLY_RETURN`, `result_available_at`, the journal event `result.returned` and the payload-free
    `run.result_returned` outbox (none for a test tree), and the planner schedules `next`. At most one of `earlyReturn`
    or `end.outputExpr` sets a run's result; a later `end` records `result_already_returned`. Nothing after it changes the
    result. Placement is the root graph only, never in a saga body, a parallel branch, a body or a handler region.
11. **Reset rules.** Every reset point inside a handler phase is refused; `result.returned` is re-materialized; a reset
    run never inherits `handler_for` (`contracts/journal-events.md`).

## Considered options

1. **A phase that is a reducer state of the run, a dispatch obligation with a lineage bound, a result set once** —
   chosen. The run stays one state machine; the drain rules stay as they are because the phase clears the stop; exactly
   one handler run per failure follows from one transaction and a unique index, and the lineage index bounds recovery.
2. **The handler under the stop** (the first text: the phase "between the stop and the terminal commit" without leaving
   the stop) — rejected after the reviews: the planner drains a stopped run and `check_finish` refuses its end while a
   step is active, so no handler step could run.
3. **Only the child form** — rejected: it drops the region form the specification approved, and a notifier region then
   costs a run per failure.
4. **Handlers as outbound subscriptions** — rejected: an at-least-once external send would model an internal admission.
5. **A handler that may recover the run** — rejected (O1161).
6. **A handler that resets under its publisher's authority** (the first text) — rejected after the reviews: ambient
   authority that no authorization row shows, and a loop with no bound.
7. **The namespace handler run admitted in the failed run's terminal commit** — rejected: one commit would create a
   second run after the failed run's own lock, on the busiest commit.
8. **The policy row `FOR SHARE` on the terminal path** — rejected: a lock for every failing run of the namespace; the
   revision read with the definition gives the same answer, auditable, at no statement.
9. **A dispatch row written by a second transaction after the terminal commit** — rejected: a crash between the two
   loses the obligation.

## Consequences

- Good: a failed run's handler starts after compensation, sees a closed context and never changes the outcome; exactly
  one namespace handler run per failure and per lineage whatever the dispatcher crashes; an early result is disclosed
  while the run runs and survives its later failure.
- Bad: observers see a failed run as `RUNNING` with a pending status for the length of its handler phase; a handler step
  behind a busy key can time out before it runs (gap G022-027); a handler that reruns its failed run through a machine identity
  is dispatched once per lineage; a rerun a person starts begins a new lineage and is dispatched again (O1176, decision
  D022-241).
- Harder: every future path that ends a run must go through the reducer (ADR-0146); every future creator copies
  `handler_tree` to its children.
- Concurrency of configuration: a change that commits while a failing run's transaction is in flight applies to that
  failure or not by what its definition statement read; the dispatch records which revision (gap G022-024).
- Compatibility and rollback: additive IR, events, columns and tables; activation through the registry. Once a journal
  holds the new events an older binary cannot decode the run: rollback is switching the capability off, never a binary
  downgrade; runs already in a phase finish it; the dispatcher keeps running until pending dispatches drain.

## Evidence

None yet: the tests named under **Enforced by** are built by tasks T022-030 to T022-038, T022-207, T022-208, T022-213 to
T022-215, T022-224, T022-225, T022-227 and T022-228. Spec: FR-022-008 to FR-022-016 and their amendments of 2026-11-24
and 2026-11-25; research: D1169 to D1177, D1195, D1197, D022-201, D022-207, D022-208, D022-210, D022-215, D022-216,
D022-222 to D022-224, D022-228, D022-237; data model: the handler tables, the `runs` columns and invariants 3 to 5 and
16.
