# Correctness confirmation check of design 041 (second re-read)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: fac00787bcaea68b47252139aadbe20bdfc660a5
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: a fresh `correctness-reviewer`, asked to confirm the fixes of its re-read's four Majors; no shell. Recorded condensed.

Re-read Majors 1 to 4 fixed (the end rule stated alike in four places with a pure predicate and new tests; the caught-up client polls; the marker's failure path; the 12,000-line tests on `isolated_server`).

## Blocker

None.

## Major

1. The marker's new 200 ms lock timeout breaks the T041-13 tests that hold a lock the marker waits on (the terminal-transition test, the barrier test): they pass only by the clock or roll the marker back; make the timeout a parameter the tests set long.

## Nit

1. data-model.md §9 says "any error" is counted `failed`, while `tenant_step_log_quota_exceeded` is `skipped_quota`.

Counts: Blocker 0, Major 1, Nit 1
