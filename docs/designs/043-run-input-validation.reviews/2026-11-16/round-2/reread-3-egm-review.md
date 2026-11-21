# Targeted re-read 3 of design 043: egm-review lenses (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: 2606e1d59960a5b72e35726a7b6db4e9f05fa74a
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the author's summary of two lenses run on the changed text (egm-comprehension and egm-implementation-readiness), a
third targeted re-read, not a complete run of the five stages.

## egm-comprehension

The Major of re-read 2 (the counting text) is resolved. Major: 1. FR-043-018 says a 0.2.x database has no schedules while
Technical Plan 8 says it already has the tables. Nits: 2. `check_before_its_start` and `check_ahead` appear only in the
sketch; 3. "one action is one validation"; 4. the rerun sentence of FR-043-018 holds only in `enforce` and gives no cause;
5. "the first's down migration" has no nearby antecedent.

## egm-implementation-readiness

No Major; every cited file and test exists. Nits: 1. "a draft save" runs no pre-check; 2. T043-24 omits the 0.2.x test,
`schedules_exist` and the runtime role; 3. the rerun sentence holds only in `enforce` and cites no test; 4. the sketch says
"before its decision" only.

Counts: Blocker 0, Major 1, Nit 8
