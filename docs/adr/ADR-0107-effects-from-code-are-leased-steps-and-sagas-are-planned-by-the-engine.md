# ADR-0107: Effects from code are leased steps in a code profile, and a saga opened from code is planned by the engine

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation (OD-2026-11-20-6 for Owner questions 044-007 and 044-010; OD-2026-11-24-4 for the acceptance)
**Refines / supersedes:** applies ADR-0004, ADR-0010 and ADR-0076 to code workflows
**Enforced by:** `an_activity_command_cannot_carry_a_secret_of_another_activity_entry`; `a_fragment_with_an_expression_field_is_refused`; `a_fragment_with_destinations_is_refused`; `a_fragment_with_an_output_schema_is_refused`; `an_approval_with_an_inline_policy_is_refused`; `publication_refuses_an_envelope_activity_with_a_reserved_prefix`; `an_aborted_code_saga_compensates_in_reverse_completion_order`; `an_unknown_outcome_in_a_code_saga_waits_for_review`; `a_terminal_command_with_an_open_saga_is_refused`; `eager_activity_leases_are_granted_in_the_completion_transaction`; `an_idempotent_replay_returns_no_lease`; the conformance scenarios `code-saga` and `code-saga-uncertain` (design 044, Tasks 044-3, 044-5, 044-14, 044-22, 044-26)

## Context

Principle III: effects are at-least-once, an unknown outcome is consulted or reviewed before it is repeated, and
compensation is a step of its own with the same lease, retry and audit machinery. Principle V: an ordinary task receives
only the secrets declared by its immutable task definition. A graph node's fields are written by the publisher; some of
them are expressions the server evaluates (`idempotencyTemplate`, `correlationExpr`, a child's `input`) and some decide
what a reader may see or where data goes (`outputSchema`, `destinations`, an approval's policy). Temporal offers local
activities and leaves compensation to hand-written code.

## Decision

1. The function MUST NOT run an effect through the platform except as a leased step (activity, script, child, approval)
   created by a command. There MUST be no local activity.
2. A work command MUST carry a fragment of the code profile of its node kind: every expression or template field MUST be
   a literal or refused, and every publisher-only field (`outputSchema`, `destinations`, an inline approval policy,
   `onError`) MUST be refused; the output schema of an activity and the approval policies MUST come from the published
   code version's envelope.
3. A command MUST stay within the current code version's envelope: an activity entry listed by name or prefix, with only
   that entry's secrets (checked at publication under feature 009); a script listed by digest and ready for the tenant and
   namespace; a listed child workflow; a named approval policy. No activity name of an envelope or a command MAY start
   with `script.` or `sagat.`. The accepted fragment MUST be stored immutably, and the secrets of a `c-<n>` step MUST be
   read from it.
4. A completion MAY offer eager activity slots and a sticky session's activity completion MAY accept the run's next
   workflow task; the leases MUST be granted in the transaction that answers, only for activities the session serves and
   is bound to; an idempotent replay MUST NOT return a lease, and the idempotency record MUST NOT store a lease token.
5. A saga opened from code MUST be an engine compensation domain: activities scheduled in it get the derived steps of
   ADR-0010; closing it drops the compensations; aborting it MUST plan the compensations of its completed steps in reverse
   order of completion, one at a time, in run-scoped plan rows, with compensation identifiers derived from the start
   version and the forward node, and run them with the rules of features 003 and 037; the function MUST learn only the
   scope's outcome.
6. A terminal command MUST be refused while a saga scope is open or aborting or a step is under review or has an unknown
   outcome; other outstanding work MUST be cancelled by its policies in the same transaction.
7. An unknown outcome of an activity from code MUST follow feature 037 (reconciliation, then review); the function MUST
   NOT be able to repeat an unknown effect itself.

## Considered options

1. **Leased steps in a code profile, an envelope per activity, eager dispatch, engine-planned sagas** — principle III and
   V hold for code as for graphs; unknown outcomes are the platform's, not each author's.
2. **The full node schema in fragments** — lost: run-time code would inject expressions, choose classifications, send
   output to any connection and set the four-eyes rule of an approval.
3. **One secret set for the whole version** — lost: code could attach any listed secret to any listed activity.
4. **Local activities** — lost: an effect repeated without a lease, an attempt or a reconciliation after a worker crash.
5. **Hand-written compensation (Temporal)** — kept as a possibility (ordinary activities in a `catch`), rejected as the
   platform's answer.

## Consequences

- Good: no blind repeat of an effect; compensation, review and secret disclosure identical in both modes.
- Bad: sequential compensation can be slower than a parallel plan; a short effect pays a lease (eager dispatch, measured
  by design 044's Non-functional requirement 044-002); the code profile of each node kind is a second schema to keep in
  step with the first.

## Evidence

Design 044, Technical Plan 5.1, 10, 11.2; Decisions 044-045, 044-046, 044-056, 044-060 and 044-061; the security
findings of round 1.
