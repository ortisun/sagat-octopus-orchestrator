# ADR-0107: Effects from code are leased steps in a code profile whose risk metadata the publisher owns, and a saga opened from code is planned by the engine

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation (OD-2026-11-20-6 for Owner questions 044-007 and 044-010; OD-2026-11-24-4 for the acceptance); no person has reviewed it
**Refines / supersedes:** applies ADR-0004 and ADR-0076 to code workflows; supersedes ADR-0010 decision 3 (a saga's plan is a pure function of the published definition) for code runs only
**Enforced by:** `an_activity_command_cannot_carry_a_secret_of_another_activity_entry`; `a_compensation_receives_the_secrets_of_its_own_entry`; `a_fragment_that_changes_irreversible_is_refused`; `a_fragment_with_an_expression_field_is_refused`; `a_fragment_with_destinations_is_refused`; `a_fragment_with_an_output_schema_is_refused`; `an_approval_with_an_inline_policy_is_refused`; `an_envelope_activity_with_a_reserved_prefix_is_refused`; `a_compensate_with_that_is_not_an_entry_is_refused`; `a_sequential_saga_compensates_in_reverse_dependency_order`; `independent_code_compensations_run_while_one_is_under_review`; `an_unknown_outcome_in_a_code_saga_waits_for_review`; `a_terminal_command_with_an_open_saga_scope_reviews_as_command_invalid`; `a_terminal_command_while_a_scope_aborts_is_deferred`; `every_kind_of_initiator_is_excluded_from_an_engine_held_approval`; `a_gated_activity_with_a_blob_input_is_command_invalid`; `an_update_result_over_the_bound_fails_the_update_only`; `a_terminal_output_at_the_retained_bytes_edge_ends_the_run`; `a_terminal_command_waits_for_a_leased_activity_before_run_finished`; `a_code_step_output_beyond_the_retained_bytes_is_an_unknown_outcome`; `eager_activity_leases_are_provisional_and_granted_in_the_completion_transaction`; `an_unacknowledged_eager_lease_expires_without_an_unknown_outcome`; `an_idempotent_replay_returns_no_lease`; the conformance scenarios `code-saga` and `code-saga-uncertain` (design 044, Tasks 044-3, 044-5, 044-14, 044-22, 044-26)
**Required before acceptance:** the fault tests of design 044 Task 044-22 and the measurements of Task 044-25

## Context

Principle III: effects are at-least-once, an unknown outcome is consulted or reviewed before it is repeated, absent or
false risk metadata must not authorize a repeat, compensation is a step of its own, and independent compensations remain
eligible. Principle V: an ordinary task receives only the secrets declared by its immutable task definition. A graph
node's fields are written by the publisher; some are expressions the server evaluates, some decide what a reader may see
or where data goes, some are risk metadata. A graph saga's plan is computed at publication (ADR-0010 decision 3), which a
code saga cannot have. Temporal offers local activities and leaves compensation to hand-written code.

## Decision

1. The function MUST NOT run an effect through the platform except as a leased step (activity, script, child, approval)
   created by a command. There MUST be no local activity.
2. A work command MUST carry a fragment of the code profile of its node kind: expression and template fields MUST be
   literals or refused; `destinations`, an inline approval policy and `onError` MUST be refused; `secrets`,
   `irreversible`, `highRisk`, `compensateWith`, `reconcileWith`, `outputSchema`, `requiresApproval` and feature 022's
   `concurrency` and `rateLimit` policies MUST come from the published envelope entry, which a fragment may repeat or
   omit but never change; a fragment MAY carry only the literal keys of those policies, which are business literals
   (classified `pii`), while the node spec and the journal carry only feature 022's keyed digest.
3. A command MUST stay within the current code version's envelope: the activity entry that covers its name (an exact
   entry wins over a prefix, a longer prefix over a shorter one; only an exact entry MAY list secrets), the names of its
   `compensateWith` and `reconcileWith` entries too; an activity whose entry has `requiresApproval` MUST be held by the
   engine behind an approval of that very step under the named policy; the initiators that a policy with
   `excludeInitiator` excludes, for that approval and for an approval command alike, MUST be kept per run (the starter,
   the actor of every event of origin `external`, every mover, review retrier, review resolver and resetter, and the
   publisher and the activator of each code version the run is pinned to), copied to a successor, a child and a reset run with a sequence
   of 0 so that they always bind, written for a child of a graph parent from the parent's starter, chain initiator and
   publisher, written only in trees that contain a code run, checked by every approval of a run that has them (a graph
   child's approval node included), and otherwise bound by the issuing activation's start; the binder of a session's
   binding MUST be written as an initiator of the run when the session leases its workflow task, since a workflow-type
   binding grants authorship of the runs, so the copy rules carry the binder to children, successors and resets; a derived step MUST receive the secrets of its own entry; a
   script entry by digest with its own secrets, schemas and compensation; a listed child
   workflow at its listed version; a listed event type within its look-back; a named approval policy. No activity name MAY start with
   `script.` or `sagat.`. An activity input MUST be validated and classified by its entry's `inputSchema`.
4. A successful report whose output cannot be retained MUST be an unknown outcome (`STORAGE_QUOTA_EXCEEDED`), reconciled
   or reviewed under feature 037; the function MUST NOT be told the step failed. A completion whose payloads exceed the
   run's 4 MiB MUST put the run under review (`RETAINED_BYTES_LIMIT`); one refused by the tenant's content quota MUST be a
   counted failure; the engine MUST NOT end a code run for either. A terminal output and a continue-as-new input and
   carried state leave the run and MUST be judged against the tenant's content quota only; an update result over the
   run's bound MUST fail that update only. A gated activity's input and an approval's subject MUST travel inline. A
   command-value upload MUST be charged to the tenant's content quota only, the run's bound being judged at commit. A `RETAINED_BYTES_LIMIT` review has no retry, because
   the same activation would make the same commands.
5. Eager activity leases MUST be provisional leases of feature 006, granted through the claim's admission function
   (quotas, breakers, pools, capacity windows, gates, approvals), released safely when unacknowledged; an idempotent
   replay MUST NOT return a lease, and the idempotency record MUST NOT store a lease token.
6. A saga opened from code MUST be an engine compensation domain: closing a top-level scope drops the compensations,
   while a closed scope nested in an open parent stays compensable by it; aborting it MUST plan
   the compensations of its completed steps in the run-scoped tables of feature 003 (`run_sagas`, `run_saga_entries`),
   with step B depending on step A when A's outcome is at or before the start of the workflow task whose completion
   recorded B, so that independent
   compensations stay eligible together; compensation identifiers MUST be derived from the start version and the forward
   node; they MUST run with the rules of features 003 and 037; the function MUST learn only the scope's outcome; an engine-driven stop MUST apply each open
   scope's `cancelPolicy`, carried by the open-saga command. Each code scope MUST have a step row `s-<n>` waiting on the
   saga, never offered to a worker, so that feature 003's saga machinery drives it and a stop never winds down its
   compensations. An abort command MUST only journal `saga.aborting`; the plan MUST be written once every member has
   settled, the scope's `cancelPolicy` applied to members still in flight. A scope MAY be nested in an open parent,
   recorded by the open-saga command and the overlay: aborting the parent MUST abort its open children first, and
   closing a parent with an open child MUST be `COMMAND_INVALID`. The dependency source of `plan_entries` MUST have a
   code arm reading each command's issuing `S(k)` and each step's stored outcome sequence, and MUST store the linear
   pairs `(issued_seq, outcome_seq)`, never a quadratic list. Before a terminal or continue-as-new command the SDK MUST
   abort every open scope, so the command is deferred rather than refused.
7. A terminal command MUST be deferred, not counted, while a saga scope aborts or a step is under review or has an
   unknown outcome, and MUST put the run under review (`COMMAND_INVALID`) while a scope is open and not aborting, since
   only the function can close it; closing a scope while a member is not settled, and a command that names a closed
   scope, MUST be `COMMAND_INVALID`. An accepted terminal command MUST go through the stop drain, and `run.finished` MUST
   be written only when no leased work is left or what is left went to review.

## Considered options

1. **Leased steps in a code profile, risk metadata in the envelope, journal-derived saga dependencies** — principles III
   and V hold for code as for graphs; unknown outcomes are the platform's.
2. **The full node schema in fragments** — lost: run-time code would inject expressions, choose classifications, send
   output anywhere, set the four-eyes rule and claim an effect is reversible.
3. **Strict reverse completion order, one compensation at a time** — lost: one compensation under review would block
   independent ones (principle III).
4. **Local activities** — lost: an effect repeated without a lease, an attempt or a reconciliation after a worker crash.
5. **Hand-written compensation (Temporal)** — kept as a possibility (ordinary activities in a `catch`), rejected as the
   platform's answer.

## Consequences

- Limit: a gated activity's input and an approval's subject travel inline, so the sum of an activation's gated inputs
  and subjects is at most 192 KiB less the declared state's reserve (at least 64 KiB in all).
- Good: no blind repeat of an effect; compensation, review, secret disclosure and risk metadata identical in both modes.
- Bad: the code profile of each node kind is a second schema to keep in step with the first; eager dispatch waits for
  feature 006's acknowledgement.
- Compatibility with runs in flight: graph sagas keep their published plans; only code runs, which exist only after this
  decision, use run-time plans. Operational cost: one batched insert of plan rows per abort.

## Evidence

Design 044, Technical Plan 5.1, 10, 11.2; Decisions 044-045, 044-046, 044-078, 044-081, 044-083 and 044-090; the security,
correctness and architecture findings of rounds 1 and 2.
