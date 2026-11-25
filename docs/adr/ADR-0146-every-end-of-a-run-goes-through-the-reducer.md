# ADR-0146: Every end of a run goes through the reducer: the collection and loop finish paths are routed through the planner and `persist`

**Status:** Proposed (2026-11-25)
**Decided by:** agent proposal (feature 022, decisions D022-207 and D022-232, 2026-11-24 and 2026-11-25) under the owner's delegation (OD-2026-11-24-4, "Tomem a melhor decisão"), after the reviews of 2026-11-25 (`specs/022-flow-control-step-policies/reviews/2026-11-25/`, correctness Major 8, architecture Major 5 and minor 15); accepted by the coordinating session once the three reviewers find no Blocker in a re-review (task T022-200)
**Refines / supersedes:** refines ADR-0030 (structured instances share the reducer; this ADR extends that sharing to the moment a structured scope finishes the run) and ADR-0094 decision 4 (the finish paths answer the rounds instead of refusing a gated successor; amended 2026-11-24). Supersedes the run-time refusals of feature 022 for a gated successor of a finish path (`RuntimeNotIntegrated`) and for a start gate with a collection or loop (D022-049).
**Enforced by:** a rule of `scripts/check-architecture.py` (task T022-226) that refuses, outside `persist` in `crates/infra-postgres/src/execution.rs`, a `run.finished` journal write or outbox insert, a `finished_at =` assignment and a direct `task.ready` outbox insert, with one allow-listed exception in `crates/infra-postgres/src/chain_progress.rs` that cites the `continueAsNew` refusals; the tests of task T022-207 (one per finish path and successor kind, a crash at the finish commit, a loop and a collection that end `FAILED` with `onFailure` and with a namespace handler); `verify_run`.

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
   progress) keep their transaction and their order: chain → run → the scope's rows → the active-run counter row when
   the commit ends the run → the rounds → `persist` → usage rows.
2. **Only `persist` ends a run or announces a task.** Outside `persist`, no code MAY journal or outbox `run.finished`,
   assign `finished_at`, or insert a `task.ready` outbox row. The one exception is the chain finish of
   `chain_progress.rs`: a continuation chain carries no feature 022 policy, because `concurrency`, `rateLimit`,
   `earlyReturn` and `onFailure` are refused with `continueAsNew` (`FLOW_CONTROL_COMBINATION`), and `cache` acts at a
   step's readiness and completion, never at a run's end; the allow-list names this reason.
3. **The interim refusals go with the routing.** Until the routing lands, publication refuses `cache`, `earlyReturn` and
   `onFailure` in a definition with a `forEach`, `while` or `repeatUntil` node (`UNSUPPORTED_NODE`, dependency
   `COLLECTION_FINISH_ROUTING`); the change that routes the paths removes this refusal, `RuntimeNotIntegrated` and the
   start-gate refusal of D022-049, with their tests.
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

None yet: task T022-207 (the routing and its tests), task T022-226 (the fitness rule). Research: D022-207, D022-232,
G1197; feature 014's plan, "Transaction and scheduling design"; ADR-0030.
