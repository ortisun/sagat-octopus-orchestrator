# Targeted re-read of design 046, second pass: correctness (2026-11-26)

Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: f40f0080e80b82e64de28e87fa887243a7abae08
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output, condensed by the author; every finding and severity is kept. A confirmation of the fixes
of `reread-1-correctness.md`; the reviewer read the diff of the fixes, the design, data model and contracts, and the
crates it cites; it ran nothing.

## The previous findings

All nine answered: the build job failed and the quota joining the version state (versions are inserted `BUILDING`,
`scripts.rs:795`); the engine's lock order matching `execution.rs:479-500` and `2197-2220`; the materialization route
failing the activity (the protocol not updated: finding 4); the set of evaluated `locked_seq` values; `StaleInputs` at
most 5 times (finding 3); `CANCELLED` in the data model (the design not updated: finding 4); backfill per (artifact,
target) (finding 1); one deadline (120 s + 4,096/100 × 20 s = 939 s, about 16 minutes); the lock timeout and alert.

## New findings

1. Major. The `BACKFILL` partial unique index has no state condition: a backfill that exhausts its attempts, or whose
   version stops being `READY`, ends `FAILED`, and every later pass's insert for that pair hits the index, so the target
   never gets a record. Fix: index only `QUEUED` and `RUNNING`, or re-queue `FAILED`; pick the version at claim;
   `a_failed_backfill_is_retried_by_the_next_pass`.
2. Nit (plausible). An install whose last attempt is lost is refused by the claim's sweep, not a completion, so the build
   job stays `QUEUED`; a `RUNNING` sibling whose lease lapses after a refusal is never reclaimed. Fix: the sweep runs the
   refusal steps and cancels expired `RUNNING` jobs of non-`BUILDING` versions.
3. Nit. The finish no longer says it saves the findings and revisions it was given, so a `READY` version may carry a
   resolve-time report. Fix: step 2 writes them.
4. Nit. The protocol's materialization row omits the failing of the activity; the design's job states omit
   `CANCELLED`; an author-lock publication has no job to "fail retryable" after 5 `StaleInputs`.

Counts: Blocker 0, Major 1, Nit 3

## The author's answers (all fixed; Decision 046-075)

1. The index covers `QUEUED` and `RUNNING` only, a failed pair is queued again by the next pass, the version is picked at
claim; the test named. 2. The sweep runs the refusal steps and cancels expired `RUNNING` jobs of non-`BUILDING`
versions; `a_lost_last_install_attempt_fails_the_build_job`. 3. Step 2 writes the evaluation and revisions. 4. The
protocol row, the job states, and `503 SCRIPT_POLICY_BUSY` for an author-lock publication.
