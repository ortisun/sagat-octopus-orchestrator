# Targeted re-read 1 of design 054: egm-review lenses (2026-11-26)

Doc: docs/designs/054-buffered-signals.md
Reviewed-commit: f322a84080a2920e0d2c15cc43b09d4628c6caef
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the author's summary of one lens (egm-design, dispatched directly) run on the fixes of the Blocker and Major findings of
stages 5 to 9 and on the text changed since, a targeted re-read (Owner decision 2026-11-25-2, step 4), not a complete run of the
five stages. The lens read files only and checked fixes against the code (the ledger's re-read before each charge, the
`payloads.content` column, the run statuses, the registry's lock order, `chainPolicy` and `continue_evaluated`, feature 018's
reset eligibility, the gate row).

Every Blocker and Major of the stage reports is resolved (zoom-out 1 to 4; comprehension 1 to 5; design critic 1 to 7;
structural 1 and 2; intermediate zoom-out 1 and 2; readiness 2 to 7; codebase details 1 to 9), except readiness 1 (finding 1).

Counts: Blocker 0, Major 1, Nit 4

## Major

1. **Readiness 1 is not resolved in Task 054-3,** which still states the old `NO_CONSUMER_AHEAD` predicate that refuses an early
   signal, though Requirement 054-005 states the corrected one. *Answer: fixed:* Task 054-3 states the predicate in the words of
   Requirement 054-005, with unit tests of an early signal before routing, a branch not taken and every wait entered.

## Nit

2. The Risks table cites Task 054-3 for the store-side release test, which moved to Task 054-9. *Answer: fixed.*
3. Technical Plan 3 gives an older signature of `buffer_signal`. *Answer: fixed:* it refers to Technical Plan 17.
4. The code-run exception refuses only above 2 MiB, so one payload can overshoot. *Answer: fixed:* refused when pending bytes and
   the new payload together would pass 2 MiB.
5. Owner decisions needed omits Tasks 054-4 and 054-6. *Answer: fixed.*
