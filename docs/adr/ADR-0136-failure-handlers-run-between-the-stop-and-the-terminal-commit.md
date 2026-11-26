# ADR-0136: A failure handler runs in a phase between the stop and the terminal commit, the namespace handler is an obligation admitted once, and an early result is set once before the run is terminal

**Status:** Accepted (2026-11-25, delegate OD-2026-11-24-4, after three review rounds recorded in specs/022-flow-control-step-policies/reviews/2026-11-25/); revised after the reviews of `14b9e40b`, the re-reviews of `6d02632e` and the third reads of `c910345b` (decisions D022-222 to D022-271); revised 2026-11-26 after the re-reads of `1276d5b7` and the second security re-read of `9960a3c1` (decisions D022-272 to D022-296)
**Decided by:** agent proposal (feature 022, design of the rest of release 0.3.0, 2026-11-24) under the owner's delegation (OD-2026-11-24-4, "Tomem a melhor decisão"), revised with the coordinating session's rulings of 2026-11-25 (decisions D022-222 to D022-240, then D022-242 to D022-259); accepted on 2026-11-25 by the coordinating session, delegate (OD-2026-11-24-4), after three review rounds (decision D022-271); revised with the re-reads of 2026-11-26 (decisions D022-272 to D022-285)
**Refines / supersedes:** refines ADR-0056 decision 2 (the namespace failure-handler policy row is not locked; amended 2026-11-24 and 2026-11-25) and ADR-0050 decision 7 (the dispatch row's slot and the recorded deviation of the terminal commit's insert, amended 2026-11-25); depends on ADR-0146 (every end of a run goes through the reducer); builds on ADR-0010 (compensation first, derived `~` nodes), ADR-0012 and ADR-0025 (obligations), ADR-0039 (source identity guard), ADR-0094 (evaluation rounds before `persist`) and ADR-0050 decision 9 (fleet activation). Supersedes none.
**Enforced by:** proposed checks, built by the slices of US03 and US04 in `specs/022-flow-control-step-policies/tasks.md`: `crates/domain/tests/failure_handler.rs` (one test per case: a gated region step, a region step failing without `onError`, a cancellation, ~~the run deadline~~ (struck 2026-11-25, decision D022-270: the run deadline does not apply during the phase; the overdue pass's no-op is tested instead), the timeout with a leased attempt, a stop request during the phase, a further stop refused `409` while draining, no second phase; the status after the phase equals the pending status for every outcome) and `crates/domain/tests/early_return.rs` (a result set at most once and never after it is set); `verify_run` (invariants 3, 4, 5 and 16 of `data-model.md`); `integration_flow_control_step_policies::{failure_handlers, early_return, handler_dispatch}`; `fault_flow_control_step_policies::handlers` (a crash at each commit of the phase and of the dispatch, a duplicate dispatcher, a crash inside the admission still reaching `DEAD`, exactly one handler run per failure and per lineage, two failing runs of one lineage committing at once with one live and one suppressed row); the unique indexes on `runs (tenant_id, handler_for_run_id)` and on the dispatch rows' lineage; the rule of `scripts/check-architecture.py` that only the terminal path of `persist` inserts a dispatch row and only the dispatcher and the redrive lock one (task T022-225); the slot-order references of `tests/fault/schedules-bulk-operations/slot_order.rs` extended to the dispatch (task T022-224). *Added 2026-11-26 (decisions D022-272, D022-277, D022-278, D022-279):* the domain tests of the principal's namespace scope (task T022-256); the battery of every route as a `RunRecovery` subject against an unbound namespace (task T022-257); the rule of `scripts/check-architecture.py` on role-only checks and the scope classification (task T022-258); the `slot_order.rs` case of a `RunRecovery` admission against a revocation of its binding; the domain case that the overdue pass is a no-op during the phase while the handler timer drains (task T022-243); a case per creator of a continue-as-new refusal under a handler tree, resets and reruns included (task T022-244).

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
  publication locks a workflow row before the namespace's secrets (`crates/infra-postgres/src/definitions.rs:618-642`).
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
2. **The phase ends once, and its draining is a journal fact.** A region step that fails without an `onError` route
   ends the phase. During the phase every stop source (`run.stop_requested`, the storage-quota stop, a region step
   failing without `onError`, an expression error in a region condition or switch, the handler's own timer) MUST map to
   one journaled event `failure_handler.draining {cause}`, cause one of `CANCELLED`, `TIMEOUT`, `REGION_FAILED`,
   `STORAGE_QUOTA`, `RUN_DATA_LIMIT`, `EXPRESSION_ERROR`, and never to the run's stop. There is one cause: while the run
   drains, a further stop request (an operator's cancellation, a second timeout) is refused `409`, as a second stop is
   today, and region step failures settle into the drain without adding a cause; qualification reads the one journaled
   cause (decision D022-261; the first text recorded later causes in its detail). Draining (`failure_handler_state` `CANCELLING` for `CANCELLED`, `TIMING_OUT` for
   `TIMEOUT`, `CANCELLING` for the others) asks the region's leased attempts and the `onFailure~h` child to stop; it
   overrides a pause as a stop does, and the recovery sweep also selects draining runs; its last settle commits
   `failure_handler.finished` (`CANCELLED`, `FAILURE_HANDLER_TIMEOUT`, or `FAILED` for the other causes; the column
   `CANCELLED`, `TIMED_OUT` or `FAILED`), the permit's release and `run.finished {P}`. The column follows the journal. A
   phase never opens a second phase. **The run deadline does not apply during the phase**: only the handler's own timer
   bounds it (`timeoutMs`, created by the commit that opens the phase, cancelled by `failure_handler.finished`, at most
   3,600,000 ms), and the overdue pass excludes runs with `failure_handler_state IS NOT NULL`; a run that opens its phase
   between the pass's pick and its lock is skipped by the pass: the run deadline is a deadline source, not a stop
   reason, and the pass calls `enforce_run_deadline`, a typed no-op under the run's lock for a run in its phase, counted
   `skipped_handler_phase`, while `stop_run(TimedOut)` keeps its meaning, so the handler's own timer still drains the
   phase with cause `TIMEOUT` (decision D022-278, which restates D022-262); a deadline that fires
   before the phase opens is the run's stop, and the phase then opens normally. *(Revised 2026-11-25, decisions D022-246
   and D022-247: the first text treated the run deadline as a cancellation of the phase, which would cancel a handler
   of a `TIMED_OUT` run at the next pass; the ruling's cause `RUN_DEADLINE` is therefore unreachable and is not in the
   closed set.)*
3. **The two forms reuse what exists.** The region form enters a region of the root graph (decision D022-210): its nodes
   are root nodes, decided by the same lookup and gate rounds; publication refuses `compensateWith` in it (no new
   compensable work after the stop); the runs its `child` nodes start are handler-tree runs. The child form is the derived child node `onFailure~h` (the ADR-0010 `~`
   precedent), created through `Evaluation::Child` and admitted through `Engine::start`; publication refuses a child
   form whose workflow's start permit shares the parent's group and a key that can coincide, which could never be
   granted while the parent holds its permit (`FAILURE_HANDLER_SELF_GATE`; the direct case only: a longer cycle is
   bounded by the queue and handler timeouts). The derived `onFailure~h` node cancels and awaits its child when the
   phase drains. The handler receives the closed,
   payload-free failure context `{status, code, node, attempt, kind, occurredAt}`.
4. **A handler never changes the outcome and holds no authority of its own.** Whatever the handler's outcome, the run
   MUST finish with `P`. A `CANCELLED` run never enters the phase. Recovery is a new run: a handler that wants the work
   done again calls the public API with a credential of the built-in role `RunRecovery` (`run:read`, `run:start`,
   `run:operate`, and no publication, approval, version or secret permission) given to it as a secret, audited as that
   subject; a subject whose only role is `RunRecovery` is refused as a publication approver and as an approval decider.
   A `RunRecovery` subject (stored role `run-recovery`) is bound to named namespaces (an administrator's audited
   setting, as worker bindings are); the role also holds `definition:read`; it may not redrive a dispatch (`403`,
   `RECOVERY_ROLE_MAY_NOT_REDRIVE`); it cannot be assigned until `failure-handler-v1` is activated (decision D022-266).
   *Revised 2026-11-26 (decisions D022-272 to D022-277):* the binding is enforced by the principal, never per route.
   `Principal` carries a namespace scope, built only by `Membership::principal()` from the binding rows read with the
   membership on every request (no row: no namespace; the capability switched off: no namespace, the rows kept);
   `Principal::authorize` refuses every namespace-scoped permission to a namespace-bound principal, and
   `Principal::authorize_in(namespace, permission)` checks the scope first (outside it, the opaque `404`, audited
   `run_recovery.namespace_denied` and counted) and the role second, so the `404` comes before
   `RECOVERY_ROLE_MAY_NOT_REDRIVE`. The namespace resolvers take the operation's permissions and end with
   `authorize_in`; a route that forgets them refuses the role. An admission or a command by a namespace-bound subject
   reads its binding row `FOR SHARE` in slot 4a after the memberships, so a revocation that commits first refuses it;
   an idempotent replay is checked again by the same resolvers. The binding is changed under `membership:manage` by
   `bindRunRecovery` and `unbindRunRecovery`, audited `run_recovery.bound` and `run_recovery.unbound`; a database
   trigger refuses the role on `memberships` until `failure-handler-v1` is active.
   *Revised 2026-11-26 after the second security re-read (decisions D022-286 to D022-296):* any change of a member's
   role removes its bindings (a trigger, as for reveal grants), and a bind reads the role under the membership's lock;
   the triggers' functions are `SECURITY DEFINER` with a pinned search path, executable by no role; the role is
   namespace-bound with an empty scope from the slice that adds it, so it can act nowhere before the scope is built, and
   the manifest of `failure-handler-v1` names the scope's artefacts; a pending reset of a bound subject completes through
   `authorize_in` and its in-transaction re-check reads the binding, failing `AUTHORITY_REVOKED` outside it; for a bound
   subject a miss and a denial cost the response the same; an open event stream is cut within its 30-second re-check.
   No run is granted its publisher's rights. *(Revised 2026-11-25, decision D022-244: "a machine identity" is not a
   principal kind of the platform, and an Operator or Developer credential could approve publications.)*
5. **Handler runs are a tree of their own.** A run carries `handler_tree`, set for a dispatched handler run, an
   `onFailure~h` child, every child of a handler-tree run, and every reset or rerun whose source is a handler-tree run
   or whose request's principal holds `RunRecovery` (what "whose initiator is a handler-tree run" means at an admission,
   decision D022-279); `handler_kind` (`DISPATCHED`, `REGION_CHILD`) says which kind of handler run it is. Both columns, and
   `lineage_run_id`, are guarded by the database (a `BEFORE UPDATE OF` trigger, `ENABLE ALWAYS`; a child's
   `handler_tree` checked against its parent's at insert). A handler-tree
   run MUST NOT be dispatched to the namespace handler. `handler_for_run_id` and its unique index apply to dispatched
   runs only, and a rerun or reset of a handler run inherits neither.
6. **No workflow handler without the start's grants.** A run that ends before it was ever granted its start's gates
   (queue timeout, key error at admission) MUST finish directly; only the namespace handler may follow.
7. **The namespace handler is an obligation admitted once per failure and per lineage.** A run qualifies when its status
   is in the configuration's statuses, the configuration is enabled, no workflow handler phase of the run ended
   `SUCCEEDED`, it is not a handler-tree run, not a test tree, and no operator asked to cancel it (a draining cause
   other than `CANCELLED` qualifies). The configuration (statuses, revision, enabled flag) is read by a left join in the
   statement that locks the run, `FOR UPDATE OF runs`, so the configuration row is never locked. The terminal commit of a qualifying run MUST
   insert one dispatch row keyed by `(tenant_id, namespace_id, source_run_id)`, recording the `config_revision` read and
   the failure context, and always does: the pending-dispatch ceiling refuses a new or re-enabled configuration and
   raises an alert, never a terminal commit. This insert is a recorded deviation of ADR-0050 decision 7 (a terminal
   transition writes a slot-3 row): the row is invisible until the commit, its key is the failed run, whose lock
   serializes every writer of it, and the dispatcher locks only committed rows `SKIP LOCKED`, so no cycle can pass
   through it. A namespace handler is dispatched at most once per lineage (the source chain of resets and reruns,
   `lineage_run_id`, which every reset and rerun extends whoever starts it): the dispatch row carries `lineage_first`,
   the unique index is `(tenant_id, namespace_id, lineage_run_id) WHERE lineage_first`, the terminal commit inserts its
   row with `lineage_first = true ... ON CONFLICT ... WHERE lineage_first DO NOTHING`, and only when that inserted
   nothing a second statement inserts the row as `SUPPRESSED_LINEAGE` (`lineage_first = false`), counted, journaled on
   the run, listed and redrivable. The two statements are followed in the run's commit only by the `run.finished` row of
   the run's own journal, then usage rows (decision D022-280), so the first
   one's wait for another run's uncommitted first row of the lineage is a leaf wait (the deviation extends to the
   lineage key). The `run.finished` journal row is appended after the two dispatch statements, its
   `namespace_dispatch` detail taken from what they did (a row of the run's own commit, so the leaf argument holds; the
   reducer ignores the field), and the terminal path updates no `run_events` row (decision D022-260). *(Revised
   2026-11-25, decision D022-242, which reverses D022-241 and answers O1176 (a).)*
8. **The dispatcher counts first, then admits, within a budget.** A shard-fenced pass checks the namespace dispatch
   budget (dispatches a namespace may admit per window, with a per-workflow share; beyond it the dispatch is deferred,
   never dropped, and an alert is raised) and, within it, counts the attempt (attempts, `next_attempt_at`) in its own
   committed transaction (the order of plan row T9, decision D022-285), then admits the handler run through `Engine::start` (initiator `system:engine`; its steps run under the
   handler version's publisher as a triggered run's do, D1177, with no operator right; provenance `handlerFor`) and marks the row
   `ADMITTED` in that transaction, locking the row inside `RunStore::create` through a `NewRun` field (the
   `pending_reset` precedent), in ADR-0050's slots: the dispatch row (slot 3), the source run's identity guard
   (`KEY SHARE`, slot 3R), the handler workflow's head read without a lock, the new run (slot 6), its gates (slot 7), the
   row's update (8a), usage rows (8b). After 12 counted attempts the row is `DEAD`, alerted and redrivable. A crash
   inside the admission still reaches `DEAD`. A deferral by the budget is not an attempt: it moves `next_attempt_at`
   only (decision D022-264). A dispatch whose handler definition continues as new under the handler tree fails with
   `last_error_code` `CONTINUE_AS_NEW_REFUSED` (decision D022-263).
9. **Every end of a run goes through the reducer.** The handler phase, the namespace dispatch and the early-result mark
   MUST be written by `persist` from the planner's events, a chain's finalization included (its terminal write goes
   through `persist`'s terminal path for the dispatch, so a failing chain head is dispatched); publication refuses
   `continueAsNew` with `onFailure` or `earlyReturn`, and a continue-as-new successor of a handler-tree run is refused
   at admission with a typed code (a child step under a handler tree fails `CONTINUE_AS_NEW_REFUSED`), the runtime check
   staying as a defence that ends the run through a typed finalization (decision D022-263); a reset or rerun under a
   handler tree (decision 5) of a definition that continues as new is refused at admission, `422 VALIDATION_FAILED`
   (`CONTINUE_AS_NEW_REFUSED`), and the defence ends a run that reaches the boundary anyway `FAILED` with the durable
   code `CONTINUE_AS_NEW_REFUSED`, as the continuation pass ends a chain with `CHAIN_LIMIT` (decision D022-279);
   the capabilities `failure-handler-v1` and `early-return-v1`
   are listed by a binary only once ADR-0146's routing and every artefact of their manifest are in it (decision
   D022-256). Until the routing
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
   ~~revision read with the definition~~ configuration read by a left join in the statement that locks the run (struck
   and replaced 2026-11-25, decision D022-270) gives the same answer, auditable, at no statement.
9. **A dispatch row written by a second transaction after the terminal commit** — rejected: a crash between the two
   loses the obligation.

## Consequences

- Good: a failed run's handler starts after compensation, sees a closed context and never changes the outcome; exactly
  one namespace handler run per failure and per lineage whatever the dispatcher crashes; an early result is disclosed
  while the run runs and survives its later failure.
- Bad: observers see a failed run as `RUNNING` with a pending status for the length of its handler phase; a handler step
  behind a busy key can time out before it runs (gap G022-027); every reset or rerun extends its lineage, so the
  failure of a rerun a person or an unrelated automation starts is not dispatched again but written as a
  `SUPPRESSED_LINEAGE` dispatch, shown in the listing and redrivable by an operator (O1176 answered (a), decision
  D022-242); a handler that starts fresh runs is bounded by the namespace dispatch budget.
- Harder: every future path that ends a run must go through the reducer (ADR-0146); every future creator copies
  `handler_tree` to its children.
- Concurrency of configuration: a change that commits while a failing run's transaction is in flight applies to that
  failure or not by what ~~its definition statement~~ the statement that locks the run (struck and replaced 2026-11-25,
  decision D022-270) read; the dispatch records which revision (gap G022-024).
- Compatibility and rollback: additive IR, events, columns and tables; activation through the registry. Once a journal
  holds the new events an older binary cannot decode the run: rollback is switching the capability off, never a binary
  downgrade; runs already in a phase finish it; the dispatcher keeps running until pending dispatches drain.

## Evidence

None yet: the tests named under **Enforced by** are built by tasks T022-030 to T022-038, T022-207, T022-208, T022-213 to
T022-215, T022-224, T022-225, T022-227 and T022-228, and since 2026-11-26 T022-243, T022-244 and T022-256 to T022-259. Spec: FR-022-008 to FR-022-016 and their amendments of 2026-11-24
and 2026-11-25; research: D1169 to D1177, D1195, D1197, D022-201, D022-207, D022-208, D022-210, D022-215, D022-216,
D022-222 to D022-224, D022-228, D022-237, D022-242 to D022-250, D022-255, D022-272 to D022-280; data model: the handler tables, the `runs` columns and invariants 3 to 5 and
16.
