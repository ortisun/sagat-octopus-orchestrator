# Correctness review of design 049 (2026-11-24)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: aa07ae4189cc61ab3770aae749af36d5a6a124af
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files
only and ran nothing; "plausible" findings rest on PostgreSQL internals and name the check that would confirm them.

## Blockers

1. The legacy partition's new key is a bare unique index, so the attach of step 4 builds a new unique index on 100 GiB
   while the renaming transaction holds `ACCESS EXCLUSIVE`. Fix: a short migration before step 4 that replaces the
   primary key `USING INDEX`; a migration test that the legacy table's indexes are unchanged by step 4.
2. Foreign keys that reference the partitioned `run_payloads` make retirement fail or block the hot path: a detach
   checks every referencing table in `SHARE` mode and joins it against the partition, and a detached `run_events`
   partition keeps its own key to `run_payloads`. Fix: no foreign keys from the 20 columns to `run_payloads`; enforce
   the reference otherwise, or order the deletes and measure the locks.
3. A run-owned reference without a foreign key, `collection_scope_items.item_digest`, is missing from the census: the
   move skips the items not yet admitted and the cleanup deletes their content. Fix: a checked registry of reference
   columns, foreign keys or not; a test that fails on an unregistered digest column.

## Majors

4. Detaching or dropping a partition that has a foreign key to `runs` takes heavy locks on `runs` (the drop removes the
   key's triggers on `runs`).
5. A `NOT VALID` foreign key on a partitioned table is refused before PostgreSQL 18.
6. Nothing in the database ties a row's bucket to its run, and the legacy defaults stay after the switch: an insert
   site that forgets the column, or an older process after the switch, writes the legacy bucket silently.
7. A run's input payload is written before the run row, so it cannot take the bucket from the run row; `created_at` is
   the application's clock, not the database's.
8. Quota counters disagree with the rows: double counting during the dual write, no release on a drop.
9. A crash or cancellation inside `DETACH ... CONCURRENTLY` leaves a pending detach that needs `FINALIZE`; the platform
   pool's 5 s statement timeout can cancel it.
10. Success criterion 049-002's 120 s bound does not hold (the idle timeout does not bound a transaction's life).
11. `persistent(false)` adds a round trip per journal statement that the budget does not count; foreign-key checks run
    as cached internal plans outside the driver.
12. The static check can be satisfied by a join equality that does not prune, and cannot see SQL in database
    functions.
13. The move can finish while processes of the previous release still write `payloads` only, and references to an
    existing digest are made without a write.
14. The contract's gate allows release-N processes that still use `payloads`; the cleanup's delete is refused by the
    always-enabled append-only trigger.
15. `audit_records` cannot use the attach-the-legacy method as written (the legacy boundary must be a future instant;
    the key `(id, created_at)` loses the uniqueness of `id`, which `chain_progress.rs` relies on).
16. "Mark purged" is undefined, and the cost of deleting the run-owned rows of the unpartitioned tables is not counted.
17. As planned, the tests cannot catch most of the above: no suite after the switch with two dated partitions, no
    controllable clock across a month boundary, no retirement on the post-contract schema, no fault inside the detach,
    small tables only.
18. Stubs with null digests reverse feature 020's compatibility decision and break older readers.

## Nits

19. 12 run-owned tables and 2 shared, 14 in all; `journal_bucket` is added to 10 tables besides `runs` and
    `run_events`.
20. Step 4 locks in the opposite order to the hot path when it creates foreign keys.
21. A retired bucket is not retryable; tell it from a missing future partition.
22. Requirement 049-004 confuses waiting for a lock with holding it.

Counts: Blocker 3, Major 15, Nit 4
