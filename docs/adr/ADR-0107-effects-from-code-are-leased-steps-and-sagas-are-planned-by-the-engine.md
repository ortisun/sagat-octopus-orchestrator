# ADR-0107: Effects from code are leased steps in a code profile whose risk metadata the publisher owns, and a saga opened from code is planned by the engine

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation (OD-2026-11-20-6 for Owner questions 044-007 and 044-010; OD-2026-11-24-4 for the acceptance); no person has reviewed it
**Refines / supersedes:** applies ADR-0004 and ADR-0076 to code workflows; supersedes ADR-0010 decision 3 (a saga's plan is a pure function of the published definition) for code runs only
**Enforced by:** `an_activity_command_cannot_carry_a_secret_of_another_activity_entry`; `a_compensation_receives_the_secrets_of_its_own_entry`; `a_fragment_that_changes_irreversible_is_refused`; `a_fragment_with_an_expression_field_is_refused`; `a_fragment_with_destinations_is_refused`; `a_fragment_with_an_output_schema_is_refused`; `an_approval_with_an_inline_policy_is_refused`; `publication_refuses_an_envelope_activity_with_a_reserved_prefix`; `publication_refuses_a_compensate_with_that_is_not_an_entry`; `an_aborted_code_saga_compensates_in_reverse_dependency_order`; `independent_code_compensations_run_while_one_is_under_review`; `an_unknown_outcome_in_a_code_saga_waits_for_review`; `a_terminal_command_with_an_open_saga_is_refused`; `a_terminal_command_waits_for_a_leased_activity_before_run_finished`; `a_code_step_output_beyond_the_retained_bytes_is_an_unknown_outcome`; `eager_activity_leases_are_provisional_and_granted_in_the_completion_transaction`; `an_unacknowledged_eager_lease_expires_without_an_unknown_outcome`; `an_idempotent_replay_returns_no_lease`; the conformance scenarios `code-saga` and `code-saga-uncertain` (design 044, Tasks 044-3, 044-5, 044-14, 044-22, 044-26)
**Required before acceptance:** the fault tests of design 044 Task 044-22

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
   `irreversible`, `highRisk`, `compensateWith`, `reconcileWith` and `outputSchema` MUST come from the published
   envelope entry, which a fragment may repeat or omit but never change.
3. A command MUST stay within the current code version's envelope: an activity entry listed by name or prefix, the names
   of its `compensateWith` and `reconcileWith` entries too; a derived step MUST receive the secrets of its own entry; a
   script listed by digest and ready; a listed child workflow; a named approval policy. No activity name MAY start with
   `script.` or `sagat.`. An activity input MUST be validated and classified by its entry's `inputSchema`.
4. A successful report whose output cannot be retained MUST be an unknown outcome (`STORAGE_QUOTA_EXCEEDED`), reconciled
   or reviewed under feature 037; the function MUST NOT be told the step failed.
5. Eager activity leases MUST be provisional leases of feature 006, released safely when unacknowledged; an idempotent
   replay MUST NOT return a lease, and the idempotency record MUST NOT store a lease token.
6. A saga opened from code MUST be an engine compensation domain: closing it drops the compensations; aborting it MUST plan
   the compensations of its completed steps in run-scoped plan rows (`code_saga_plan_entries`), with step B depending on
   step A only when B's command was issued in an activation after the one that delivered A's outcome, so that independent
   compensations stay eligible together; compensation identifiers MUST be derived from the start version and the forward
   node; they MUST run with the rules of features 003 and 037; the function MUST learn only the scope's outcome.
7. A terminal command MUST be refused while a saga scope is open or aborting or a step is under review or has an unknown
   outcome; otherwise it MUST go through the stop drain, and `run.finished` MUST be written only when no leased work is
   left or what is left went to review.

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

- Good: no blind repeat of an effect; compensation, review, secret disclosure and risk metadata identical in both modes.
- Bad: the code profile of each node kind is a second schema to keep in step with the first; eager dispatch waits for
  feature 006's acknowledgement.

## Evidence

Design 044, Technical Plan 5.1, 10, 11.2; Decisions 044-045, 044-046, 044-078, 044-081, 044-083 and 044-090; the security,
correctness and architecture findings of rounds 1 and 2.
