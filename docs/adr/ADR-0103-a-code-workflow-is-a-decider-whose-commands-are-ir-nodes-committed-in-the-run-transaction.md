# ADR-0103: A code workflow is a decider whose commands are IR nodes, committed in the run's one transaction

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation of the 0.3.0 design questions (OD-2026-11-20-6) and of the acceptance of design 044's architecture decisions (OD-2026-11-24-4); scope by the owner (OD-2026-11-24-3)
**Refines / supersedes:** refines ADR-0003 (declarative IR: "procedural worker with determinism rules later") and ADR-0001; reuses ADR-0010, ADR-0076 and the step review of feature 003
**Enforced by:** the domain unit test `a_code_run_never_finishes_by_routing`; the fault test `a_completion_commits_all_or_nothing_at_each_statement`; the budget test `the_completion_runs_at_most_10_statements_for_256_commands`; `every_command_kind_round_trips_through_its_code_profile`; `an_expired_workflow_task_is_released_with_the_next_generation_without_review`; `a_workflow_task_lease_is_refused_by_every_activity_rpc`; `a_star_binding_does_not_cover_a_workflow_type` (design 044, Tasks 044-4, 044-5, 044-8, 044-9)

## Context

Owner decision 2026-11-24-3 puts workflow as code in release 0.3.0: a workflow function in an SDK, replayed
deterministically against the journal. The engine already executes every kind of durable work (activities, scripts,
timers, waits, children, approvals, sagas with derived compensation steps, the uncertain-effect rules of feature 037,
the gates of feature 022) and commits each decision with its journal, next work and outbox in one transaction
(principle I, AGENTS rule 3). Its claim routes a step to a session by the step's activity name, with an index, hints and
a reconciler. The constitution forbids tenant code in a platform process (principle V). Temporal runs separate history
and matching services and treats workflow tasks as a protocol of their own.

## Decision

1. A code workflow MUST be a decider: the function runs in the tenant's worker, in workflow tasks, and answers each with
   commands. The platform MUST NOT run workflow code in any of its processes.
2. A workflow task MUST be a lease of the run's reserved step `~workflow`, whose activity field holds the reserved name
   `sagat.workflow:<type>:<digest>` of the run's current code version, claimed by the existing claim, fenced by attempt,
   generation and lease token like any step. Only a session that registered that type and digest under the exact
   binding `workflow:<type>` MUST receive it; `*` and prefix bindings MUST NOT cover it; an activity registration or a
   binding that names `sagat.workflow` MUST be refused.
3. A workflow task MUST carry no secret, MUST have no heartbeat, MUST be refused by every activity RPC, and its expiry
   MUST re-lease it with the next generation without reconciliation or review, because it has no external effect.
4. Every command that creates work MUST carry the canonical JSON of a fragment of the *code profile* of an existing node
   kind (expressions and templates replaced by literals or refused, publisher-only fields refused), parsed by the
   publication parser of that kind in its code-profile mode, stored immutably, and executed by the engine's existing
   command functions. There MUST be no second execution engine.
5. A completion MUST commit, in one transaction with the run row and the `~workflow` step locked: the journal events of
   the completion and of every command, the steps, timers and waits, the projections, the outbox hints and the scheduling
   of the next workflow task. An ordinary completion MUST run at most 10 statements whatever the number of its commands
   (at most 256); a terminal or continue-as-new completion MUST add exactly the existing terminal or handoff path.
6. Every event a code run's function must see MUST, in its own transaction, mark a workflow task pending and make the
   `~workflow` step `READY` with its hint when it was waiting.
7. The engine MUST NOT end a code run by routing; only a command, a deadline, a forced cancellation after the grace or a
   termination ends it.
8. A run whose function needs a person MUST go under the existing step review of feature 003 on its `~workflow` step;
   skipping or resolving that step MUST be refused.

## Considered options

1. **Decider with IR-node commands on the existing engine and claim** — one substrate, every guarantee of the graph mode
   for code runs, the Studio draws code runs with the node pictures.
2. **A separate engine for code workflows (Temporal's history and matching services)** — lost: duplicates leases,
   retries, compensation and feature 037, and a decision would span two engines' transactions.
3. **Replay on the server in a sandbox** — lost: principle V, and a sandbox per language on the server.
4. **Commands as their own vocabulary** — lost: every node check and limit would exist twice and drift.
5. **A separate claim path and table for workflow tasks** — lost: a second copy of the routing, hints and reconciler.
6. **A new hold state with its own route and events** — lost: the step review already holds a step for a person, with a
   retry, permissions and audit.

## Consequences

- Good: compensation, uncertain effects, gates, step logs, classification and blobs work for code runs without a second
  engine; the completion is one fenced transaction.
- Bad: the planner gains a code arm, `PgRunTx` a batched persistence path (new code with its own crash tests), and the
  `steps` table a reserved row per code run; the commands are limited to node kinds whose code profile is defined
  (database, lake and structured nodes are not commands in 0.3.0).
- Compatibility: additive; graph runs are untouched. Rollback with code runs open is refused by the migration's down.

## Evidence

Design 044 (`docs/designs/044-workflow-as-code.md`, Technical Plan 2, 4, 5 and 6.3); the transaction of `PgRunTx::persist`
and the claim by activity (`crates/infra-postgres/src/execution.rs`); the review findings of round 1
(`docs/designs/044-workflow-as-code.reviews/2026-11-24/round-1/`); the spikes of Decision 044-041 in
`specs/044-workflow-as-code/research.md`.
