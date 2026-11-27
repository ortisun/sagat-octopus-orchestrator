# Architecture review of design 049 (2026-11-24)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: aa07ae4189cc61ab3770aae749af36d5a6a124af
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files
only (the design, Architecture decision 0132, Architecture decisions 0027, 0054 and 0084, feature 020, the
migrations) and ran nothing; findings that rest on PostgreSQL internals name the function for Task 049-3 to confirm.

## Blockers

1. Retirement cannot detach a `run_payloads` partition while unpartitioned tables still reference it: PostgreSQL
   refuses the detach while a referencing row points into it (`ATDetachCheckNoForeignKeyRefs`), and marking runs purged
   deletes none of those rows. Fix: either only bucket-partitioned tables reference `run_payloads`, or retirement runs
   feature 020's per-run delete of the unpartitioned rows first and the success criterion counts it.
2. Dropping a partition never gives the tenant's content quota back: the quota triggers fire on rows, `DROP TABLE` fires
   none (Architecture decision 0027: purges release only the rows deleted; Architecture decision 0084 rejected time
   partitioning for this reason). Fix: sum the bytes per tenant from the detached table and release them in one audited
   transaction before the drop; a reconciliation test.
3. Dropping journal partitions needs an amendment of principle I (the journal is append-only), which feature 020's
   Architecture decision 0054 also waits for and which the delegation does not cover. Fix: owner gates on Tasks 049-12
   and 049-14, the Constitution Check row conditional, listed under "Owner decisions needed".

## Majors

1. The nullable stub (Requirement 049-015) breaks older readers (`runs.input_digest` is `NOT NULL` and read as a
   non-optional value), contradicts Architecture decision 0054 decision 5, depends on open owner question O1082, and is
   missing from Architecture decision 0132.
2. Architecture decision 0132 replaces parts of 0054 (decisions 4 to 6, option 3) while saying it refines it; its
   decisions 3 and 6 have no check.
3. Sending statements unprepared does not stop foreign-key checks, whose cached internal plans become generic, from
   locking every partition; the lock count must include them.
4. On PostgreSQL 16 and 17 a `NOT VALID` foreign key on a partitioned referencing table is refused; the contract step
   would scan 100 GiB under a lock that blocks inserts. Fix: per partition, then the parent.
5. Step 4's rename breaks design 048's own rule (Requirement 048-032); `lock_timeout` bounds the wait, not the hold;
   attaching to the referenced `run_payloads` also locks the referencing tables.
6. The dual write and per-run copies are charged twice against the tenant's content quota.
7. The payload move can miss payloads that processes of the previous release write during the rollout; it must start
   after the census.
8. With mixed retention policies (no policy keeps everything, the default), buckets will almost never drop; the drop is
   opportunistic and the per-run purge the main path; a reason label on held runs.
9. Approved features that plan new references to `payloads` (022, 026, 030, 025) are not updated, and decision 3 has no
   catalog check.
10. Retirement is a second deletion path and nothing says which process holds the owner credential.
11. Dropping audit partitions is a new irreversible decision with no record and no owner answer.
12. "One bucket per run" is only tested; a rollback after the switch writes dated runs' events into the legacy
    partition silently. Fix: a foreign key `(tenant_id, run_id, journal_bucket)` to the run, drop the default at the
    switch, the switch raises the minimum release.

## Nits

1. 22 columns in 14 tables (12 run-owned, 2 shared), not 15 tables.
2. A missing partition returns SQLSTATE 23514, the same as a failed check; name a typed code.
3. A crash during a concurrent detach leaves it pending; it needs `DETACH ... FINALIZE`.
4. `ensure_journal_buckets()` needs a lock or `IF NOT EXISTS`, statement-level truncate triggers per partition, and the
   check that triggers are enabled always.
5. The 120 s bound is mis-derived (idle timeout bounds idle gaps, not transactions; owner jobs and backups hold
   snapshots); state a measured target.
6. Audit partitioning makes the key `(id, created_at)`; the legacy check boundary must be a future month.
7. A retirement's audit record needs a platform-level home.
8. The static check must require a bind parameter or literal, and cover migrations and functions.
9. The lock limit is stated two ways.
10. Owner decision 2026-11-24-4 is not a row in this tree's `docs/owner-decisions.md`.

Counts: Blocker 3, Major 12, Nit 10
