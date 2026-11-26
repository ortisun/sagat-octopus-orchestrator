# ADR-0146: Every end of a run goes through the reducer: the collection and loop finish paths are routed through the planner and `persist`

**Status:** Accepted (2026-11-25, delegate OD-2026-11-24-4, after three review rounds recorded in specs/022-flow-control-step-policies/reviews/2026-11-25/); revised after the re-reviews of `6d02632e` and the third reads of `c910345b` (decisions D022-249 to D022-251, D022-267, D022-268)
**Decided by:** agent proposal (feature 022, decisions D022-207 and D022-232, 2026-11-24 and 2026-11-25) under the owner's delegation (OD-2026-11-24-4, "Tomem a melhor decisão"), after the reviews of 2026-11-25 (`specs/022-flow-control-step-policies/reviews/2026-11-25/`, correctness Major 8, architecture Major 5 and minor 15); accepted by the coordinating session once the three reviewers find no Blocker in a re-review (task T022-200)
**Refines / supersedes:** refines ADR-0030 (structured instances share the reducer; this ADR extends that sharing to the moment a structured scope finishes the run) and ADR-0094 decision 4 (the finish paths answer the rounds instead of refusing a gated successor; amended 2026-11-24). Supersedes the run-time refusals of feature 022 for a gated successor of a finish path (`RuntimeNotIntegrated`) and for a start gate with a collection or loop (D022-049).
**Enforced by:** a rule of `scripts/check-architecture.py` (task T022-226), judged by enclosing function (`enclosing_function`, as `run_finished_errors` does), that refuses a `run.finished` journal write or outbox insert, an `UPDATE runs` that sets `finished_at` and a direct `task.ready` outbox insert outside the allow-listed functions (decision 2), with negative fixtures; a mixed-version test (an older binary replays a journal the routed finish wrote) and a held-lock test against concurrent feature 014 iteration transactions; the tests of task T022-207 (one per finish path and successor kind, a crash at the finish commit, a loop and a collection that end `FAILED` with `onFailure` and with a namespace handler); `verify_run`.

## Context

Feature 014 (`specs/014-workflow-primitives/plan.md`, "Transaction and scheduling design") gave collections and loops
their own transactions on exact instance paths, and ADR-0030 made structured instances share the reducer for the steps
inside them. When such a scope finishes, though, several paths write the run's end themselves, without the planner or
`persist`:

- `crates/infra-postgres/src/primitives.rs` near line 3495 (the collection finish journals `collection.finished`,
  `run.finished` and updates `runs` itself), near lines 4026 to 4045 (a direct `task.ready` outbox insert and a
  `run.finished`) and near line 4061;
- `crates/infra-postgres/src/loop_progress.rs` near line 778 (the loop finish journals `end.reached` and `run.finished`)
  and near lines 3409 to 3449 (a `run.finished` and a `finished_at` update);
- `crates/infra-postgres/src/chain_progress.rs` lines 2246 to 2248 (a chain's `run.finished` and `chain.finished` outbox).

Feature 022 adds what a run's end must do: release the start's permit, journal a gated or cacheable successor's
decision, enter a failure handler phase instead of finishing, insert a namespace handler dispatch row, mark an early
result already returned. A path that ends a run without the reducer skips all of it. Until now the feature refused at
run time (a gated successor `RuntimeNotIntegrated`, retried for ever; a start gate with a collection
`BINARY_CAPABILITY_MISSING`, D022-049) — a run failing for a rule publication could have checked (gap G1197).

## Decision

1. **The finish paths are routed.** When a collection or a loop finishes, its transaction MUST hand the run's state to
   the planner and write through `persist`: the successor's release and its lookup and gate rounds, and the run's end
   (`run.finished`, the `runs` update, the run-finished deliveries, the start's releases, the handler phase, the dispatch
   row), are decided and written exactly as for any other commit. The scope's own rows (014's collection and loop
   progress) keep their transaction and their order: chain → run → the scope's rows → the rounds → `persist` (the
   active-run counter row when the commit ends the run, the give-backs, the rest) → usage rows, as row T31 of feature
   022's plan says (revised 2026-11-25, decision D022-268). The finish-path rule lands in the routing slice, its pull
   request's first commit showing it red, with a test that a chain finalization goes through `persist`'s terminal path
   (decision D022-267).
2. **Only `persist` ends a run or announces a task.** Outside the `persist` family (`persist`, `persist_addressed`,
   `persist_scoped_addressed`, `outbox_task_ready*`, `outbox_run_finished`), no function MAY journal or outbox
   `run.finished`, set `finished_at` in an `UPDATE runs`, or insert a `task.ready` outbox row. Allow-listed with their
   reasons: the announcements of a body iteration (014's exact-path transactions announce the next instance of a body,
   not a root step, and no 022 policy applies to a body node); and the chain hand-off and finalization of
   `chain_progress.rs`, for what they write besides the dispatch: `continueAsNew` is refused with every declared 022
   policy (`concurrency`, `rateLimit`, `cache`, `onFailure`, `earlyReturn`) and for a successor of a test-tree or
   handler-tree run (decision D022-249). The namespace handler is not a declaration, so it is no reason: a chain's
   finalization writes its terminal status through `persist`'s terminal path for the namespace dispatch, and a failing
   chain head is dispatched like any run (decision D022-250). *(Revised 2026-11-25: the first text could not pass, since
   `persist`'s own helpers write the statements it refused, and its chain reason did not cover the dispatch.)*
3. **The interim refusals go with the routing.** Until the routing lands, publication refuses `cache`, `earlyReturn` and
   `onFailure` in a definition with a `forEach`, `while` or `repeatUntil` node (`UNSUPPORTED_NODE`, dependency
   `COLLECTION_FINISH_ROUTING`); the change that routes the paths removes `RuntimeNotIntegrated` and the start-gate
   refusal of D022-049, with their tests. *Revised 2026-11-25 (decision D022-258):* the publication refusal is kept as a
   defence until the routing's tests and the fitness rule have run in a release, and is then removed by its own change.
4. **The capabilities wait for it.** No binary lists `failure-handler-v1`, `early-return-v1` or `step-cache-v1` before
   this routing is in it (decision D022-236).

## Considered options

1. **Route every finish through the planner and `persist`, with a fitness rule** — chosen. One terminal logic, one
   place a new rule of a run's end is written, and a check that keeps the next path from ending a run on its own.
2. **Teach each finish path the new rules** — rejected: three copies of the terminal logic that drift, the "guard each
   caller" shape ADR-0089 rejected.
3. **Keep the run-time refusals** — rejected: a run fails at run time, for ever retried, for a rule publication could
   have checked; and the feature's stories would be unusable with collections and loops.
4. **Refuse collections and loops with every 022 policy, for good** — rejected: collections and loops are the
   platform's fan-out, and a failure handler that ignores them would miss the runs most likely to fail.

## Consequences

- Good: a run's end has one implementation; gated, cacheable and early-return successors of a scope work; a collection
  or loop that fails enters its handler phase and is dispatched like any run.
- Bad: the finish transactions of collections and loops gain the planner's work and `persist`'s statements; their budget
  lines are recorded against today's by the routing change (plan, "Hot-path budget lines to record").
- Harder: every future structured scope must end the run through `persist`; the fitness rule refuses otherwise.
- Compatibility: no stored state changes; the journal of a run that finishes through a collection is the same events
  in the same order, plus whatever the run's policies add. Runs in flight finish through the new path after an upgrade.

## Evidence

None yet: task T022-207 (the routing and its tests), task T022-226 (the fitness rule), task T022-247 (the mixed-version
and held-lock tests). Research: D022-207, D022-232,
G1197; feature 014's plan, "Transaction and scheduling design"; ADR-0030.
