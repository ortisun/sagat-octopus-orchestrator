# egm-design (critic) on design 049, full round of 2026-11-26

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 66634fb997c95e6725e2e8773f120c112759b347
Note: the reviewer's output, kept whole. The reviewer read files only and ran nothing. Line numbers are of the reviewed
commit.

## Blocker

1. A child created in a later month than its root breaks two rules. The CHECK that the bucket is legacy or the month of
   `created_at` refuses it. The late-legacy trigger refuses every child of a legacy root after switch + 30 s, so in-flight
   legacy runs can no longer start children. Where: 049 L93-101, L182-187, L414-419; ADR-0132 d7. Fix: a child's bucket
   must equal its root's, checked by a trigger that reads the root (not a CHECK), and late-legacy exempts children of
   legacy roots.
2. `run_payloads` is keyed by `root_run_id`, but purge (049-014), the post-detach check (step 4) and feature 020's
   per-run archive/purge jobs all work per run. Purging the root first deletes payloads its unpurged, possibly unarchived
   children still reference; purging a child first leaves them for good. Where: L119-128, L205-213, L441-442; 020
   FR-020-034. Fix: delete a tree's `run_payloads` only when its last run is purged (or purge per tree), and make step 4
   check per tree.

## Major

1. The switch row is read only at the run insert, after the input payload (and its table, `payloads` or `run_payloads`)
   was already chosen. A switch committing in between makes the reference trigger refuse the creation, so "no creation in
   flight is refused" is false. Also, `FOR SHARE` on one row from every creation churns multixacts on a single hot tuple.
   Where: L96-97, L182-187. Fix: take a shared advisory lock (or read the switch) in the creating transaction's first
   statement; drop the row lock.
2. Step 1 is one no-transaction file of many non-idempotent statements. That breaks 053-002 (one concurrent statement per
   no-transaction file; the runner retries whole files). Each of the 21 FK drops takes ACCESS EXCLUSIVE on
   `payloads`/`runs` too (the referenced table's RI triggers are removed), plus 13 ADD COLUMNs and about 16 CREATE
   TRIGGERs. Where: L365-381. Fix: one idempotent migration per blocking statement, and bound the total stall, not just
   each statement's.
3. These migrations ship in 0.3.0, which is a stopped upgrade (L399-402). So the online machinery and Success criterion
   049-003 (100 GiB under load) cover a path no install takes. Fix: say which release runs them online, or drop SC
   049-003 and the under-load requirement.
4. The drop path is weighed unfairly. A month is pinned by any run in a namespace with no policy (the default), and by
   any never-final chain. The mark-only form still deletes every run-owned row of 12 unpartitioned tables, so "avoids
   millions of deletes" (L512-513) is overstated. Where: L161-181, L251-252, Alternatives. Fix: estimate how often a month
   qualifies, and weigh per-run purge by key alone (deferring Task 049-14) or partitioning by retention class.
5. The reference trigger checks "same tenant, run and bucket" (L133-137), but `run_payloads` has no run_id; the key is
   the root. Fix: define the lookup by root, and carry `root_run_id` on referencing rows (or count the extra `runs` lookup
   in budget 049-012).

## Nit

1. The `UPDATE OF` reference triggers fire whenever the column appears in the SET list; the built-in key check skips
   unchanged keys. Where: L133. Fix: add `WHEN (OLD.col IS DISTINCT FROM NEW.col)`.
2. `transaction_timeout` does not exist on PostgreSQL 16 (it arrives in 17), so setting it errors. Where: L433. Fix:
   "where the server has it", as design 053 says.
3. The 120 s detach-to-drop target counts only the detach waits. It leaves out the full scans of steps 4 and 5 over the
   detached tables. Where: L230-232. Fix: budget those scans or measure them separately.
4. On PostgreSQL 16 a cached generic plan locks every partition before pruning, and that includes PL/pgSQL trigger
   lookups. Open Question 8 does not list `ALTER FUNCTION ... SET plan_cache_mode = force_custom_plan` scoped to the
   trigger functions. Where: L292-297. Fix: add it as a fourth option.
5. `ensure_journal_buckets()` standalone tables should be created `LIKE` the parent, so a column added later (feature
   032) cannot break the attach. It should also assert the cloned triggers' `tgenabled`. Where: L148-151.
6. Late-legacy compares the application-bound `created_at` with the switch time, so clock skew across hosts matters.
   Where: L98-101. Fix: compare on the database clock.
7. The bucket lock spans several transactions and a DETACH CONCURRENTLY. It must be a session-level advisory lock on a
   direct connection, not one through a transaction pooler. Where: L429-433. Fix: state it.
8. Open Question 4 says new writes "never touch" the partial legacy indexes; Requirement 049-007 says their cost on new
   writes is not nil. Where: L283 vs L144-146. Fix: reconcile the two.

Counts: Blocker 2, Major 5, Nit 8
