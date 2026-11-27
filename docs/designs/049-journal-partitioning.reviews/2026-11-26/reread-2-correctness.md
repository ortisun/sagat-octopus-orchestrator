# Targeted re-read 2 of design 049: correctness (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 12e3fe58421c0ddaf51a3da10a26b12635996655
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A second targeted re-read
of the fixes of re-read 1 (changes since 4060f927), not a full review. The reviewer read files only and ran nothing.

Re-read 1: A partly resolved (Majors 1, 2), B resolved, C partly resolved (Majors 3, 4), D resolved, E resolved at READ
COMMITTED (Nit 1), F resolved (Nit 2).

## Major

1. The claim that the hot path writes `run_payloads` before `run_events` is false (`declared_operations.rs` reads
   `run_events` first; a reset writes `run_events` before the flush writes payloads), so step 4 can deadlock with a
   reset. Fix: drop the claim, or avoid the cycle.
2. `LOCK TABLE` without `ONLY` locks every partition, the legacy one included, each wait with its own timeout while the
   parent is held; a vacuum can hold a partition. Fix: `LOCK TABLE ONLY` on the parents and lock only the retired
   partitions.
3. A run that fails the mark-only re-check after step 2 starts (retention lengthened, a review opened) leaves the
   retirement unable to finish or be abandoned, refusing per-run purges for good. Fix: freeze eligibility, or allow
   abandon with a sweep of what was purged.
4. Where "step 2 started" is recorded is not said; a crash after a mark-only chunk could allow an abandon that leaves
   rows nothing deletes. Fix: record it atomically, or refuse abandon when a run of the bucket is purged.

## Nit

1. The separate-statement check holds only at READ COMMITTED; name the isolation level.
2. A month retired during the following month is still the "previous month" that `ensure_journal_buckets()` creates;
   skip months with a `journal_retirements` row; "not marked retired" against "marks dropped".
3. Task 049-14 says four retirement functions, Requirement 049-016 lists five, and neither covers step 2.

Counts: Blocker 0, Major 4, Nit 3

All seven were fixed at `b2edd5bc`: one transaction per table that locks only its parent (`LOCK TABLE ONLY`) and the
bucket's partition, with the reason it cannot deadlock with the hot path (Majors 1, 2); an abandon allowed before the
drops that deletes by key, under feature 020's authorization row, the rows of the runs already purged (Majors 3, 4); the
isolation level named; the retired month skipped and the wording aligned; `journal_retire_purge_chunk` for step 2 and
six functions named.
