# Correctness review of design 049, full round of 2026-11-26

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 66634fb997c95e6725e2e8773f120c112759b347
Reviewer: correctness (full round; the approval's correctness report is the re-read that follows the fixes)
Note: the reviewer's output, kept whole. The reviewer read files only and ran nothing. Line numbers are of the reviewed
commit.

## Blocker

1. The check on `runs` (bucket is legacy or the month of `created_at`) refuses a child created in a later month than its
   root, yet a child must take its root's bucket. Child `created_at` is `clock_timestamp()` at creation (execution.rs:1265,
   1418), and pending children are created later by a scheduler pass (maintenance.rs:542). Child chain successors stay in
   the parent's tree (chain_progress.rs:1759-1782). Design 049:93-101. Fix: apply the check only to roots, and add a
   database trigger requiring `bucket = root's bucket` for runs with a parent.
2. The late-legacy trigger refuses every child of a legacy root created more than 30 s after the switch. The error is
   `JOURNAL_SWITCHED`, retried forever, so legacy trees with pending fan-out or child chains never finish. Design
   049:98-101, 416-418. Fix: refuse a late legacy bucket only when `parent_run_id IS NULL`.

## Major

3. Purge key mismatch: `run_payloads` is keyed by `root_run_id`, but Requirements 049-002 and 049-006 say "same run" and
   049-014 purges by `(tenant, run_id, bucket)`. A child's purge deletes nothing; the root's purge deletes payloads that
   tree members not yet archived or purged still reference. Design 049:102-104, 135-136, 205-207. Fix: delete a tree's
   `run_payloads` only with its last run, after every member is verified; key the triggers by root.
4. "A chain's next segment (a new root) writes its own copy" is false for child chains: the successor keeps the
   predecessor's root and parent (chain_progress.rs:1759-1761). Tasks 049-5 and 049-12 would give it its own month and a
   copy, which splits the tree. Fix: a successor inherits the root's bucket; only root chains copy.
5. The mark-only form deletes the run's run-purpose blobs unconditionally. A rerun inherits the source's input blob from
   another tree (execution.rs:1275-1287; `runs_input_blob_same_scope_fk`). Retirement step 2 then fails with 23503, or
   leaves the rerun's input dangling. Design 049:209-211. Fix: keep feature 020's unreferenced-blob check in the
   mark-only form, or extend the copy rule of Requirement 049-005 to blobs.
6. Step 6 releases the quota first, which locks every affected tenant's `tenant_retained_usage` row `FOR UPDATE`. Only
   then does it run `DROP TABLE`, with `statement_timeout = 0`. The drop waits for any holder of a lock on the detached
   table (pg_dump, monitoring, autovacuum until the deadlock timeout). Meanwhile every content commit of those tenants
   blocks in its deferred flush. Design 049:444-446. Fix: take `ACCESS EXCLUSIVE` on the detached tables (or drop them)
   under `lock_timeout` first, then release.
7. The 120 s bound in Success criterion 049-002 is derived wrongly. A concurrent detach does not "wait twice for older
   transactions". It waits for its `SHARE UPDATE EXCLUSIVE` locks (anti-wraparound vacuum, `ensure_journal_buckets`),
   then for every current lock holder on the parent of any age, then for `ACCESS EXCLUSIVE` on the partition. A one-time
   "nothing older than 60 s" check bounds nothing. Steps 4 and 5 are full scans inside the detach-to-drop window and are
   not counted. Design 049:229-232. Fix: derive the bound from an enforced maximum transaction lifetime plus the measured
   scans.
8. The mode of the bucket lock is not stated, and it is taken per transaction. Exclusive try-locks make concurrent
   per-run purges in one bucket refuse each other. A per-chunk lock also lets retirement start between the chunks of a
   run that is mid-purge (`PURGING`). Design 049:167-168, 431-432. Fix: purges take a shared transaction-level try-lock
   and retirement an exclusive session lock; the mark-only form finishes runs already in `PURGING`.
9. Every run creation takes `FOR SHARE` on the single `journal_bucket_switch` row: this creates a MultiXact and writes WAL
   for each creation, and new sharers can keep overtaking the switch's `UPDATE`, which may starve (plausible). The bucket
   is also needed before the input payload insert, so the read cannot be "folded into the run insert". Design
   049:184-186. Fix: read the switch row unlocked in the existing `SELECT clock_timestamp()` (execution.rs:1265) and rely
   on the 30 s grace.
10. The migrations break design 053 rules beyond the two recorded exceptions: step 1 is a no-transaction file with dozens
    of lock-taking DDL statements that are not idempotent (`CREATE TRIGGER` has no `IF NOT EXISTS`), and 053's runner
    reruns the whole file on 55P03; step 2 holds several concurrent builds, plus the drop of its own index, in one file;
    step 5 takes its blocking lock in its first statement, not its last. Design 049:365-400 vs 053:65-73. Fix: one
    statement per migration, or record each exception as a lint waiver.
11. Requirement 049-011 ("compare the bucket with a bind parameter or a literal") cannot be met by reads that span many
    runs. `collection_candidates` (maintenance.rs:242-259) and `run_result` (maintenance.rs:581-583) join payloads by
    digest, so the bucket comes from a join column. Either the static check fails, or each scheduler pass locks every
    `run_payloads` partition. Fix: specify and budget the multi-run read pattern.

## Nit

12. The partition count in Open Question 2 is wrong. Requirement 049-008 keeps three months ahead, step 5 creates two,
    Open Question 2 says two. Retained months are `ceil(R/30)+1` with the current one, plus run lifetime: 90 d gives at
    least 7, 365 d at least 17. Design 049:273-275.
13. `transaction_timeout` does not exist on PostgreSQL 16, so lifting it fails there. Set it only where the server has
    it, as 053 does. Design 049:433.
14. "Dropped once no legacy run remains" never happens: stubs keep the legacy bucket forever. Say "no unpurged legacy
    run". Design 049:448-449.
15. Step 2 builds the partial legacy indexes of Requirement 049-007 in the migration, while 049-007 and Task 049-11
    deliver them later, gated after Task 049-3's measurement. Design 049:142-147, 382-384.
16. The step 5 parent must declare `DEFAULT '1970-01-01'` on `journal_bucket`. Otherwise inserts from the previous
    release, which carry no bucket, fail with 23514. Design 049:391.
17. Mapping a missing older partition to non-retryable `RUN_PURGED` is wrong during a reattach (step 4) or a crashed
    detached-not-dropped state. In that state a per-run purge of a `CONTENT_PENDING_DROP` run also reports done while its
    bytes stay charged. Design 049:156-158, 178-180.
18. `UPDATE OF col` triggers fire even when the column is unchanged; the built-in key check skips that case. Add `WHEN
    (OLD.col IS DISTINCT FROM NEW.col)`. Design 049:133.
19. There are two clocks: `created_at` is the database clock (execution.rs:1265), but command time from the application
    for chain successors (chain_progress.rs:1283, 1781). The month-boundary suite of Task 049-5 must drive both.
20. Requirement 049-012 counts only tables an operation writes, but lease reads of `run_payloads` lock a partition too.
    Design 049:198-201.

Counts: Blocker 2, Major 9, Nit 9
