# ADR-0107: Effects from code are leased steps only, and a saga opened from code is planned by the engine

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation (OD-2026-11-20-6 for Owner questions 044-007 and 044-010; OD-2026-11-24-4 for the acceptance)
**Refines / supersedes:** applies ADR-0004, ADR-0010 and ADR-0076 to code workflows
**Enforced by:** `an_aborted_code_saga_compensates_in_reverse_completion_order`; `an_unknown_outcome_in_a_code_saga_waits_for_review`; `eager_activity_leases_are_granted_in_the_completion_transaction`; the absence of a local-activity command, held by `every_command_kind_round_trips_through_the_node_parser`; the conformance scenarios `code-saga` and `code-saga-uncertain` (design 044, Tasks 044-5, 044-14, 044-22, 044-26)

## Context

Principle III: effects are at-least-once, an unknown outcome is consulted or reviewed before it is repeated, and
compensation is a step of its own with the same lease, retry and audit machinery. Temporal offers local activities,
whose effect runs inside a workflow task and is recorded with its completion, and leaves compensation to hand-written
code. Feature 037 treats every leased activity as external-capable.

## Decision

1. The function MUST NOT run an effect through the platform except as a leased step (activity, script, child, approval)
   created by a command. There MUST be no local activity.
2. A completion MAY offer eager activity slots; the server MUST lease up to that many of the activities it just
   created to the completing session, inside the completion's transaction, only for activities the session serves and is
   bound to. A sticky session's activity completion MAY return the run's next workflow task lease.
3. A saga opened from code MUST be an engine compensation domain: activities scheduled in it get the derived steps of
   ADR-0010; closing it drops the compensations; aborting it MUST plan the compensations of its completed steps in
   reverse order of completion, one at a time, and run them with the rules of features 003 and 037; the function MUST
   learn only the scope's outcome.
4. An unknown outcome of an activity from code MUST follow feature 037 (reconciliation, then review); the function MUST
   NOT be able to repeat an unknown effect itself.

## Considered options

1. **Leased steps only, eager dispatch, engine-planned sagas** — principle III holds for code as for graphs; unknown
   outcomes are the platform's, not each author's.
2. **Local activities** — lost: an effect repeated without a lease, an attempt or a reconciliation after a worker crash.
3. **Hand-written compensation (Temporal)** — kept as a possibility (ordinary activities in a `catch`), rejected as the
   platform's answer.
4. **Compensation in parallel by declared dependencies** — not in 0.3.0: code declares none, and reverse completion is
   the safe order.

## Consequences

- Good: no blind repeat of an effect; compensation and review identical in both modes.
- Bad: sequential compensation can be slower than a parallel plan; a short effect pays a lease (mitigated by eager
  dispatch, measured by the commit count of design 044's Non-functional requirement 044-002).

## Evidence

Design 044, Technical Plan 10, 11.2; Owner questions 044-007 and 044-010.
