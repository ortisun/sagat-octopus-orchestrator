# Architecture review of design 044, round 4 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: ba5a3fdd02abfc57e03b76efc4a9fb2e237b5c45
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Every Blocker of rounds 2
and 3 is closed; round 3's Majors 2, 3, 6 to 8, 10 to 16, 18 to 26 are closed; 4, 5, 9, 17 and round 2's 14 are partly
closed and raised below.

## Major

1. The journal is not self-contained for a code run: command records carry no node spec or body digest, origin is
   stored nowhere, the seed only on the run row; `verify_run`, the reset's chain, the move's first difference and the
   waking rule need them; the overlay holds facts no event has. Put them in the journal and make the overlay its
   projection; a parity test of `is_waking` against the reconciler's SQL.
2. The code-run reset rule drops "no active scope", so a point inside an open saga becomes eligible and the source's
   cancellation compensates effects the reset run believes hold; keep the condition.
3. Publication and removal of the requirement can both commit under read committed; hold the requirement `FOR SHARE`
   through a definer function shared with Architecture decision 0110 decision 8; a barrier test.
4. Design 045's fields are read only above a level computed at publication, which a code version cannot compute from its
   document; the envelope declares the level; a fragment above it is refused.
5. The fallback that builds Architecture decision 0111's inbox here builds another feature's mechanism whose design does
   not exist and contradicts Architecture decision 0108; remove it and gate Task 044-21.
6. Architecture decision 0106 still mandates the Python import finder the design rejected, and omits the UTC
   precondition.
7. The fragment chooses feature 022's limits, periods and groups at run time; the envelope entry owns them.

## Nit

8. "Enforced by" lines name tests the tasks do not create; the per-statement fault test.
9. Architecture decision 0105's numbering and "fits" wording.
10. Gap 044-021 is stale against the draft.
11. Three reads missing from the statement budget (pin, pause, outstanding count).
12. `APPROVAL` already exists with `approval_request_id`.
13. Scope the raw-insert check to `crates/` and `apps/`.
14. Fail closed only for a missing node in `ensure_activity_bound`.
15. A deadline must end a run whose workflow task is under review.
16. Task 044-6 before Task 044-3.
17. One owner for `steps.min_protocol_minor`.
18. Code signals must not satisfy wait-for-event steps.
19. The `sagat.workflow` bucket is a new quota rule; refuse tenant overrides on `sagat.` names.
20. The closing window counts workflow-task events.
21. "MUST NOT be offered to any other session".
22. Compatibility and operational-cost lines in Architecture decisions 0106 and 0109.
23. An empty `activities` list relaxes a domain rule and the contract.
24. The SDK tasks' gate lines should name the protocol dependencies.
25. A source-text check that only the move item's apply path moves runs.

Counts: Blocker 0, Major 7, Nit 18
