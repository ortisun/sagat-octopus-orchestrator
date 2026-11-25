# ADR-0103: A code workflow is a decider whose commands are IR nodes held in a per-run overlay, committed in the run's one transaction

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation of the 0.3.0 design questions (OD-2026-11-20-6) and of the acceptance of design 044's architecture decisions (OD-2026-11-24-4); scope by the owner (OD-2026-11-24-3); no person has reviewed it
**Refines / supersedes:** refines ADR-0003 (declarative IR: "procedural worker with determinism rules later") and ADR-0001; reuses ADR-0010, ADR-0076 and the step review of feature 003
**Enforced by:** `every_node_lookup_of_a_code_run_resolves_through_the_overlay`; `the_workflow_task_step_is_neither_derived_nor_forwarded`; `a_code_run_never_finishes_by_routing`; the fault test `a_completion_commits_all_or_nothing_at_each_statement`; the differential test `the_batched_path_writes_what_persist_writes`; the budget test `the_completion_statements_stay_within_the_budget_for_256_commands`; `the_claim_statements_of_graph_steps_do_not_grow`; `every_command_kind_round_trips_through_its_code_profile`; `an_expired_workflow_task_is_counted_without_review`; `a_workflow_task_lease_is_refused_by_every_activity_rpc`; `an_activity_binding_named_workflow_order_gives_no_workflow_authority` (design 044, Tasks 044-4, 044-5, 044-8, 044-9)
**Required before acceptance:** the measurements of design 044 Task 044-25 and the fault tests of Tasks 044-9 and 044-28, as ADR-0050 requires of a decision that changes the execution path

## Context

Owner decision 2026-11-24-3 puts workflow as code in release 0.3.0; constitution 5.0.0 (owner decision 2026-11-24-2)
states the procedural model. The engine already executes every kind of durable work and commits each decision with its
journal, next work and outbox in one transaction (principle I, AGENTS rule 3). It reads every step's node from the
published definition (`def.node_for_instance`, in the claim, completion, failure, retry, review, secret resolution,
restore), and its claim routes a step to a session by the step's activity name, with an index, hints and a reconciler.
A code version has no graph. The constitution forbids tenant code in a platform process (principle V).

## Decision

1. A code workflow MUST be a decider: the function runs in the tenant's worker, in workflow tasks, and answers each with
   commands. The platform MUST NOT run workflow code in any of its processes.
2. A code run MUST have a per-run node overlay: the node spec of every accepted work command (at most 4 KiB) and its
   derived companions, stored when the command is accepted, through which every path that reads a step's node finds it.
   Every call site of `node_for_instance` MUST resolve through the overlay for a code run or be shown unreachable.
3. A workflow task MUST be a lease of the run's reserved workflow-task step, whose node identifier is a reserved form
   recognised by one predicate (`is_workflow_task_step`) tested before `is_derived` and `forward`, and whose activity
   field holds `sagat.workflow:<type>:<digest>` (or, while sticky, `sagat.workflow-sticky:<session>`), so that the
   existing claim routes it without a change to its SQL. Only a session that registered that type and digest under a
   binding whose `workflowTypes` lists the type MUST receive it; an activity binding MUST NOT cover a workflow type; an
   activity registration or binding that names `sagat.workflow` MUST be refused.
4. A workflow task MUST carry no secret, MUST have no heartbeat, MUST be refused by every activity RPC, and its expiry MUST
   be counted as a workflow task failure, never sent to reconciliation or review, because it has no external effect.
5. Every command that creates work MUST carry the canonical JSON of a fragment of the code profile of an existing node
   kind, parsed by the publication parser of that kind in its code-profile mode, and executed by the engine's existing
   command functions. There MUST be no second execution engine.
6. A completion MUST commit, in one transaction with the run row and the workflow-task step locked: the journal events
   of the completion and of every command, the overlay rows, the steps, timers and waits, the projections, the outbox
   hints and the state of the workflow-task step. It MUST run at most the statements of today's activity completion plus
   4, reads included, whatever the number of its commands (at most 256); the batched persistence path MUST write what
   `persist` writes.
7. The engine MUST NOT end a code run by routing; a terminal command MUST go through the engine's stop drain.
8. A run whose function needs a person MUST go under the existing step review of feature 003 on its workflow-task step;
   skipping or resolving that step MUST be refused.

## Considered options

1. **Decider with IR-node commands, a per-run overlay, on the existing engine and claim** — one substrate, every guarantee
   of the graph mode for code runs, the Studio draws code runs with the node pictures.
2. **A separate engine for code workflows (Temporal's history and matching services)** — lost: duplicates leases,
   retries, compensation and feature 037, and a decision would span two engines' transactions.
3. **Replay on the server in a sandbox** — lost: principle V, and a sandbox per language on the server.
4. **Synthetic nodes written into the published version** — lost: a version would no longer be immutable.
5. **A separate claim path and table for workflow tasks** — lost: a second copy of the routing, hints and reconciler.
6. **A new hold state with its own route and events** — lost: the step review already holds a step for a person.

## Consequences

- Good: compensation, uncertain effects, gates, step logs, classification and blobs work for code runs without a second
  engine; the completion is one fenced transaction; graph steps' claim is unchanged.
- Bad: the planner gains a code arm, the store an overlay read per run transaction (bounded by the 2,048 outstanding
  steps), `PgRunTx` a batched path (new code with its own crash and differential tests); the commands are limited to node
  kinds whose code profile is defined.
- Compatibility: additive; graph runs are untouched. Rollback is refused by the migration's down while a code run or code
  version exists.

## Evidence

Design 044 (`docs/designs/044-workflow-as-code.md`, Technical Plan 2, 4, 5 and 6.3); `crates/application/src/worker.rs`
(the claim under the run lock, `declared_secrets_of`), `crates/domain/src/execution/command.rs`,
`crates/domain/src/workflow.rs` (`NoNodes`, `parse_stored`, `is_derived`), `crates/infra-postgres/src/execution.rs`; the
review reports of rounds 1 and 2 (`docs/designs/044-workflow-as-code.reviews/`); Decisions 044-041, 044-070, 044-071 and
044-077 in `specs/044-workflow-as-code/research.md`.
