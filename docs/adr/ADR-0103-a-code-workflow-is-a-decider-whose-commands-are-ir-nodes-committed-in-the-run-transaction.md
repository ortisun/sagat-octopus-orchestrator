# ADR-0103: A code workflow is a decider whose commands are IR nodes, committed in the run's one transaction

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation of the 0.3.0 design questions (OD-2026-11-20-6) and of the acceptance of design 044's ADRs (OD-2026-11-24-4); scope by the owner (OD-2026-11-24-3)
**Refines / supersedes:** refines ADR-0003 (declarative IR: "procedural worker with determinism rules later") and ADR-0001; reuses ADR-0010 and ADR-0076
**Enforced by:** the domain unit test `a_code_run_never_finishes_by_routing`; the fault test `a_completion_commits_all_or_nothing_at_each_statement`; the budget test `the_completion_runs_at_most_12_statements_for_256_commands`; `every_command_kind_round_trips_through_the_node_parser`; `an_expired_workflow_task_is_released_with_the_next_generation_without_review`; `a_workflow_task_lease_cannot_resolve_secrets` (design 044, Tasks 044-4, 044-5, 044-8, 044-9)

## Context

Owner decision 2026-11-24-3 puts workflow as code in release 0.3.0: a workflow function in an SDK, replayed
deterministically against the journal. The engine already executes every kind of durable work (activities, scripts,
timers, waits, children, approvals, sagas with derived compensation steps, the uncertain-effect rules of feature 037,
the gates of feature 022) and commits each decision with its journal, next work and outbox in one transaction
(principle I, AGENTS rule 3). The constitution forbids tenant code in a platform process (principle V). Temporal runs a
separate history and matching service and treats workflow tasks as a protocol of their own.

## Decision

1. A code workflow MUST be a decider: the function runs in the tenant's worker, in workflow tasks, and answers each
   with commands. The platform MUST NOT run workflow code in any of its processes.
2. A workflow task MUST be a lease of the run's reserved step `~workflow` (reserved activity name `sagat.workflow`, task
   kind `WORKFLOW`), fenced by attempt, generation and lease token like any step. It MUST carry no secret, MUST have no
   heartbeat, and its expiry MUST re-lease it with generation + 1 without reconciliation or review, because it has no
   external effect.
3. Every command that creates work MUST carry the canonical JSON of an IR node fragment of an existing node kind, parsed
   and checked by the publication parser of that kind, and MUST be executed by the engine's existing command functions.
   There MUST be no second execution engine.
4. A completion MUST commit, in one transaction with the run row locked: the journal events of the completion and of
   every command, the steps, timers and waits, the projections, the outbox hints and the scheduling of the next workflow
   task, in at most 12 statements whatever the number of its commands (at most 256).
5. Every event a code run's function must see MUST, in its own transaction, mark a workflow task pending and make the
   `~workflow` step `READY` with its hint when it was waiting.
6. The engine MUST NOT end a code run by routing; only a command, a deadline, a forced cancellation after the grace, a
   termination or the history limit ends or holds it.

## Considered options

1. **Decider with IR-node commands on the existing engine** — one substrate, every guarantee of the graph mode for free,
   the Studio draws code runs with the node pictures.
2. **A separate engine for code workflows (Temporal's history and matching services)** — lost: duplicates leases,
   retries, compensation and feature 037, and a decision would span two engines' transactions.
3. **Replay on the server in a sandbox** — lost: principle V, and a sandbox per language on the server.
4. **Commands as their own vocabulary** — lost: every node check and limit would exist twice and drift.

## Consequences

- Good: compensation, uncertain effects, gates, step logs, classification and blobs work for code runs without new
  execution code; the completion is one fenced transaction.
- Bad: the planner gains a code arm, and the `steps` table a reserved row per code run; the command vocabulary is limited
  to node kinds whose semantics are defined outside a graph (database, lake and structured nodes are not commands in
  0.3.0).
- Compatibility: additive; graph runs are untouched. Rollback with code runs open is refused by the migration's down.

## Evidence

Design 044 (`docs/designs/044-workflow-as-code.md`, Technical Plan 2, 4 and 5); the transaction of `PgRunTx::persist`
(`crates/infra-postgres/src/execution.rs`); the determinism spikes of Decision 044-041 in
`specs/044-workflow-as-code/research.md`.
