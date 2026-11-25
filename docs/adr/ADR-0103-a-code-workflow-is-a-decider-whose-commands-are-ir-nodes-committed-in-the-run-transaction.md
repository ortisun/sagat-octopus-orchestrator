# ADR-0103: A code workflow is a decider whose commands are IR nodes held in a per-run overlay, committed in the run's one transaction

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation of the 0.3.0 design questions (OD-2026-11-20-6) and of the acceptance of design 044's architecture decisions (OD-2026-11-24-4); scope by the owner (OD-2026-11-24-3); no person has reviewed it
**Refines / supersedes:** refines ADR-0003 (declarative IR: "procedural worker with determinism rules later") and ADR-0001; reuses ADR-0010, ADR-0076 and the step review of feature 003
**Enforced by:** `integration_workflow_as_code::nodes::every_command_kind_runs_through_every_path`; `the_workflow_task_step_is_neither_derived_nor_forwarded`; `a_code_run_never_finishes_by_routing`; the crash tests of `fault_workflow_as_code` (between the statements of the batched path, after the commit); the differential test `the_batched_path_writes_what_persist_writes`; the budget test `a_completion_of_256_commands_of_every_kind_stays_within_62_statements`; `code_definition_sites::every_definition_loader_calls_the_composer`; `code_workflow_sites::only_the_cancellation_function_calls_stop_run_with_cancelled`; `a_deferred_continue_as_new_after_a_cancellation_request_ends_cancelled`; `cancelling_a_run_under_review_stops_it_without_the_grace_and_applies_the_cancel_policies`; `code_definition_sites::every_call_site_is_listed`; `the_claim_statements_of_graph_steps_do_not_grow`; `every_command_kind_round_trips_through_its_code_profile`; `an_expired_workflow_task_is_counted_without_review`; `workflow_task_expiry_parity`; `a_workflow_task_lease_is_refused_by_every_activity_rpc`; `an_activity_binding_named_workflow_order_gives_no_workflow_authority` (design 044, Tasks 044-4, 044-5, 044-8, 044-9)
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
2. A code version MUST parse to a definition whose graph is the code variant, and a run transaction of a code run MUST
   compose it with the run's node overlay (the node spec of every accepted work command, at most 1 KiB, and its derived
   companions, frozen at acceptance), so that every method of `WorkflowDefinition`, the saga and scope accessors
   included, answers through it or from the code version's document. A source-text check, with no new dependency,
   hosted where pull-request CI runs it whenever a scanned crate changes (a test of `apps/execution-engine`), MUST count
   the calls of the distinctive method names of `WorkflowDefinition` in the production code of `crates/*/src` and
   `apps/*/src` against a checked-in allowlist of `(file, method, count)`, and MUST hold that every loader of a
   definition (the callers of `DefinitionCache::get_parsed`, `get_or_parse` and `get_or_parse_private`) either composes
   the overlay or is stated to need only the version's definition, so that a new call site or loader fails until it is reviewed; an arm that would let an
   unknown node through MUST fail closed.
3. A workflow task MUST be a lease of the run's reserved workflow-task step, of `steps.kind` `WORKFLOW_TASK` and
   `NodeKind::WorkflowTask` (built from the step row and the pin), whose reserved node identifier `is_derived`, `forward`
   and `parse_stored` handle themselves, and whose activity field holds `sagat.workflow:<type>:<hex digest>`, so that the
   candidate scan finds it without a change to its SQL; the claim MUST have a workflow-task arm that checks the digest
   and the pause and skips the activity-only rules (quotas, breakers, pools, capacity windows, the run-document input),
   counting the task in the namespace quota bucket `sagat.workflow`. Only a session that registered that type and digest under a
   binding whose `workflowTypes` lists the type MUST receive it; an activity binding MUST NOT cover a workflow type; an
   activity registration or binding pattern that starts with `sagat.workflow` MUST be refused, graph publication MUST
   refuse an activity name that starts with `sagat.` (except feature 017's managed names), and the claim and the
   completion MUST refuse a reserved workflow name on a step whose kind is not `WORKFLOW_TASK`.
4. A workflow task MUST carry no secret, MUST have no heartbeat, MUST be refused by every activity RPC, and its expiry MUST
   be counted as a workflow task failure, never sent to reconciliation or review, because it has no external effect.
5. Every command that creates work MUST carry the canonical JSON of a fragment of the code profile of an existing node
   kind, parsed by the publication parser of that kind in its code-profile mode, and executed by the engine's existing
   command functions. There MUST be no second execution engine.
6. A completion MUST commit, in one transaction with the run row and the workflow-task step locked: the journal events
   of the completion and of every command, the overlay rows, the steps, timers and waits, the projections, the outbox
   hints and the state of the workflow-task step. It MUST run at most the 50 statements of today's `COMPLETION_WITH_A_NEXT_STEP` plus 12,
   reads included, whatever the number of its commands (at most 256), that is at most 62; the count of the steps that
   can still act MUST be read, never kept by hand; the batched persistence path MUST write what
   `persist` writes. Every journal write of every run MUST go through one typed append function.
7. The engine MUST NOT end a code run by routing; a terminal command MUST go through the engine's stop drain.
8. A run whose function needs a person MUST go under the existing step review of feature 003 on its workflow-task step;
   skipping or resolving that step MUST be refused. The platform has no terminate (ADR-0050 decision 5): the remedy that
   ends such a run is its cancellation, which stops a run under review at once.
9. Cancelling a code run MUST be cooperative: every path that cancels a run (the route, a chain-addressed cancel, a
   parent's `propagate` policy, a pending reset's source, the function's cancel of its own child) MUST go through one
   domain function, which for a code run journals `run.cancel_requested` once with the version's grace and sets the
   deadline, without stopping the run; a pass MUST stop the run at the deadline, judged by the database clock; the
   deadline MUST be cleared when a terminal command is accepted or a stop begins; a deferred terminal command is not an
   answer; a continue-as-new accepted after the request MUST end the run `CANCELLED` with no handoff; a request to a run
   that already has a stop reason MUST journal nothing.

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
