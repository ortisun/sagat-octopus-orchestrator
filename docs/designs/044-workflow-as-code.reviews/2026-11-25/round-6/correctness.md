# Correctness review of design 044, round 6 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: b5c23b83c9a63c8ed42f15b4cdf881ecb683dd4e
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 5's Majors 1 to 5
are closed (the reset side of 4 returns as F); round-5 Nits 8 to 10 closed, 6 and 7 partly.

## Major

A. `continueAsNewSuggested` and `historyLimitReached` reach the function only through the lease, so a cold replay in the
   closing window diverges: journal both flags in each `workflow_task.started`; a cold-replay test.
B. A continue-as-new accepted after `run.cancel_requested` loses the cancellation, or meets the chain and link latches'
   unmapped errors: end such a run `CANCELLED`; set the chain latch; barrier tests.
C. A move applies to a run in its stop drain and reopens a `CLOSED` workflow-task step: refuse it (`RUN_NOT_MUTABLE`).
D. The "terminate" remedy and "a termination" stop do not exist on this platform: use cancel, which stops a run under
   review at once; a test.
E. Feature 018 defines `EXCLUDE` as making every later point ineligible: `run.code_version_moved` must be `COPY`, and a
   settled code abort must not make later points ineligible; tests after a move and after a caught abort.
F. A reset with an open code update: refuse such a point; `reapply: UPDATES` refused for code runs; a test.
G. A completion over the run's 4 MiB or the tenant's content quota has no defined outcome and may reach
   `stop_for_storage_quota`: a cause, counted for the tenant quota, a review for the run's own bytes; tests.

## Nit

1. Cooperative cancellation is in no architecture decision.
2. A test asserts a consumption in the crossing completion, which is refused.
3. Which clock decides the cancellation deadline against a completion.
4. `409 RUN_CLOSING` for updates has no test.

Counts: Blocker 0, Major 7, Nit 4
