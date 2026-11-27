# Targeted re-read of design 046 after review round 3: correctness (2026-11-26)

Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: b851a417d21d874d2ba186a9569e8d511ceac2e7
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output, condensed by the author; every finding and severity is kept. A targeted re-read of the
changes since round 3 (approval rule of Owner decision 2026-11-25-2), not a full review; the reviewer read the diff since
`4c35bed4`, the design, the data model and contracts, the round-3 report and resolution, and the crates it names; it
ran nothing.

## Round 3

Answered: 2, 3, 5, 6, 7, 9, 11, 12, 13, 15. Partly answered: 1 (finding 1 below), 4 (3), 8 and 10 (5), 14 (8).

## Findings

1. Major. Only resolve and install jobs of `BUILDING` versions stop counting; the quota query
   (`crates/infra-postgres/src/quotas.rs:1198-1203`) still counts every `QUEUED` build job, so each refused install leaves
   a build job that counts against `script_jobs` forever. Fix: fail the build job in the refusing transaction, or join the
   version state; the test must assert the count.
2. Major (plausible). `fail_ready_step` locks the step row first, but the lease path locks the run first
   (`execution.rs:2199-2220`), so the two can deadlock. Fix: the engine's run transaction (chain, then run), then the
   step; the barrier test between the two locks.
3. Major. The block pass runs once per block; a step of that artifact that becomes ready later, on a runner without its
   volume, gets `403` from the materialization route and nothing more, and `LAYER_UNAVAILABLE` never fires. Fix: the
   materialization route fails that activity through `fail_ready_step`; a test.
4. Major. The switch refuses only when a row above the highest `locked_seq` read exists, so a writer holding a lower
   value that commits late is missed. Fix: pass the set (or count) of evaluated rows and refuse when any row above the
   mark is not in it; a held-writer test.
5. Major. Technical Plan 9 step 2 still re-runs the evaluation inside the finish and ignores waiver expiry, against
   Technical Plan 1's `StaleInputs`; the retry has no bound. Fix: compare, else `StaleInputs`; bound the retries.
6. Nit. A sibling "finishes as cancelled" but the job states have no `CANCELLED`, and a `QUEUED` sibling never ends.
7. Nit. Backfill jobs are per version but records per (artifact, target): cache-hit versions sharing an artifact have no
   defined second outcome. Fix: one job per (artifact, target); `AlreadyDone` or a typed mismatch.
8. Nit. The bounds table ("600 s") and Technical Plan 3 ("at most 10 minutes") contradict the growing deadline.
9. Nit (plausible). Shared lockers may keep passing a waiting exclusive locker, delaying the barrier and the switch. Fix: a
   `lock_timeout` with retry and an alert.

Counts: Blocker 0, Major 5, Nit 4

## The author's answers (all fixed; Decision 046-074)

1. The refusing transaction fails the build job and cancels `QUEUED` siblings with `SKIP LOCKED`; the quota joins the
   version state for both tables; `a_refused_install_releases_the_quota_count`. 2. The engine's order, chain, run, step;
   the barrier between the run and step locks. 3. The route fails the activity; `a_step_ready_after_the_block_pass_fails_at_materialization`.
4. The set of evaluated `locked_seq` values; `a_late_writer_with_a_lower_sequence_is_evaluated_before_the_switch`.
5. Step 2 compares and answers `StaleInputs`, at most 5 in a row. 6. `CANCELLED`, set by the refusal or by the sibling
   itself. 7. One job per (artifact, target), `AlreadyDone` or `LAYER_DIGEST_MISMATCH`. 8. One deadline everywhere.
9. A 5 s `lock_timeout`, backoff, and an alert at 10 minutes.
